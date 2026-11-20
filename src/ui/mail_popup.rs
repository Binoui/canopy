use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::{json, Value};

const FILE: &str = "mail-popup";
const SAME_LOOK_MS: u64 = 60_000;

/// When the user last opened the mail list, and what counts as new during the current look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    opened_at: Option<i64>,
    new_after: Option<i64>,
}

impl Look {
    pub fn open(&mut self, now_ms: i64) -> bool {
        if self
            .opened_at
            .is_some_and(|last| last.abs_diff(now_ms) < SAME_LOOK_MS)
        {
            return false;
        }
        self.new_after = self.opened_at;
        self.opened_at = Some(now_ms);
        true
    }

    pub fn new_after(&self) -> Option<i64> {
        self.new_after
    }
}

pub(super) struct Setting {
    path: PathBuf,
    look: Cell<Look>,
    #[cfg(feature = "gui-smoke")]
    now_ms: Cell<Option<i64>>,
}

impl Setting {
    pub fn load(directory: &Path) -> Self {
        let path = directory.join(FILE);
        let look = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .and_then(|value: Value| {
                if value["version"].as_u64()? != 1 {
                    return None;
                }
                Some(Look {
                    opened_at: Some(value["openedAt"].as_i64()?),
                    new_after: if value.get("newAfter")?.is_null() {
                        None
                    } else {
                        Some(value["newAfter"].as_i64()?)
                    },
                })
            })
            .unwrap_or_default();
        Self {
            path,
            look: Cell::new(look),
            #[cfg(feature = "gui-smoke")]
            now_ms: Cell::default(),
        }
    }

    pub fn get(&self) -> Look {
        self.look.get()
    }

    pub fn open(&self) -> Result<()> {
        let now_ms = glib::real_time() / 1000;
        #[cfg(feature = "gui-smoke")]
        let now_ms = self.now_ms.get().unwrap_or(now_ms);
        let mut look = self.get();
        if look.open(now_ms) {
            self.look.set(look);
            self.save(look)?;
        }
        Ok(())
    }

    fn save(&self, look: Look) -> Result<()> {
        let value = json!({"version": 1, "openedAt": look.opened_at, "newAfter": look.new_after});
        super::settings::write(&self.path, value.to_string())
            .context("Could not save the last mail look")
    }

    #[cfg(feature = "gui-smoke")]
    pub fn set_now_ms(&self, now_ms: i64) {
        self.now_ms.set(Some(now_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_look_sets_no_cutoff_and_quick_reopens_keep_it() {
        let mut look = Look::default();
        assert!(look.open(100_000));
        assert_eq!(look.new_after(), None);
        assert!(!look.open(159_999));
        assert!(look.open(160_000));
        assert_eq!(look.new_after(), Some(100_000));
        assert!(!look.open(180_000));
        assert_eq!(look.new_after(), Some(100_000));
        assert!(look.open(220_000));
        assert_eq!(look.new_after(), Some(160_000));
    }

    #[test]
    fn a_restart_keeps_the_previous_look() {
        let directory = tempfile::tempdir().unwrap();
        let setting = Setting::load(directory.path());
        let mut look = setting.get();
        look.open(100_000);
        look.open(160_000);
        setting.save(look).unwrap();
        let mut restored = Setting::load(directory.path()).get();
        assert!(!restored.open(180_000));
        assert_eq!(restored.new_after(), Some(100_000));
        assert!(restored.open(220_000));
        assert_eq!(restored.new_after(), Some(160_000));
    }

    #[test]
    fn missing_unreadable_and_unknown_files_start_with_no_previous_look() {
        let directory = tempfile::tempdir().unwrap();
        let check = || {
            let mut look = Setting::load(directory.path()).get();
            assert!(look.open(100_000));
            assert_eq!(look.new_after(), None);
        };
        check();
        let path = directory.path().join(FILE);
        fs::create_dir(&path).unwrap();
        check();
        fs::remove_dir(&path).unwrap();
        for bytes in [
            &b"broken"[..],
            br#"{"version":2,"openedAt":40000,"newAfter":null}"#,
            br#"{"version":1,"openedAt":"yesterday","newAfter":null}"#,
        ] {
            fs::write(&path, bytes).unwrap();
            check();
        }
    }
}
