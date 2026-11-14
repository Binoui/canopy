use std::{
    fs, io,
    path::{Path, PathBuf},
};

const FALLBACK: &str = "download";
/// Leaves room under the usual 255-byte name limit for a " (123)" suffix.
const MAX_NAME_BYTES: usize = 240;
const MAX_EXTENSION_BYTES: usize = 16;

/// A file name that stays inside its folder, whatever a server suggested.
pub fn file_name(suggested: &str) -> String {
    let last = suggested.rsplit(['/', '\\']).next().unwrap_or_default();
    let visible: String = last.chars().filter(|c| !c.is_control()).collect();
    let name = visible
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .trim_start();
    if name.is_empty() {
        return FALLBACK.into();
    }
    if name.len() <= MAX_NAME_BYTES {
        return name.into();
    }
    let (stem, extension) = split(name);
    if extension.len() > MAX_EXTENSION_BYTES {
        return truncate(name, MAX_NAME_BYTES).into();
    }
    format!(
        "{}{extension}",
        truncate(stem, MAX_NAME_BYTES - extension.len())
    )
}

/// `name`, then `stem (1).ext`, `stem (2).ext`, … in the order they should be tried.
pub fn candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, extension) = split(name);
    std::iter::once(name.to_owned()).chain((1..).map(move |n| format!("{stem} ({n}){extension}")))
}

/// Where a transfer is written before it gets its name.
pub struct Staging(tempfile::TempDir);

impl Staging {
    /// Creates a folder inside `directory` that only this user can enter, so nothing can
    /// swap the file or plant a link while the transfer runs, even in a shared folder.
    /// Dropping it removes whatever the transfer left.
    pub fn new(directory: &Path) -> io::Result<Self> {
        if directory.to_str().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The download folder's path is not valid UTF-8.",
            ));
        }
        fs::create_dir_all(directory)?;
        use std::os::unix::fs::PermissionsExt;
        tempfile::Builder::new()
            .prefix(".canopy-download-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(directory)
            .map(Self)
    }

    /// The path the transfer writes to. It is valid UTF-8, as WebKit expects.
    pub fn file(&self) -> PathBuf {
        self.0.path().join("incoming")
    }

    /// Moves the finished file next to the staging folder under the first free candidate
    /// name. The move never replaces a file and never follows a link.
    pub fn publish(self, suggested: &str) -> io::Result<PathBuf> {
        let directory = self.0.path().parent().unwrap_or(Path::new("/"));
        let mut staged = tempfile::TempPath::try_from_path(self.file())?;
        for name in candidates(&file_name(suggested)).take(10_000) {
            let target = directory.join(name);
            match staged.persist_noclobber(&target) {
                Ok(()) => return Ok(target),
                Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                    staged = error.path;
                }
                Err(error) => return Err(error.error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Every name for this file is taken.",
        ))
    }
}

fn split(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

fn truncate(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_an_ordinary_name() {
        assert_eq!(file_name("report.pdf"), "report.pdf");
        assert_eq!(file_name("Relevé de compte.pdf"), "Relevé de compte.pdf");
    }

    #[test]
    fn a_suggested_path_keeps_only_its_last_component() {
        assert_eq!(file_name("../../.bashrc"), "bashrc");
        assert_eq!(file_name("/etc/passwd"), "passwd");
        assert_eq!(file_name("..\\..\\boot.ini"), "boot.ini");
        assert_eq!(file_name("dir/"), FALLBACK);
    }

    #[test]
    fn names_that_mean_a_folder_or_nothing_fall_back() {
        for suggested in ["", ".", "..", "...", "  ", "\u{0}", "\n\t", "a/.."] {
            assert_eq!(file_name(suggested), FALLBACK, "{suggested:?}");
        }
    }

    #[test]
    fn hidden_and_control_characters_are_removed() {
        assert_eq!(file_name(".hidden"), "hidden");
        assert_eq!(file_name("re\u{0}port\n.pdf"), "report.pdf");
        assert_eq!(file_name("  notes.txt. . "), "notes.txt");
    }

    #[test]
    fn a_long_name_fits_with_room_for_a_counter_and_keeps_its_extension() {
        let name = file_name(&format!("{}.pdf", "é".repeat(400)));
        assert!(name.len() <= MAX_NAME_BYTES, "{}", name.len());
        assert!(name.ends_with("é.pdf"));
        let numbered = candidates(&name).nth(999).unwrap();
        assert!(numbered.len() <= 255, "{}", numbered.len());
        assert!(numbered.ends_with(" (999).pdf"));
        let odd = file_name(&format!("a.{}", "x".repeat(400)));
        assert_eq!(odd.len(), MAX_NAME_BYTES);
    }

    #[test]
    fn candidates_number_before_the_extension() {
        let tried: Vec<String> = candidates("report.pdf").take(3).collect();
        assert_eq!(tried, ["report.pdf", "report (1).pdf", "report (2).pdf"]);
        let tried: Vec<String> = candidates("README").take(2).collect();
        assert_eq!(tried, ["README", "README (1)"]);
    }

    fn save(directory: &Path, suggested: &str, bytes: &str) -> PathBuf {
        let staging = Staging::new(directory).unwrap();
        fs::write(staging.file(), bytes).unwrap();
        staging.publish(suggested).unwrap()
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn publishing_never_replaces_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let first = save(directory.path(), "report.pdf", "first");
        let second = save(directory.path(), "report.pdf", "second");
        let third = save(directory.path(), "../report.pdf", "third");
        assert_eq!(first, directory.path().join("report.pdf"));
        assert_eq!(second, directory.path().join("report (1).pdf"));
        assert_eq!(third, directory.path().join("report (2).pdf"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(fs::read_to_string(&second).unwrap(), "second");
        assert_eq!(
            names(directory.path()),
            ["report (1).pdf", "report (2).pdf", "report.pdf"],
            "no staging folder is left"
        );
    }

    #[test]
    fn publishing_skips_a_link_planted_during_the_transfer_instead_of_following_it() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, "precious").unwrap();
        let downloads = directory.path().join("Downloads");
        let staging = Staging::new(&downloads).unwrap();
        fs::write(staging.file(), "new").unwrap();
        std::os::unix::fs::symlink(&outside, downloads.join("report.pdf")).unwrap();
        std::os::unix::fs::symlink("missing", downloads.join("report (1).pdf")).unwrap();
        let path = staging.publish("report.pdf").unwrap();
        assert_eq!(path, downloads.join("report (2).pdf"));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "precious");
    }

    #[test]
    fn staging_creates_a_missing_folder_and_an_abandoned_transfer_leaves_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let downloads = directory.path().join("a/Downloads");
        let staging = Staging::new(&downloads).unwrap();
        fs::write(staging.file(), "partial").unwrap();
        fs::write(staging.file().with_extension("wkdownload"), "partial").unwrap();
        drop(staging);
        assert!(names(&downloads).is_empty());
    }

    #[test]
    fn a_staging_folder_is_closed_to_other_users() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path()).unwrap();
        let mode = fs::metadata(staging.file().parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}
