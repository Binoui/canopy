//! Only the WebKit backend, the chrome and the environment setup may name the web engine's
//! crates. Everything else in `src/` reaches the engine through `src/pages/engine/`, so a
//! second engine can be added beside WebKit.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// The engine crates, as the code names them: `soup3` is used as `soup`, `javascriptcore-rs`
/// as `javascriptcore`.
const CRATES: [&str; 5] = [
    "webkit2gtk",
    "javascriptcore",
    "javascriptcore_rs",
    "soup",
    "soup3",
];

/// Paths under `src/` that may name them.
const ALLOWED: [&str; 3] = ["src/pages/engine/webkit/", "src/chrome/", "src/lib.rs"];

#[test]
fn only_the_backend_names_engine_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(!files.is_empty(), "no source files found");
    let mut found = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWED.iter().any(|allowed| relative.starts_with(allowed)) {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            if let Some(name) = names_crate(code) {
                found.push(format!(
                    "{relative}:{}: {name}: {}",
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "only src/pages/engine/webkit/, src/chrome/ and src/lib.rs may name {CRATES:?}; \
         reach the engine through src/pages/engine/ instead:\n{}",
        found.join("\n")
    );
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// The first engine crate `code` names as a whole word.
fn names_crate(code: &str) -> Option<&'static str> {
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .find_map(|word| CRATES.into_iter().find(|name| *name == word))
}

#[test]
fn whole_words_only() {
    assert_eq!(names_crate("use webkit2gtk::WebView;"), Some("webkit2gtk"));
    assert_eq!(names_crate("soup::Session::new()"), Some("soup"));
    assert_eq!(
        names_crate("let x: javascriptcore::Value;"),
        Some("javascriptcore")
    );
    assert_eq!(names_crate("let soupy = 1; // webkit"), None);
    assert_eq!(names_crate("WebKitView::new"), None);
}
