use serde_json::{json, Value};

use super::{
    tree_view::{self, Fit},
    view::{ago, plural},
    ChromeFrame,
};

pub(super) fn value(frame: &ChromeFrame) -> Value {
    let pinned: Vec<_> = frame
        .forest
        .pinned()
        .into_iter()
        .map(|tree| {
            let pages = plural(tree.page_count, "page");
            let meta = tree
                .last_event_at
                .map_or_else(|| pages.clone(), |at| format!("{pages} · {}", ago(at)));
            json!({
                "id": tree.id.root().to_string(), "title": tree.title,
                "tip": format!("{}\n{}", tree.title, frame.forest.visit(tree.id.root()).unwrap().url),
                "waiting": tree.pending_count, "meta": meta,
                "mini": mini_value(&tree_view::miniature(&frame.forest, tree, Fit::CARD)),
            })
        })
        .collect();
    json!({"pinned": pinned, "bookmarks": bookmarks(&frame.bookmarks)})
}

fn mini_value(mini: &tree_view::Miniature) -> Value {
    json!({ "w": mini.width, "h": mini.height, "s": mini.icon, "cut": mini.cut,
        "nodes": mini.nodes.iter().map(|n| json!({"id": n.id.to_string(), "x": n.x, "y": n.y, "last": n.last, "waiting": n.waiting})).collect::<Vec<_>>(),
        "edges": mini.edges.iter().map(|e| json!({"c": e.curve, "wait": e.kind == super::graph::EdgeKind::Pending})).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        browser::{Browser, StartSuggestions, ThemeMode},
        forest::ForestProjection,
    };

    #[test]
    fn cards_follow_pin_order_and_roots_without_events_show_only_the_page_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&dir.path().join("forest.sqlite3")).unwrap();
        let ids = browser
            .add_roots(&[
                url::Url::parse("https://example.org/a").unwrap(),
                url::Url::parse("https://example.org/b").unwrap(),
            ])
            .unwrap();
        let states = browser.tree_states().unwrap();
        browser.set_pinned(states[1].root, true).unwrap();
        browser.set_pinned(states[0].root, true).unwrap();
        let frame = ChromeFrame {
            forest: ForestProjection::from_snapshot(
                browser.snapshot().unwrap(),
                browser.tree_states().unwrap(),
            )
            .unwrap(),
            loaded: Default::default(),
            capture_enabled: true,
            theme: ThemeMode::Dark,
            suggestions: StartSuggestions::default(),
            bookmarks: vec![],
            files: vec![],
        };
        let start = value(&frame);
        assert_eq!(start["pinned"][0]["id"], ids[1].to_string());
        assert_eq!(start["pinned"][1]["id"], ids[0].to_string());
        assert_eq!(start["pinned"][0]["meta"], "1 page");
        assert_eq!(start["pinned"][0]["waiting"], 0);
        drop(browser);
        let reopened = Browser::open(&dir.path().join("forest.sqlite3")).unwrap();
        assert!(reopened
            .tree_states()
            .unwrap()
            .iter()
            .all(|t| t.pin.is_some()));
    }
}

pub(super) fn bookmark_label(b: &crate::browser::Bookmark) -> &str {
    if b.title.trim().is_empty() {
        super::view::short_host(&b.url)
    } else {
        &b.title
    }
}

fn bookmarks(bookmarks: &[crate::browser::Bookmark]) -> Value {
    use std::collections::BTreeMap;
    let row = |b: &crate::browser::Bookmark| json!({"id":b.id.get(), "label":bookmark_label(b), "url":b.url.as_str()});
    let top: Vec<_> = bookmarks.iter().filter(|b| b.folder.is_none()).collect();
    let count = if top.len() > 16 { 15 } else { top.len() };
    let mut folders = BTreeMap::<&str, Vec<_>>::new();
    for b in bookmarks {
        if let Some(name) = b.folder.as_deref() {
            folders.entry(name).or_default().push(b);
        }
    }
    let mut folders: Vec<_> = folders.into_iter().collect();
    folders.sort_by(|(a, aa), (b, bb)| bb.len().cmp(&aa.len()).then(a.cmp(b)));
    let width = |name: &str, count: usize| {
        super::view::text_width(name, 11., 0.56)
            + super::view::text_width(&count.to_string(), 11., 0.56)
            + 42.
    };
    let total: f64 = folders
        .iter()
        .map(|(name, rows)| width(name, rows.len()))
        .sum();
    let mut room = if total > 520. { 456. } else { 520. };
    let mut visible = 0;
    for (name, rows) in &folders {
        room -= width(name, rows.len());
        if room < 0. {
            break;
        }
        visible += 1;
    }
    let folder = |(name, rows): &(&str, Vec<&crate::browser::Bookmark>)| json!({"name":name,"count":rows.len(),"rows":rows.iter().map(|b| row(b)).collect::<Vec<_>>()});
    json!({
        "tiles": top[..count].iter().map(|b| row(b)).collect::<Vec<_>>(),
        "more": (top.len() > count).then(|| json!({"label":format!("{} more", top.len()-count),"rows":top[count..].iter().map(|b| row(b)).collect::<Vec<_>>() })),
        "folders": folders[..visible].iter().map(folder).collect::<Vec<_>>(),
        "hidden": (visible < folders.len()).then(|| json!({"label":format!("{} more", folders.len()-visible),"folders":folders[visible..].iter().map(folder).collect::<Vec<_>>() })),
        "empty": bookmarks.is_empty(),
    })
}

#[cfg(test)]
mod bookmark_tests {
    use super::*;
    use crate::browser::{Browser, ImportedBookmark};
    #[test]
    fn bookmark_band_has_sixteen_cells_with_n_more_and_sorted_folder_chips() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(&dir.path().join("db")).unwrap();
        let mut input = Vec::new();
        for i in 0..17 {
            input.push(ImportedBookmark {
                url: url::Url::parse(&format!("https://example.org/{i}")).unwrap(),
                title: String::new(),
                folder: "Bookmarks bar".into(),
                visits: 17 - i,
            });
        }
        for i in 0..20 {
            input.push(ImportedBookmark {
                url: url::Url::parse(&format!("https://example.org/folder/{i}")).unwrap(),
                title: "Folder".into(),
                folder: format!("Bookmarks bar › Folder {i}"),
                visits: 0,
            });
        }
        b.import_pages("test", &[], &input).unwrap();
        let rows = b.bookmarks().unwrap();
        let band = bookmarks(&rows);
        assert_eq!(band["tiles"].as_array().unwrap().len(), 15);
        assert_eq!(band["more"]["label"], "2 more");
        assert_eq!(band["tiles"][0]["label"], "example.org");
        assert_eq!(band["more"]["rows"][0]["url"], "https://example.org/15");
        assert_eq!(band["folders"][0]["name"], "Folder 0");
        assert_eq!(
            band["folders"].as_array().unwrap().len()
                + band["hidden"]["folders"].as_array().unwrap().len(),
            20
        );
        assert_eq!(bookmarks(&[])["empty"], true);
        b.remove_bookmark(rows[16].id).unwrap();
        let band = bookmarks(&b.bookmarks().unwrap());
        assert_eq!(band["tiles"].as_array().unwrap().len(), 16);
        assert!(band["more"].is_null());
    }
}
