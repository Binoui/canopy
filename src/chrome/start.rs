use serde_json::{json, Value};

use super::{
    tree_view::{self, Fit},
    ChromeFrame,
};

pub(super) fn value(frame: &ChromeFrame, frozen: &[crate::suggest::Suggestion]) -> Value {
    let shelf: Vec<_> = frame
        .forest
        .shelf()
        .into_iter()
        .map(|tree| super::tree_card::shelf_card(&frame.forest, tree))
        .collect();
    json!({"shelfTitle":"Saved for later","shelfHint":"Save for later in a tab’s menu takes a tree off the bar and keeps it here.","shelf": shelf, "bookmarks": bookmarks(&frame.bookmarks), "suggested": suggested(frame, frozen)})
}

/// The chips still on offer, each in the place it had when the band was ranked, so the
/// ones that stay never slide under the pointer when one leaves.
fn suggested(frame: &ChromeFrame, frozen: &[crate::suggest::Suggestion]) -> Vec<Value> {
    frozen
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            crate::suggest::admits(&frame.forest, &frame.bookmarks, &frame.declined, s)
        })
        .map(|(slot, s)| {
            let mut chip = chip(&frame.forest, s);
            chip["slot"] = json!(slot);
            chip
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        browser::{Browser, ThemeMode},
        forest::ForestProjection,
    };

    #[test]
    fn a_chip_that_leaves_does_not_move_the_ones_that_stay() {
        let dir = tempfile::tempdir().unwrap();
        let browser = Browser::open(&dir.path().join("forest.sqlite3")).unwrap();
        let page = |path: &str| crate::suggest::Suggestion::Page {
            url: url::Url::parse(&format!("https://example.org/{path}")).unwrap(),
            title: path.into(),
            why: crate::suggest::PageWhy::ThisWeek(2),
        };
        let frozen = [page("a"), page("b"), page("c")];
        let frame = ChromeFrame {
            canopy_layout: Default::default(),
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            mail_view: Default::default(),
            forest: ForestProjection::from_snapshot(
                browser.snapshot().unwrap(),
                browser.tree_states().unwrap(),
            )
            .unwrap(),
            loaded: Default::default(),
            live: Default::default(),
            playing: Default::default(),
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: [frozen[0].key()].into(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: vec![],
        };
        let start = value(&frame, &frozen);
        assert_eq!(start["suggested"].as_array().unwrap().len(), 2);
        assert_eq!(start["suggested"][0]["title"], "b");
        assert_eq!(start["suggested"][0]["slot"], 1);
        assert_eq!(start["suggested"][1]["slot"], 2);
    }

    #[test]
    fn cards_follow_shelf_order_and_roots_without_events_show_only_the_page_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&dir.path().join("forest.sqlite3")).unwrap();
        let ids = browser
            .add_roots(&[
                url::Url::parse("https://example.org/a").unwrap(),
                url::Url::parse("https://example.org/b").unwrap(),
            ])
            .unwrap();
        let states = browser.tree_states().unwrap();
        browser
            .place_tree(
                states[1].root,
                crate::browser::Move::Shelf(crate::browser::Slot::End),
            )
            .unwrap();
        browser
            .place_tree(
                states[0].root,
                crate::browser::Move::Shelf(crate::browser::Slot::End),
            )
            .unwrap();
        let frame = ChromeFrame {
            canopy_layout: Default::default(),
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            mail_view: Default::default(),
            forest: ForestProjection::from_snapshot(
                browser.snapshot().unwrap(),
                browser.tree_states().unwrap(),
            )
            .unwrap(),
            loaded: Default::default(),
            live: Default::default(),
            playing: Default::default(),
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: vec![],
        };
        let start = value(&frame, &[]);
        assert_eq!(start["shelf"][0]["id"], ids[1].to_string());
        assert_eq!(start["shelf"][1]["id"], ids[0].to_string());
        assert_eq!(start["shelf"][0]["meta"], "1 page");
        assert_eq!(start["shelf"][0]["waiting"], 0);
        drop(browser);
        let reopened = Browser::open(&dir.path().join("forest.sqlite3")).unwrap();
        assert!(reopened
            .tree_states()
            .unwrap()
            .iter()
            .all(|t| matches!(t.spot.place(), crate::browser::Place::Shelf { .. })));
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

pub(super) fn chip(
    forest: &crate::forest::ForestProjection,
    s: &crate::suggest::Suggestion,
) -> Value {
    use crate::suggest::{site, Suggestion};
    let (kind, title, tip, glyph) = match s {
        Suggestion::Page { url, title, .. } => {
            let label = if title.trim().is_empty() {
                site(url)
            } else {
                title
            };
            (
                "page",
                label.to_owned(),
                format!("{label}\n{url}"),
                json!({"url":url.as_str()}),
            )
        }
        Suggestion::Tree { tree, .. } => {
            let t = forest.trees().iter().find(|t| t.id == *tree).unwrap();
            (
                "tree",
                t.title.clone(),
                format!("{}\n{}", t.title, forest.visit(tree.root()).unwrap().url),
                json!({"mini":super::tree_card::mini_value(forest, &tree_view::miniature(forest,t,Fit::CHIP))}),
            )
        }
        Suggestion::Path { first, second, .. } => (
            "path",
            format!("{}, then {}", site(first), site(second)),
            format!("{first}\n{second}"),
            json!({"first":first.as_str(),"second":second.as_str()}),
        ),
    };
    json!({"key":s.key().to_string(),"kind":kind,"title":title,"tip":tip,"why":s.why(),"glyph":glyph})
}

pub(super) fn open(s: &crate::suggest::Suggestion) -> super::ChromeIntent {
    use crate::suggest::Suggestion;
    match s {
        Suggestion::Page { url, .. } => super::ChromeIntent::Navigate {
            url: url.clone(),
            target: crate::browser::NavigationTarget::Root,
        },
        Suggestion::Tree { tree, .. } => super::ChromeIntent::SelectTree(*tree),
        Suggestion::Path { first, second, .. } => super::ChromeIntent::OpenPath {
            first: first.clone(),
            second: second.clone(),
        },
    }
}
pub(super) fn keep(s: &crate::suggest::Suggestion) -> super::ChromeIntent {
    use crate::suggest::{site, Suggestion};
    match s {
        Suggestion::Page { url, title, .. } => super::ChromeIntent::Bookmark {
            url: url.clone(),
            title: title.clone(),
        },
        Suggestion::Tree { tree, .. } => super::ChromeIntent::PlaceTree {
            tree: *tree,
            to: crate::browser::Move::Shelf(crate::browser::Slot::First),
        },
        Suggestion::Path { first, .. } => super::ChromeIntent::Bookmark {
            url: first.clone(),
            title: site(first).into(),
        },
    }
}

pub(super) fn help() -> Value {
    json!({
        "title": "How trees work",
        "lines": [
            {"mark": "tree", "text": "Every link you follow grows the tree in the strip above. Back goes to the parent page."},
            {"mark": "waiting", "text": "Middle-click a link and it waits as a tab after the current page. Ctrl+Tab opens the first waiting tab. Hold Ctrl to pick a waiting or read page."},
            {"mark": "branches", "text": "Rest on a strip page to see its branches. Right-click a page for more: settle a branch, move it to its own tree, dismiss a waiting page."},
            {"mark": "settle", "text": "Done with a tree? Settle it with the cross on its tab. Nothing is deleted; Ctrl+Shift+T brings it back."}
        ],
        "legend": [
            [{"mark": "waiting", "text": "waiting"}],
            [{"mark": "unread", "text": "not read yet"}],
            [{"mark": "live", "text": "still open"}],
            [{"mark": "leaf", "text": "downloaded file"}],
            [{"mark": "branches", "text": "read on other branches"}]
        ]
    })
}
