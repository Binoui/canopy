//! Cards and menus for a tree outside the strip.

use serde_json::{json, Value};

use crate::forest::{ForestProjection, TreeSummary};

use super::{
    tree_view::{self, Fit},
    view::{ago, plural},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Place {
    Tab,
    NewTreePage,
}

pub(super) fn menu(tree: &TreeSummary, place: Place) -> Value {
    let pin = json!({
        "kind": "pin",
        "label": if tree.pin.is_some() { "Remove from saved" } else { "Save tree" },
        "pinned": tree.pin.is_none(),
    });
    if place == Place::NewTreePage {
        return json!([pin]);
    }
    let mut rows = vec![
        json!({"kind": "rename", "label": "Rename", "name": tree.title}),
        pin,
    ];
    if tree.lifecycle.is_open() {
        rows.push(json!({"kind": "settle", "label": "Settle tree"}));
    }
    json!(rows)
}

pub(super) fn card(forest: &ForestProjection, tree: &TreeSummary, place: Place) -> Value {
    let pages = plural(tree.page_count, "page");
    let meta = tree
        .last_event_at
        .map_or_else(|| pages.clone(), |at| format!("{pages} · {}", ago(at)));
    json!({
        "id": tree.id.root().to_string(), "title": tree.title,
        "tip": format!("{}, saved\n{}", tree.title, forest.visit(tree.id.root()).unwrap().url),
        "name": format!("{}, {}{}, saved", tree.title, pages,
            if tree.pending_count > 0 { format!(", {} waiting", tree.pending_count) } else { String::new() }),
        "waiting": tree.pending_count, "meta": meta,
        "mini": mini_value(forest, &tree_view::miniature(forest, tree, Fit::CARD)),
        "menu": menu(tree, place),
    })
}

pub(super) fn mini_value(
    forest: &crate::forest::ForestProjection,
    mini: &tree_view::Miniature,
) -> Value {
    json!({ "w": mini.width, "h": mini.height, "s": mini.icon, "cut": mini.cut,
        "nodes": mini.nodes.iter().map(|n| json!({"id": n.id.to_string(), "url": forest.visit(n.id).map(|v| v.url.as_str()), "x": n.x, "y": n.y, "last": n.last, "waiting": n.waiting})).collect::<Vec<_>>(),
        "edges": mini.edges.iter().map(|e| json!({"c": e.curve, "wait": e.kind == super::graph::EdgeKind::Pending})).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{browser::Browser, forest::ForestProjection};

    #[test]
    fn menu_rows_follow_place_saved_state_and_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&dir.path().join("forest.sqlite3")).unwrap();
        let root = browser
            .add_roots(&[url::Url::parse("https://example.org/project").unwrap()])
            .unwrap()[0];
        let id = crate::browser::TreeId(root);
        browser.rename_tree(id, "Project").unwrap();
        for open in [true, false] {
            if !open {
                browser.settle_tree(id).unwrap();
            }
            for saved in [false, true] {
                browser.set_pinned(id, saved).unwrap();
                let forest = ForestProjection::from_snapshot(
                    browser.snapshot().unwrap(),
                    browser.tree_states().unwrap(),
                )
                .unwrap();
                let tree = &forest.trees()[0];
                let pin = json!({
                    "kind": "pin",
                    "label": if saved { "Remove from saved" } else { "Save tree" },
                    "pinned": !saved,
                });
                let mut expected = vec![
                    json!({"kind": "rename", "label": "Rename", "name": "Project"}),
                    pin.clone(),
                ];
                if open {
                    expected.push(json!({"kind": "settle", "label": "Settle tree"}));
                }
                assert_eq!(menu(tree, Place::Tab), json!(expected));
                assert_eq!(menu(tree, Place::NewTreePage), json!([pin]));
                if saved {
                    let card = card(&forest, tree, Place::NewTreePage);
                    assert_eq!(card["menu"], menu(tree, Place::NewTreePage));
                    assert_eq!(card["name"], "Project, 1 page, saved");
                    assert_eq!(
                        card["mini"]["nodes"][0]["url"],
                        "https://example.org/project"
                    );
                }
            }
        }
    }
}
