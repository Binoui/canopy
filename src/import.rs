//! Reads a Chrome profile's history and bookmarks for Canopy's search. Nothing is loaded,
//! and cookies, passwords and form data are never read.

use std::{collections::HashMap, path::Path, time::Duration};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::{address::web_url, browser::ImportedPage};

/// Chrome counts microseconds from 1601-01-01; this is that epoch in Unix milliseconds.
const CHROME_EPOCH_MS: i64 = -11_644_473_600_000;

/// Every web page in the profile's history and bookmarks, one entry per URL.
pub fn chrome_profile(profile: &Path) -> Result<Vec<ImportedPage>> {
    let history = profile.join("History");
    let bookmarks = profile.join("Bookmarks");
    if !history.is_file() && !bookmarks.is_file() {
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
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(5))?;
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
        let pages = chrome_profile(dir.path()).unwrap();
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
