use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::{
    address::web_url,
    browser::{ImportedBookmark, ImportedPage},
};

mod linux;
use crate::{
    address::WebOrigin,
    cookies::{Cookie, CookieDomain, SameSite},
    logins::Login,
    secret::Secret,
};
use linux::ChromeKeys;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Chrome counts microseconds from 1601-01-01; this is that epoch in Unix milliseconds.
const CHROME_EPOCH_MS: i64 = -11_644_473_600_000;

/// Chrome's microseconds since 1601 as Unix milliseconds. Zero, never used, stays zero.
fn chrome_ms(value: i64) -> i64 {
    if value > 0 {
        value / 1000 + CHROME_EPOCH_MS
    } else {
        0
    }
}

/// Every web page in the profile's history, one entry per URL.
fn read_pages(profile: &Path) -> Result<Vec<ImportedPage>> {
    let history = profile.join("History");
    if !["History", "Bookmarks", "Cookies", "Login Data"]
        .iter()
        .any(|name| profile.join(name).is_file())
    {
        bail!(
            "{} has no Chrome History or Bookmarks file.",
            profile.display()
        );
    }
    let mut pages = HashMap::new();
    if history.is_file() {
        read_history(&history, &mut pages)
            .with_context(|| format!("Could not read {}", history.display()))?;
    }
    let mut pages: Vec<_> = pages.into_values().collect();
    pages.sort_by(|a, b| a.url.as_str().cmp(b.url.as_str()));
    Ok(pages)
}

fn read_history(path: &Path, pages: &mut HashMap<String, ImportedPage>) -> Result<()> {
    let (_snapshot, connection) = open_chrome_db(path)?;
    let mut statement = connection.prepare(
        "SELECT url, title, visit_count, typed_count, last_visit_time FROM urls WHERE hidden = 0",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let url: String = row.get(0)?;
        let Ok(url) = web_url(&url) else { continue };
        let last: i64 = row.get(4)?;
        pages.insert(
            url.to_string(),
            ImportedPage {
                url,
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                visits: row.get::<_, i64>(2)?.max(0) as usize,
                typed: row.get::<_, i64>(3)?.max(0) as usize,
                last_visit: (last > 0).then(|| last / 1000 + CHROME_EPOCH_MS),
            },
        );
    }
    Ok(())
}

/// Reads Chrome's bookmarks separately, retaining history counts for the initial order.
fn read_bookmarks(path: &Path, pages: &[ImportedPage]) -> Result<Vec<ImportedBookmark>> {
    let mut bookmarks = Vec::new();
    let visits: HashMap<_, _> = pages.iter().map(|p| (&p.url, p.visits)).collect();
    let mut seen = HashSet::new();
    let json: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let roots = json["roots"]
        .as_object()
        .context("The Bookmarks file has no roots.")?;
    for (key, root) in roots {
        let name = match key.as_str() {
            "bookmark_bar" => "Bookmarks bar",
            "other" => "Other bookmarks",
            "synced" => "Mobile bookmarks",
            _ => continue,
        };
        let mut stack = vec![(root, name.to_owned())];
        while let Some((node, folder)) = stack.pop() {
            match node["type"].as_str() {
                Some("url") => {
                    let Some(Ok(url)) = node["url"].as_str().map(web_url) else {
                        continue;
                    };
                    let title = node["name"].as_str().unwrap_or_default().to_owned();
                    let visits = visits.get(&url).copied().unwrap_or(0);
                    if seen.insert(url.clone()) {
                        bookmarks.push(ImportedBookmark {
                            url,
                            title,
                            visits,
                            folder,
                        });
                    }
                }
                Some("folder") => {
                    let path = if std::ptr::eq(node, root) {
                        folder
                    } else {
                        format!("{folder} › {}", node["name"].as_str().unwrap_or_default())
                    };
                    for child in node["children"].as_array().into_iter().flatten().rev() {
                        stack.push((child, path.clone()));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(bookmarks)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportError {
    SourceBusy,
    Storage,
    KeyringUnavailable,
    Undecryptable,
}
impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SourceBusy => "Chrome is writing this database. Run the import again.",
            Self::Storage => "Could not read the Chrome database.",
            Self::KeyringUnavailable => {
                "Chrome Safe Storage is unavailable. Unlock the system keyring and retry."
            }
            Self::Undecryptable => "Could not decrypt this Chrome value.",
        })
    }
}
impl std::error::Error for ImportError {}
impl From<rusqlite::Error> for ImportError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SkipReason {
    Expired,
    Partitioned,
    InsecureSameSite,
    InvalidCookie,
    Undecryptable,
    Blacklisted,
    NonHtml,
    InvalidOrigin,
    RealmMismatch,
    EmptyPassword,
    Duplicate,
}
impl SkipReason {
    /// How the import report names rows skipped for this reason.
    pub fn label(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Partitioned => "partitioned third-party",
            Self::InsecureSameSite => "insecure cross-site",
            Self::InvalidCookie => "invalid",
            Self::Undecryptable => "unreadable",
            Self::Blacklisted => "never-save",
            Self::NonHtml => "not from a web form",
            Self::InvalidOrigin => "not a web address",
            Self::RealmMismatch => "for another realm",
            Self::EmptyPassword => "empty",
            Self::Duplicate => "duplicate",
        }
    }
}

pub type Section<T> = std::result::Result<Decrypted<T>, ImportError>;
pub struct Decrypted<T> {
    pub items: Vec<T>,
    pub skipped: BTreeMap<SkipReason, usize>,
}
impl<T> Default for Decrypted<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            skipped: BTreeMap::new(),
        }
    }
}
impl<T> Decrypted<T> {
    fn skip(&mut self, why: SkipReason) {
        *self.skipped.entry(why).or_default() += 1;
    }
}
pub struct ChromeProfile {
    pub bookmarks: Vec<ImportedBookmark>,
    pub name: String,
    pub source: String,
    pub pages: Vec<ImportedPage>,
    pub cookies: Section<Cookie>,
    pub logins: Section<Login>,
}

pub fn chrome_profile(profile: &Path) -> Result<ChromeProfile> {
    let pages = read_pages(profile)?;
    let bookmarks = if profile.join("Bookmarks").is_file() {
        read_bookmarks(&profile.join("Bookmarks"), &pages)?
    } else {
        vec![]
    };
    let name = profile
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let keys = ChromeKeys::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    Ok(ChromeProfile {
        bookmarks,
        source: format!("chrome:{name}"),
        name,
        pages,
        cookies: read_cookies(&profile.join("Cookies"), &keys, now),
        logins: read_logins(&profile.join("Login Data"), &keys),
    })
}

/// The first eight bytes of a SQLite rollback journal that holds an unfinished write.
const JOURNAL_MAGIC: [u8; 8] = [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];

fn open_chrome_db(
    path: &Path,
) -> std::result::Result<(tempfile::TempDir, Connection), ImportError> {
    if !path.is_file() {
        return Err(ImportError::Storage);
    }
    let sidecar = |suffix: &str| {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        std::path::PathBuf::from(name)
    };
    // SQLite's own hot-journal rule: a rollback journal holds an unfinished write only while
    // its header starts with the journal magic. Chrome keeps committed journals around with
    // a zeroed header (persistent journal mode), and those are safe to copy past.
    let busy = || {
        let hot_journal = std::fs::read(sidecar("-journal"))
            .is_ok_and(|journal| journal.starts_with(&JOURNAL_MAGIC));
        let wal = std::fs::metadata(sidecar("-wal")).is_ok_and(|m| m.len() > 0);
        hot_journal || wal
    };
    for attempt in 0..10 {
        let snapshot = tempfile::tempdir().map_err(|_| ImportError::Storage)?;
        let copy = snapshot.path().join("snapshot.sqlite3");
        if !busy() && std::fs::copy(path, &copy).is_ok() && !busy() {
            if let Ok(connection) =
                Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY)
            {
                if connection
                    .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                    .is_ok_and(|s| s == "ok")
                {
                    return Ok((snapshot, connection));
                }
            }
        }
        if attempt < 9 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    Err(ImportError::SourceBusy)
}

fn read_cookies(path: &Path, keys: &ChromeKeys, now: i64) -> Section<Cookie> {
    let (_snapshot, db) = open_chrome_db(path)?;
    let version: i64 = db
        .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
            r.get::<_, String>(0)
        })?
        .parse()
        .map_err(|_| ImportError::Storage)?;
    let partitioned = db
        .prepare("PRAGMA table_info(cookies)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|s| s == "top_frame_site_key");
    let sql = format!("SELECT host_key,name,path,expires_utc,is_secure,is_httponly,samesite,encrypted_value,value,{} FROM cookies",
        if partitioned { "top_frame_site_key" } else { "''" });
    let mut stmt = db.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Decrypted::default();
    while let Some(row) = rows.next()? {
        if !row.get::<_, String>(9)?.is_empty() {
            out.skip(SkipReason::Partitioned);
            continue;
        }
        let expiry: i64 = row.get(3)?;
        let expires = (expiry != 0).then_some(expiry / 1_000_000 - 11_644_473_600);
        if expires.is_some_and(|e| e <= now) {
            out.skip(SkipReason::Expired);
            continue;
        }
        let host: String = row.get(0)?;
        let encrypted: Vec<u8> = row.get(7)?;
        let value = if encrypted.is_empty() {
            Secret::new(row.get(8)?)
        } else {
            let bytes = match keys.decrypt(&encrypted) {
                Ok(bytes) => bytes,
                Err(ImportError::KeyringUnavailable) => {
                    return Err(ImportError::KeyringUnavailable)
                }
                Err(_) => {
                    out.skip(SkipReason::Undecryptable);
                    continue;
                }
            };
            let bytes = if version >= 24 {
                if bytes.get(..32) != Some(Sha256::digest(host.as_bytes()).as_slice()) {
                    out.skip(SkipReason::Undecryptable);
                    continue;
                }
                Zeroizing::new(bytes[32..].to_vec())
            } else {
                bytes
            };
            let Some(value) = Secret::from_utf8(bytes) else {
                out.skip(SkipReason::Undecryptable);
                continue;
            };
            value
        };
        let Some(domain) = CookieDomain::parse(&host) else {
            out.skip(SkipReason::InvalidCookie);
            continue;
        };
        let same_site = match row.get::<_, i64>(6)? {
            -1 | 1 => SameSite::Lax,
            2 => SameSite::Strict,
            0 => SameSite::None,
            _ => {
                out.skip(SkipReason::InvalidCookie);
                continue;
            }
        };
        let cookie = Cookie {
            domain,
            name: row.get(1)?,
            path: row.get(2)?,
            value,
            expires,
            secure: row.get(4)?,
            http_only: row.get(5)?,
            same_site,
        };
        if same_site == SameSite::None && !cookie.secure {
            out.skip(SkipReason::InsecureSameSite);
        } else if !cookie.valid() {
            out.skip(SkipReason::InvalidCookie);
        } else {
            out.items.push(cookie);
        }
    }
    Ok(out)
}

fn read_logins(path: &Path, keys: &ChromeKeys) -> Section<Login> {
    let (_snapshot, db) = open_chrome_db(path)?;
    let mut stmt = db.prepare("SELECT origin_url,signon_realm,username_value,password_value,date_last_used,date_created,blacklisted_by_user,scheme FROM logins")?;
    let mut rows = stmt.query([])?;
    let mut out = Decrypted::default();
    let mut accounts = BTreeMap::new();
    while let Some(row) = rows.next()? {
        if row.get::<_, bool>(6)? {
            out.skip(SkipReason::Blacklisted);
            continue;
        }
        if row.get::<_, i64>(7)? != 0 {
            out.skip(SkipReason::NonHtml);
            continue;
        }
        let origin = url::Url::parse(&row.get::<_, String>(0)?)
            .ok()
            .and_then(|url| WebOrigin::of(&url));
        let Some(origin) = origin else {
            out.skip(SkipReason::InvalidOrigin);
            continue;
        };
        if row.get::<_, String>(1)? != format!("{}/", origin.as_str()) {
            out.skip(SkipReason::RealmMismatch);
            continue;
        }
        let encrypted: Vec<u8> = row.get(3)?;
        if encrypted.is_empty() {
            out.skip(SkipReason::EmptyPassword);
            continue;
        }
        let password = match keys
            .decrypt(&encrypted)
            .and_then(|bytes| Secret::from_utf8(bytes).ok_or(ImportError::Undecryptable))
        {
            Ok(password) => password,
            Err(ImportError::KeyringUnavailable) => return Err(ImportError::KeyringUnavailable),
            Err(_) => {
                out.skip(SkipReason::Undecryptable);
                continue;
            }
        };
        if password.expose().is_empty() {
            out.skip(SkipReason::EmptyPassword);
            continue;
        }
        let username: String = row.get(2)?;
        let rank = (row.get::<_, i64>(4)?, row.get::<_, i64>(5)?);
        let login = Login {
            origin: origin.clone(),
            username: username.clone(),
            password,
            last_used: chrome_ms(rank.0),
        };
        let key = (origin, username);
        if let Some((old, _)) = accounts.get(&key) {
            out.skip(SkipReason::Duplicate);
            if old > &rank {
                continue;
            }
        }
        accounts.insert(key, (rank, login));
    }
    out.items = accounts.into_values().map(|(_, login)| login).collect();
    Ok(out)
}

#[cfg(test)]
mod encrypted_tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn synthetic_chrome_filters_and_decrypts() {
        let dir = tempfile::tempdir().unwrap();
        let cookie_path = dir.path().join("Cookies");
        let db = Connection::open(&cookie_path).unwrap();
        db.execute_batch("CREATE TABLE meta(key TEXT,value TEXT); INSERT INTO meta VALUES('version','24');
            CREATE TABLE cookies(host_key TEXT,name TEXT,path TEXT,expires_utc INTEGER,is_secure INTEGER,is_httponly INTEGER,samesite INTEGER,encrypted_value BLOB,value TEXT,top_frame_site_key TEXT);").unwrap();
        let keys = ChromeKeys::with_v11_password(b"fake-key");
        for (name, host, expiry, secure, same_site, partition, valid_hash, tag) in [
            ("session", ".example.test", 0, true, -1, "", true, "v10"),
            (
                "persistent",
                "example.test",
                13_000_000_000_000_000_i64,
                true,
                2,
                "",
                true,
                "v11",
            ),
            ("none", "example.test", 0, true, 0, "", true, "v10"),
            ("expired", "example.test", 1, true, 1, "", true, "v10"),
            (
                "partitioned",
                "example.test",
                0,
                true,
                1,
                "https://other.test",
                true,
                "v10",
            ),
            ("insecure", "example.test", 0, false, 0, "", true, "v10"),
            ("__Host-bad", ".example.test", 0, true, 1, "", true, "v10"),
            ("hash", "example.test", 0, true, 1, "", false, "v10"),
        ] {
            let mut plain = if valid_hash {
                Sha256::digest(host.as_bytes()).to_vec()
            } else {
                vec![0; 32]
            };
            plain.extend(b"fake-cookie");
            let encrypted = linux::encrypt(
                if tag == "v10" {
                    b"peanuts"
                } else {
                    b"fake-key"
                },
                tag.as_bytes(),
                &plain,
            );
            db.execute(
                "INSERT INTO cookies VALUES(?1,?2,'/',?3,?4,1,?5,?6,'never-fallback',?7)",
                params![host, name, expiry, secure, same_site, encrypted, partition],
            )
            .unwrap();
        }
        drop(db);
        let out = read_cookies(&cookie_path, &keys, 1).unwrap();
        assert_eq!(out.items.len(), 3);
        assert!(out.items[0].value.expose() == "fake-cookie");
        assert_eq!(out.items[0].domain.dotted(), ".example.test");
        assert_eq!(out.items[0].expires, None);
        assert_eq!(out.items[0].same_site, SameSite::Lax);
        assert!(out.items[1].expires.is_some());
        assert_eq!(out.items[1].same_site, SameSite::Strict);
        assert_eq!(out.items[2].same_site, SameSite::None);
        for reason in [
            SkipReason::Expired,
            SkipReason::Partitioned,
            SkipReason::InsecureSameSite,
            SkipReason::InvalidCookie,
            SkipReason::Undecryptable,
        ] {
            assert_eq!(out.skipped.get(&reason), Some(&1));
        }

        let path = dir.path().join("Login Data");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE logins(origin_url TEXT,signon_realm TEXT,username_value TEXT,password_value BLOB,date_last_used INTEGER,date_created INTEGER,blacklisted_by_user INTEGER,scheme INTEGER)").unwrap();
        for (origin, realm, name, password, used, blacklisted, scheme, tag) in [
            (
                "https://example.test/a",
                "https://example.test/",
                "alice",
                "old-fake",
                1,
                false,
                0,
                "v10",
            ),
            (
                "https://example.test/b",
                "https://example.test/",
                "alice",
                "new-fake",
                2,
                false,
                0,
                "v11",
            ),
            (
                "https://example.test/",
                "https://example.test/",
                "bob",
                "bob-fake",
                0,
                false,
                0,
                "v10",
            ),
            (
                "https://example.test/",
                "https://example.test/",
                "blocked",
                "fake",
                0,
                true,
                0,
                "v10",
            ),
            (
                "https://example.test/",
                "https://example.test/",
                "auth",
                "fake",
                0,
                false,
                1,
                "v10",
            ),
            (
                "android://example",
                "android://example",
                "android",
                "fake",
                0,
                false,
                0,
                "v10",
            ),
            (
                "https://example.test/",
                "https://other.test/",
                "realm",
                "fake",
                0,
                false,
                0,
                "v10",
            ),
            (
                "https://example.test/",
                "https://example.test/",
                "empty",
                "",
                0,
                false,
                0,
                "v10",
            ),
        ] {
            let encrypted = linux::encrypt(
                if tag == "v10" {
                    b"peanuts"
                } else {
                    b"fake-key"
                },
                tag.as_bytes(),
                password.as_bytes(),
            );
            db.execute(
                "INSERT INTO logins VALUES(?1,?2,?3,?4,?5,0,?6,?7)",
                params![origin, realm, name, encrypted, used, blacklisted, scheme],
            )
            .unwrap();
        }
        drop(db);
        let out = read_logins(&path, &keys).unwrap();
        assert_eq!(out.items.len(), 2);
        assert!(out.items[0].password.expose() == "new-fake");
        assert!(!format!("{:?}", out.items[0]).contains("new-fake"));
        for reason in [
            SkipReason::Duplicate,
            SkipReason::Blacklisted,
            SkipReason::NonHtml,
            SkipReason::InvalidOrigin,
            SkipReason::RealmMismatch,
            SkipReason::EmptyPassword,
        ] {
            assert_eq!(out.skipped.get(&reason), Some(&1));
        }
    }

    #[test]
    fn snapshot_waits_for_a_hot_journal_or_wal_but_not_a_committed_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("History");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE test(id)")
            .unwrap();
        let journal = dir.path().join("History-journal");
        let mut hot = JOURNAL_MAGIC.to_vec();
        hot.extend([0; 504]);
        std::fs::write(&journal, &hot).unwrap();
        assert!(matches!(
            open_chrome_db(&path),
            Err(ImportError::SourceBusy)
        ));
        std::fs::write(&journal, [0; 512]).unwrap();
        assert!(
            open_chrome_db(&path).is_ok(),
            "Chrome's persistent journal with a zeroed header is committed"
        );
        std::fs::remove_file(&journal).unwrap();
        let wal = dir.path().join("History-wal");
        std::fs::write(&wal, b"frames").unwrap();
        assert!(matches!(
            open_chrome_db(&path),
            Err(ImportError::SourceBusy)
        ));
        std::fs::remove_file(wal).unwrap();
        assert!(open_chrome_db(&path).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile with Chrome's `urls` schema and a Bookmarks file.
    fn profile() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let history = Connection::open(dir.path().join("History")).unwrap();
        history
            .execute_batch(
                "CREATE TABLE urls(id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR,
                    visit_count INTEGER DEFAULT 0 NOT NULL, typed_count INTEGER DEFAULT 0 NOT NULL,
                    last_visit_time INTEGER NOT NULL, hidden INTEGER DEFAULT 0 NOT NULL);
                 INSERT INTO urls(url, title, visit_count, typed_count, last_visit_time, hidden) VALUES
                    ('https://docs.rs/wry', 'wry - Rust', 12, 3, 13438991510813493, 0),
                    ('https://news.ycombinator.com/', 'Hacker News', 40, 9, 13438991510813493, 0),
                    ('chrome://settings/', 'Settings', 5, 5, 13438991510813493, 0),
                    ('https://hidden.example/', 'Hidden', 1, 0, 13438991510813493, 1);",
            )
            .unwrap();
        std::fs::write(
            dir.path().join("Bookmarks"),
            r#"{"roots":{
                "bookmark_bar":{"type":"folder","name":"Bookmarks bar","children":[
                    {"type":"url","name":"Rust docs: wry","url":"https://docs.rs/wry"},
                    {"type":"folder","name":"Reading","children":[
                        {"type":"url","name":"Servo","url":"https://servo.org/"},
                        {"type":"url","name":"Bad","url":"javascript:alert(1)"}]},
                    {"type":"folder","name":"Work","children":[{"type":"folder","name":"Rust","children":[{"type":"url","name":"Nested","url":"https://example.org/nested"}]}]}]},
                "other":{"type":"folder","name":"Other bookmarks","children":[{"type":"folder","name":"Y","children":[{"type":"url","name":"Other","url":"https://example.org/other"}]}]},
                "synced":{"type":"folder","name":"Mobile bookmarks","children":[{"type":"url","name":"Mobile","url":"https://example.org/mobile"}]}},
              "version":1}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn bookmarks_are_read_apart_from_history() {
        let dir = profile();
        let profile = chrome_profile(dir.path()).unwrap();
        let pages = profile.pages;
        let bookmarks = profile.bookmarks;
        let urls: Vec<_> = pages.iter().map(|p| p.url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://docs.rs/wry", "https://news.ycombinator.com/"]
        );
        let wry = &pages[0];
        assert_eq!(bookmarks[0].title, "Rust docs: wry");
        assert_eq!((wry.visits, wry.typed), (12, 3));
        assert_eq!(bookmarks[0].folder, "Bookmarks bar");
        assert_eq!(wry.last_visit, Some(1_794_517_910_813));
        assert_eq!(bookmarks[1].folder, "Bookmarks bar › Reading");
        assert_eq!(bookmarks.len(), 5);
        assert_eq!(bookmarks[2].folder, "Bookmarks bar › Work › Rust");
        assert_eq!(bookmarks[3].folder, "Other bookmarks › Y");
        assert_eq!(bookmarks[4].folder, "Mobile bookmarks");
        assert_eq!(bookmarks[0].visits, 12);
    }

    #[test]
    fn an_empty_directory_is_not_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        assert!(chrome_profile(dir.path()).is_err());
    }
}
