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
    json!({"pinned": pinned})
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
