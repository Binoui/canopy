use crate::chrome::CanopyLayout;
use anyhow::{Context, Result};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
};

const FILE: &str = "canopy-layout";

pub(super) struct Setting {
    directory: PathBuf,
    layout: Cell<CanopyLayout>,
}
impl Setting {
    pub fn load(directory: &Path) -> Self {
        let layout = std::fs::read_to_string(directory.join(FILE))
            .ok()
            .and_then(|s| CanopyLayout::parse(s.trim()))
            .unwrap_or_default();
        Self {
            directory: directory.to_owned(),
            layout: Cell::new(layout),
        }
    }
    pub fn get(&self) -> CanopyLayout {
        self.layout.get()
    }
    pub fn save(&self, layout: CanopyLayout) -> Result<()> {
        crate::atomic_file::write(&self.directory.join(FILE), format!("{}\n", layout.name()))
            .context("Could not save the Canopy layout")?;
        self.layout.set(layout);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_unreadable_and_unknown_choices_default_to_flow() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Setting::load(dir.path()).get(), CanopyLayout::Flow);
        std::fs::create_dir(dir.path().join(FILE)).unwrap();
        assert_eq!(Setting::load(dir.path()).get(), CanopyLayout::Flow);
        std::fs::remove_dir(dir.path().join(FILE)).unwrap();
        for text in [b"unknown".as_slice(), b"\xff"] {
            std::fs::write(dir.path().join(FILE), text).unwrap();
            assert_eq!(Setting::load(dir.path()).get(), CanopyLayout::Flow);
        }
    }
    #[test]
    fn choice_round_trips_and_failed_save_keeps_previous_choice() {
        let dir = tempfile::tempdir().unwrap();
        let setting = Setting::load(dir.path());
        for layout in [CanopyLayout::Islands, CanopyLayout::Flow] {
            setting.save(layout).unwrap();
            assert_eq!(setting.get(), layout);
            assert_eq!(Setting::load(dir.path()).get(), layout);
        }
        std::fs::create_dir(dir.path().join("canopy-layout.tmp")).unwrap();
        assert!(setting.save(CanopyLayout::Islands).is_err());
        assert_eq!(setting.get(), CanopyLayout::Flow);
        assert_eq!(Setting::load(dir.path()).get(), CanopyLayout::Flow);
    }
}
