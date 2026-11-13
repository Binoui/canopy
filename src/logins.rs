use crate::{address::WebOrigin, secret::Secret};

#[derive(Debug)]
pub struct Login {
    pub origin: WebOrigin,
    pub username: String,
    pub password: Secret,
    pub last_used: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginId(i64);

#[derive(Clone, Debug)]
pub struct SavedLogin {
    pub id: LoginId,
    pub origin: WebOrigin,
    pub username: String,
}

use aes_gcm::{
    aead::{rand_core::RngCore, Aead, AeadCore, OsRng, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};
use url::Url;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginsError {
    KeyringUnavailable,
    KeyLost,
    Corrupt,
    Storage,
}
impl std::fmt::Display for LoginsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::KeyringUnavailable => "The system keyring is unavailable.",
            Self::KeyLost => "The vault key is missing or does not match.",
            Self::Corrupt => "The saved login could not be opened.",
            Self::Storage => "The login vault could not be accessed.",
        })
    }
}
impl std::error::Error for LoginsError {}
impl From<rusqlite::Error> for LoginsError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
type Result<T> = std::result::Result<T, LoginsError>;

pub struct Logins {
    db: Connection,
    namespace: String,
    key: RefCell<Option<Zeroizing<[u8; 32]>>>,
}

impl Logins {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).map_err(|_| LoginsError::Storage)?;
        let canonical = data_dir.canonicalize().map_err(|_| LoginsError::Storage)?;
        let namespace = Sha256::digest(canonical.as_os_str().as_encoded_bytes())[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let path = data_dir.join("logins.sqlite3");
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|_| LoginsError::Storage)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| LoginsError::Storage)?;
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS vault(singleton INTEGER PRIMARY KEY CHECK(singleton=1),key_id BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS logins(id INTEGER PRIMARY KEY,source TEXT NOT NULL,origin TEXT NOT NULL,username TEXT NOT NULL,sealed BLOB NOT NULL,last_used INTEGER NOT NULL,UNIQUE(source,origin,username));
            CREATE INDEX IF NOT EXISTS logins_origin ON logins(origin);
            PRAGMA user_version=1;")?;
        Ok(Self {
            db,
            namespace,
            key: RefCell::new(None),
        })
    }

    fn vault_key(&self) -> Result<Zeroizing<[u8; 32]>> {
        let expected: Option<Vec<u8>> = self
            .db
            .query_row("SELECT key_id FROM vault WHERE singleton=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let key = if let Some(key) = self.key.borrow().as_ref() {
            Zeroizing::new(**key)
        } else {
            let attributes = [
                ("application", "canopy"),
                ("canopy-vault", self.namespace.as_str()),
            ];
            let stored =
                crate::key_store::read(&attributes).map_err(|_| LoginsError::KeyringUnavailable)?;
            match stored {
                Some(bytes) => Zeroizing::new(
                    bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| LoginsError::KeyLost)?,
                ),
                None if expected.is_some() => return Err(LoginsError::KeyLost),
                None => {
                    let mut key = Zeroizing::new([0; 32]);
                    OsRng.fill_bytes(&mut *key);
                    crate::key_store::write(&attributes, &*key)
                        .map_err(|_| LoginsError::KeyringUnavailable)?;
                    key
                }
            }
        };
        let mut hash = Sha256::new();
        hash.update(b"canopy-logins");
        hash.update(*key);
        let id = hash.finalize();
        if let Some(expected) = expected {
            if expected != id[..16] {
                return Err(LoginsError::KeyLost);
            }
        } else {
            self.db.execute(
                "INSERT INTO vault(singleton,key_id) VALUES(1,?1)",
                [&id[..16]],
            )?;
        }
        *self.key.borrow_mut() = Some(Zeroizing::new(*key));
        Ok(key)
    }

    pub fn upsert_imported(&mut self, source: &str, logins: &[Login]) -> Result<usize> {
        if logins.is_empty() {
            return Ok(0);
        }
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let key = self.vault_key()?;
            let cipher = Aes256Gcm::new_from_slice(&*key).map_err(|_| LoginsError::Corrupt)?;
            for login in logins {
                let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
                let aad = associated_data(&login.origin, &login.username);
                let ciphertext = cipher
                    .encrypt(
                        &nonce,
                        Payload {
                            msg: login.password.expose().as_bytes(),
                            aad: &aad,
                        },
                    )
                    .map_err(|_| LoginsError::Corrupt)?;
                let mut sealed = nonce.to_vec();
                sealed.extend(ciphertext);
                self.db.execute("INSERT INTO logins(source,origin,username,sealed,last_used) VALUES(?1,?2,?3,?4,?5)
                    ON CONFLICT(source,origin,username) DO UPDATE SET sealed=excluded.sealed,last_used=excluded.last_used",
                    params![source, login.origin.as_str(), login.username, sealed, login.last_used])?;
            }
            Ok(logins.len())
        })();
        match result {
            Ok(count) => {
                self.db.execute_batch("COMMIT")?;
                Ok(count)
            }
            Err(error) => {
                let _ = self.db.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn for_page(&self, page: &Url) -> Result<Vec<SavedLogin>> {
        let Some(origin) = WebOrigin::of(page) else {
            return Ok(Vec::new());
        };
        let mut stmt = self
            .db
            .prepare("SELECT id,username FROM logins WHERE origin=?1 ORDER BY last_used DESC,id")?;
        let rows = stmt.query_map([origin.as_str()], |r| {
            Ok(SavedLogin {
                id: LoginId(r.get(0)?),
                origin: origin.clone(),
                username: r.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn password(&self, login: &SavedLogin) -> Result<Secret> {
        let sealed: Vec<u8> = self
            .db
            .query_row(
                "SELECT sealed FROM logins WHERE id=?1 AND origin=?2 AND username=?3",
                params![login.id.0, login.origin.as_str(), login.username],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(LoginsError::Corrupt)?;
        if sealed.len() < 28 {
            return Err(LoginsError::Corrupt);
        }
        let key = self.vault_key()?;
        let cipher = Aes256Gcm::new_from_slice(&*key).map_err(|_| LoginsError::Corrupt)?;
        let aad = associated_data(&login.origin, &login.username);
        let bytes = cipher
            .decrypt(
                Nonce::from_slice(&sealed[..12]),
                Payload {
                    msg: &sealed[12..],
                    aad: &aad,
                },
            )
            .map_err(|_| LoginsError::Corrupt)?;
        Secret::from_utf8(Zeroizing::new(bytes)).ok_or(LoginsError::Corrupt)
    }

    #[cfg(test)]
    fn with_key(data_dir: &Path, key: [u8; 32]) -> Self {
        let vault = Self::open(data_dir).unwrap();
        *vault.key.borrow_mut() = Some(Zeroizing::new(key));
        vault
    }
}

fn associated_data(origin: &WebOrigin, username: &str) -> Vec<u8> {
    let mut aad = origin.as_str().as_bytes().to_vec();
    aad.push(0);
    aad.extend(username.as_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;
    fn login(site: &str, password: &str) -> Login {
        Login {
            origin: WebOrigin::of(&Url::parse(site).unwrap()).unwrap(),
            username: "alice".into(),
            password: Secret::new(password.into()),
            last_used: 7,
        }
    }
    #[test]
    fn upserts_lookup_and_binding() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Logins::with_key(dir.path(), [42; 32]);
        let page = Url::parse("https://example.test/path").unwrap();
        for _ in 0..2 {
            vault
                .upsert_imported("chrome:one", &[login(page.as_str(), "fake-one")])
                .unwrap();
        }
        let offered = vault.for_page(&page).unwrap();
        assert_eq!(offered.len(), 1);
        assert!(!format!("{:?}", offered[0]).contains("fake-one"));
        assert!(vault.password(&offered[0]).unwrap().expose() == "fake-one");
        vault
            .upsert_imported("chrome:other", &[login(page.as_str(), "other-fake")])
            .unwrap();
        vault
            .upsert_imported("chrome:one", &[login(page.as_str(), "changed-fake")])
            .unwrap();
        assert_eq!(vault.for_page(&page).unwrap().len(), 2);
        assert!(vault.password(&offered[0]).unwrap().expose() == "changed-fake");
        assert!(vault
            .for_page(&Url::parse("http://example.test").unwrap())
            .unwrap()
            .is_empty());
        assert!(vault
            .for_page(&Url::parse("https://sub.example.test").unwrap())
            .unwrap()
            .is_empty());
        vault
            .upsert_imported("chrome:one", &[login("https://other.test", "site-fake")])
            .unwrap();
        let other = vault
            .for_page(&Url::parse("https://other.test").unwrap())
            .unwrap()
            .remove(0);
        vault
            .db
            .execute(
                "UPDATE logins SET sealed=(SELECT sealed FROM logins WHERE id=?1) WHERE id=?2",
                params![offered[0].id.0, other.id.0],
            )
            .unwrap();
        assert!(matches!(vault.password(&other), Err(LoginsError::Corrupt)));
        vault
            .db
            .execute(
                "UPDATE logins SET origin='https://third.test' WHERE id=?1",
                [offered[0].id.0],
            )
            .unwrap();
        assert!(matches!(
            vault.password(&offered[0]),
            Err(LoginsError::Corrupt)
        ));
        assert!(!format!("{:?}", login(page.as_str(), "fake-one")).contains("fake-one"));
        let rows = vault.for_page(&page).unwrap();
        assert!(vault.password(&rows[0]).unwrap().expose() == "other-fake");
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(!bytes
                .windows(b"other-fake".len())
                .any(|w| w == b"other-fake"));
            assert!(!bytes
                .windows(b"changed-fake".len())
                .any(|w| w == b"changed-fake"));
        }
        let locked = Logins::open(dir.path()).unwrap();
        assert_eq!(locked.for_page(&page).unwrap().len(), 1);
    }

    #[test]
    fn mismatched_key_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Logins::with_key(dir.path(), [1; 32]);
        vault
            .upsert_imported("chrome", &[login("https://example.test", "fake")])
            .unwrap();
        drop(vault);
        let mut wrong = Logins::with_key(dir.path(), [2; 32]);
        assert_eq!(
            wrong.upsert_imported("chrome", &[login("https://example.test", "fake")]),
            Err(LoginsError::KeyLost)
        );
        let correct = Logins::with_key(dir.path(), [1; 32]);
        let saved = correct
            .for_page(&Url::parse("https://example.test").unwrap())
            .unwrap();
        assert!(correct.password(&saved[0]).unwrap().expose() == "fake");
    }
}
