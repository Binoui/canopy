use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
};

use crate::chrome::Grouping;
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

#[derive(Clone, Copy, Default)]
pub(super) struct Preferences {
    pub look: Look,
    pub grouping: Grouping,
}

impl Preferences {
    fn parse(value: Value) -> Option<Self> {
        if value["version"].as_u64()? != 1 {
            return None;
        }
        let timestamp = |key| match value.get(key)? {
            Value::Null => Some(None),
            n => n.as_i64().map(Some),
        };
        let look = Look {
            opened_at: timestamp("openedAt")?,
            new_after: timestamp("newAfter")?,
        };
        if look.opened_at.is_none() && look.new_after.is_some() {
            return None;
        }
        let grouping = match value.get("grouping") {
            None => Grouping::Day,
            Some(choice) => Grouping::parse(choice.as_str()?)?,
        };
        Some(Self { look, grouping })
    }
}

pub(super) struct Setting {
    path: PathBuf,
    preferences: Cell<Preferences>,
    #[cfg(feature = "gui-smoke")]
    now_ms: Cell<Option<i64>>,
}

impl Setting {
    pub fn load(directory: &Path) -> Self {
        let path = directory.join(FILE);
        let preferences = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .and_then(Preferences::parse)
            .unwrap_or_default();
        Self {
            path,
            preferences: Cell::new(preferences),
            #[cfg(feature = "gui-smoke")]
            now_ms: Cell::default(),
        }
    }

    pub fn get(&self) -> Preferences {
        self.preferences.get()
    }

    pub fn open(&self) -> Result<()> {
        let now_ms = glib::real_time() / 1000;
        #[cfg(feature = "gui-smoke")]
        let now_ms = self.now_ms.get().unwrap_or(now_ms);
        let mut current = self.get();
        if current.look.open(now_ms) {
            self.preferences.set(current);
            self.save(current)
                .context("Could not save the last mail look")?;
        }
        Ok(())
    }

    pub fn set_grouping(&self, grouping: Grouping) -> Result<()> {
        let mut current = self.get();
        if current.grouping != grouping {
            current.grouping = grouping;
            self.save(current)
                .context("Could not save the mail grouping")?;
            self.preferences.set(current);
        }
        Ok(())
    }

    fn save(&self, preferences: Preferences) -> std::io::Result<()> {
        let value = json!({
            "version": 1,
            "openedAt": preferences.look.opened_at,
            "newAfter": preferences.look.new_after,
            "grouping": preferences.grouping.name(),
        });
        super::settings::write(&self.path, value.to_string())
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
        let mut state = setting.get();
        state.look.open(100_000);
        state.look.open(160_000);
        setting.save(state).unwrap();
        let mut restored = Setting::load(directory.path()).get().look;
        assert!(!restored.open(180_000));
        assert_eq!(restored.new_after(), Some(100_000));
        assert!(restored.open(220_000));
        assert_eq!(restored.new_after(), Some(160_000));
    }

    #[test]
    fn grouping_loads_old_files_and_saves_without_losing_the_look() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join(FILE),
            br#"{"version":1,"openedAt":160000,"newAfter":100000}"#,
        )
        .unwrap();
        let setting = Setting::load(directory.path());
        assert_eq!(setting.get().grouping, Grouping::Day);
        setting.set_grouping(Grouping::Sender).unwrap();
        let restored = Setting::load(directory.path());
        assert_eq!(restored.get().grouping, Grouping::Sender);
        assert_eq!(restored.get().look.new_after(), Some(100_000));
        assert!(!restored.get().look.open(180_000));

        fs::create_dir(directory.path().join(FILE).with_extension("tmp")).unwrap();
        assert!(restored.set_grouping(Grouping::Day).is_err());
        assert_eq!(restored.get().grouping, Grouping::Sender);
        assert_eq!(
            Setting::load(directory.path()).get().grouping,
            Grouping::Sender
        );
    }

    #[test]
    fn missing_unreadable_and_unknown_files_start_with_no_previous_look() {
        let directory = tempfile::tempdir().unwrap();
        let check = || {
            let mut look = Setting::load(directory.path()).get().look;
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
