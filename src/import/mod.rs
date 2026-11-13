use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::{address::web_url, browser::ImportedPage};

/// Chrome counts microseconds from 1601-01-01; this is that epoch in Unix milliseconds.
const CHROME_EPOCH_MS: i64 = -11_644_473_600_000;

/// Every web page in the profile's history and bookmarks, one entry per URL.
fn read_pages(profile: &Path) -> Result<Vec<ImportedPage>> {
    let history = profile.join("History");
    let bookmarks = profile.join("Bookmarks");
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
    if bookmarks.is_file() {
        read_bookmarks(&bookmarks, &mut pages)
            .with_context(|| format!("Could not read {}", bookmarks.display()))?;
    }
    let mut pages: Vec<_> = pages.into_values().collect();
    pages.sort_by(|a, b| a.url.as_str().cmp(b.url.as_str()));
    Ok(pages)
}

/// Reads Chrome's `urls` table in place, read-only. Chrome can stay open; while it writes,
/// the read waits for its lock.
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
                folder: None,
            },
        );
    }
    Ok(())
}

/// Walks Chrome's `Bookmarks` JSON. A bookmarked page keeps its history counts and takes
/// the bookmark's name.
fn read_bookmarks(path: &Path, pages: &mut HashMap<String, ImportedPage>) -> Result<()> {
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
                    let page = pages
                        .entry(url.to_string())
                        .or_insert_with(|| ImportedPage {
                            url,
                            title: String::new(),
                            visits: 0,
                            typed: 0,
                            last_visit: None,
                            folder: None,
                        });
                    if !title.is_empty() {
                        page.title = title;
                    }
                    page.folder = Some(folder);
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
    Ok(())
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
                        {"type":"url","name":"Bad","url":"javascript:alert(1)"}]}]},
                "other":{"type":"folder","name":"Other bookmarks","children":[]},
                "synced":{"type":"folder","name":"Mobile bookmarks","children":[]}},
              "version":1}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn history_and_bookmarks_merge_by_url_and_skip_non_web_pages() {
        let dir = profile();
        let pages = chrome_profile(dir.path()).unwrap().pages;
        let urls: Vec<_> = pages.iter().map(|p| p.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://docs.rs/wry",
                "https://news.ycombinator.com/",
                "https://servo.org/"
            ]
        );
        let wry = &pages[0];
        assert_eq!(wry.title, "Rust docs: wry");
        assert_eq!((wry.visits, wry.typed), (12, 3));
        assert_eq!(wry.folder.as_deref(), Some("Bookmarks bar"));
        assert_eq!(wry.last_visit, Some(1_794_517_910_813));
        assert_eq!(pages[2].folder.as_deref(), Some("Bookmarks bar › Reading"));
        assert_eq!(pages[1].folder, None);
    }

    #[test]
    fn an_empty_directory_is_not_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        assert!(chrome_profile(dir.path()).is_err());
    }
}

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
    pub name: String,
    pub source: String,
    pub pages: Vec<ImportedPage>,
    pub cookies: Section<Cookie>,
    pub logins: Section<Login>,
}

pub fn chrome_profile(profile: &Path) -> Result<ChromeProfile> {
    let pages = read_pages(profile)?;
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
        source: format!("chrome:{name}"),
        name,
        pages,
        cookies: read_cookies(&profile.join("Cookies"), &keys, now),
        logins: read_logins(&profile.join("Login Data"), &keys),
    })
}

fn open_chrome_db(
    path: &Path,
) -> std::result::Result<(tempfile::TempDir, Connection), ImportError> {
    if !path.is_file() {
        return Err(ImportError::Storage);
    }
    let busy = || {
        ["-journal", "-wal"].iter().any(|suffix| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            std::fs::metadata(name).is_ok_and(|m| m.len() > 0)
        })
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
            last_used: rank.0,
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
    fn snapshot_refuses_nonempty_journal_and_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("History");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE test(id)")
            .unwrap();
        for suffix in ["-journal", "-wal"] {
            let sidecar = dir.path().join(format!("History{suffix}"));
            std::fs::write(&sidecar, b"busy").unwrap();
            assert!(matches!(
                open_chrome_db(&path),
                Err(ImportError::SourceBusy)
            ));
            std::fs::remove_file(sidecar).unwrap();
        }
        assert!(open_chrome_db(&path).is_ok());
    }
}
