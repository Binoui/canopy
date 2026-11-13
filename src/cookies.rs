use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use crate::secret::Secret;

#[derive(Debug)]
pub struct Cookie {
    pub domain: CookieDomain,
    pub path: String,
    pub name: String,
    pub value: Secret,
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CookieDomain {
    Host(String),
    Domain(String),
}

impl CookieDomain {
    pub fn parse(dotted: &str) -> Option<Self> {
        let (domain, host) = match dotted.strip_prefix('.') {
            Some(host) => (true, host),
            None => (false, dotted),
        };
        if host.is_empty()
            || host.chars().any(|c| c.is_control() || c.is_whitespace())
            || url::Host::parse(host).is_err()
        {
            return None;
        }
        Some(if domain {
            Self::Domain(host.into())
        } else {
            Self::Host(host.into())
        })
    }

    pub fn dotted(&self) -> String {
        match self {
            Self::Host(host) => host.clone(),
            Self::Domain(host) => format!(".{host}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SameSite {
    None,
    Lax,
    Strict,
}

impl Cookie {
    pub(crate) fn valid(&self) -> bool {
        !self.name.is_empty()
            && ![
                self.name.as_str(),
                self.path.as_str(),
                self.value.expose(),
                &self.domain.dotted(),
            ]
            .iter()
            .any(|s| s.chars().any(char::is_control))
            && (self.same_site != SameSite::None || self.secure)
            && (!self.name.starts_with("__Secure-") || self.secure)
            && (!self.name.starts_with("__Host-")
                || (self.secure
                    && matches!(self.domain, CookieDomain::Host(_))
                    && self.path == "/"))
    }
}

pub struct Inbox {
    dir: PathBuf,
}
pub struct Batch {
    pub cookies: Vec<Cookie>,
    path: PathBuf,
}

impl Inbox {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("imports"),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    pub fn prepare(&self) -> Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    pub fn post(&self, cookies: &[Cookie]) -> Result<usize> {
        self.prepare()?;
        if cookies.is_empty() {
            return Ok(0);
        }
        static IDS: std::sync::Mutex<ulid::Generator> =
            std::sync::Mutex::new(ulid::Generator::new());
        let id = IDS
            .lock()
            .map_err(|_| anyhow!("Could not post imported cookies."))?
            .generate()
            .map_err(|_| anyhow!("Could not post imported cookies."))?;
        let temporary = self.dir.join(format!(".cookies-{id}.tmp"));
        let published = self.dir.join(format!("cookies-{id}.jsonl"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let result = (|| {
            for cookie in cookies {
                if !cookie.valid() {
                    return Err(anyhow!("Invalid imported cookie."));
                }
                let line = json!({
                    "domain": cookie.domain.dotted(), "path": cookie.path, "name": cookie.name,
                    "value": cookie.value.expose(), "expires": cookie.expires,
                    "secure": cookie.secure, "http_only": cookie.http_only,
                    "same_site": match cookie.same_site { SameSite::None => "None", SameSite::Lax => "Lax", SameSite::Strict => "Strict" },
                });
                writeln!(file, "{line}")?;
            }
            file.sync_all()?;
            fs::rename(&temporary, &published)?;
            fs::File::open(&self.dir)?.sync_all()?;
            Ok(cookies.len())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn collect(&self) -> Result<Vec<Batch>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(anyhow!("Could not read imported cookies.")),
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("cookies-")
                && name.ends_with(".jsonl")
                && entry.file_type()?.is_file()
            {
                paths.push(entry.path());
            }
        }
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let mut cookies = Vec::new();
                for line in BufReader::new(fs::File::open(&path)?).lines() {
                    let line = zeroize::Zeroizing::new(line?);
                    let cookie = from_line(&line)
                        .ok_or_else(|| anyhow!("Invalid imported cookie batch."))?;
                    cookies.push(cookie);
                }
                Ok(Batch { cookies, path })
            })
            .collect()
    }
}

impl Batch {
    pub fn accept(self) -> Result<()> {
        match fs::remove_file(self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(anyhow!("Could not accept imported cookies.")),
        }
    }
}

fn from_line(line: &str) -> Option<Cookie> {
    let value: Value = serde_json::from_str(line).ok()?;
    let cookie = Cookie {
        domain: CookieDomain::parse(value["domain"].as_str()?)?,
        path: value["path"].as_str()?.into(),
        name: value["name"].as_str()?.into(),
        value: Secret::new(value["value"].as_str()?.into()),
        expires: if value["expires"].is_null() {
            None
        } else {
            Some(value["expires"].as_i64()?)
        },
        secure: value["secure"].as_bool()?,
        http_only: value["http_only"].as_bool()?,
        same_site: match value["same_site"].as_str()? {
            "None" => SameSite::None,
            "Lax" => SameSite::Lax,
            "Strict" => SameSite::Strict,
            _ => return None,
        },
    };
    cookie.valid().then_some(cookie)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cookie(domain: &str, expires: Option<i64>, same_site: SameSite) -> Cookie {
        Cookie {
            domain: CookieDomain::parse(domain).unwrap(),
            path: "/".into(),
            name: "session".into(),
            value: Secret::new("fake-cookie".into()),
            expires,
            secure: true,
            http_only: true,
            same_site,
        }
    }
    #[test]
    fn inbox_keeps_files_until_accept_and_orders_batches() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path());
        inbox
            .post(&[cookie(".example.test", None, SameSite::Lax)])
            .unwrap();
        inbox
            .post(&[cookie(
                "example.test",
                Some(2_000_000_000),
                SameSite::Strict,
            )])
            .unwrap();
        let foreign = inbox.directory().join(".cookies-foreign.tmp");
        fs::write(&foreign, b"unfinished").unwrap();
        let batches = inbox.collect().unwrap();
        assert_eq!(batches.len(), 2);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(inbox.directory())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for batch in &batches {
            assert_eq!(
                fs::metadata(&batch.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let first = &batches[0].cookies[0];
        assert_eq!(first.domain.dotted(), ".example.test");
        assert_eq!(first.expires, None);
        assert_eq!(first.same_site, SameSite::Lax);
        assert!(first.value.expose() == "fake-cookie");
        assert!(!format!("{first:?}").contains("fake-cookie"));
        assert_eq!(batches[1].cookies[0].expires, Some(2_000_000_000));
        assert_eq!(batches[1].cookies[0].same_site, SameSite::Strict);
        assert_eq!(inbox.collect().unwrap().len(), 2);
        for batch in batches {
            batch.accept().unwrap();
        }
        assert!(inbox.collect().unwrap().is_empty());
        assert!(foreign.is_file());
    }
}
