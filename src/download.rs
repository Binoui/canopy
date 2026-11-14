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

/// Creates an empty file under the first free candidate name in `directory`, so no other
/// download or program can take that name before the transfer writes it. The path is valid
/// UTF-8, as WebKit expects.
pub fn reserve(directory: &Path, suggested: &str) -> io::Result<PathBuf> {
    if directory.to_str().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The download folder's path is not valid UTF-8.",
        ));
    }
    fs::create_dir_all(directory)?;
    for name in candidates(&file_name(suggested)).take(10_000) {
        let path = directory.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Every name for this file is taken.",
    ))
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

    #[test]
    fn reserving_never_reuses_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let first = reserve(directory.path(), "report.pdf").unwrap();
        fs::write(&first, "first").unwrap();
        let second = reserve(directory.path(), "report.pdf").unwrap();
        let third = reserve(directory.path(), "../report.pdf").unwrap();
        assert_eq!(first, directory.path().join("report.pdf"));
        assert_eq!(second, directory.path().join("report (1).pdf"));
        assert_eq!(third, directory.path().join("report (2).pdf"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "first");
    }

    #[test]
    fn reserving_skips_a_dangling_link_instead_of_following_it() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        let downloads = directory.path().join("Downloads");
        fs::create_dir(&downloads).unwrap();
        std::os::unix::fs::symlink(&outside, downloads.join("report.pdf")).unwrap();
        let path = reserve(&downloads, "report.pdf").unwrap();
        assert_eq!(path, downloads.join("report (1).pdf"));
        assert!(!outside.exists());
    }

    #[test]
    fn reserving_creates_a_missing_folder() {
        let directory = tempfile::tempdir().unwrap();
        let downloads = directory.path().join("a/Downloads");
        let path = reserve(&downloads, "file.zip").unwrap();
        assert_eq!(path.parent(), Some(downloads.as_path()));
        assert!(path.is_file());
    }
}
