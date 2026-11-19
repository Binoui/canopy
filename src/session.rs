//! What `canopy session` reports: the open trees and their pages as the window shows them,
//! as JSON for agents and scripts, and as text for a person or a quick look.

use std::{collections::HashSet, fmt::Write};

use serde_json::{json, Value};

use crate::{
    browser::{PendingTarget, VisitId},
    forest::{ForestProjection, TreeSummary},
};

/// The window's state at one moment, gathered by the running Canopy.
pub struct Seen<'a> {
    pub forest: &'a ForestProjection,
    /// Hidden pages still in memory, shown again without a reload.
    pub live: &'a HashSet<VisitId>,
    /// Waiting pages already loaded in the background.
    pub loaded: &'a HashSet<VisitId>,
    /// Where the shown page is headed before it commits.
    pub pending: Option<&'a PendingTarget>,
    /// The shown page is loading.
    pub loading: bool,
}

/// The session as JSON. Settled pages and dismissed waiting pages are left out, as the strip
/// leaves them out. History trees are left out: they are in the forest, not on screen.
pub fn describe(seen: &Seen) -> Value {
    let forest = seen.forest;
    let current = forest.current();
    let shown_tree = current.and_then(|id| forest.tree_of(id));
    let page = |id: VisitId| {
        let visit = forest.visit(id)?;
        Some(json!({ "id": id.to_string(), "title": visit.label(), "url": visit.url.as_str() }))
    };
    let tabs: Vec<Value> = forest
        .tabs()
        .iter()
        .map(|tree| {
            json!({
                "id": tree.id.root().to_string(),
                "name": tree.title,
                "pinned": tree.place.pinned(),
                "shown": Some(tree.id) == shown_tree,
                "waiting": tree.pending_count,
                "last_page": tree.last_visit.to_string(),
                "pages": pages(seen, tree),
            })
        })
        .collect();
    let saved: Vec<Value> = forest
        .shelf()
        .into_iter()
        .map(|tree| {
            json!({
                "id": tree.id.root().to_string(),
                "name": tree.title,
                "page_count": tree.page_count,
                "last_page": page(tree.last_visit),
            })
        })
        .collect();
    let shown = current.and_then(|id| {
        let mut shown = page(id)?;
        shown["tree"] = json!(shown_tree.map(|tree| tree.root().to_string()));
        shown["path"] = forest
            .path(id)
            .into_iter()
            .filter_map(&page)
            .collect::<Vec<_>>()
            .into();
        shown["loading"] = seen.loading.into();
        Some(shown)
    });
    let pending = seen.pending.map(|target| match target {
        PendingTarget::Visit(id) => json!({ "page": page(*id) }),
        PendingTarget::NewTree(url) => json!({ "new_tree": url.as_str() }),
    });
    json!({ "shown": shown, "pending": pending, "tabs": tabs, "saved": saved })
}

/// The tree's pages, parents before children and siblings oldest first, with their depth.
fn pages(seen: &Seen, tree: &TreeSummary) -> Vec<Value> {
    let forest = seen.forest;
    let mut pages = Vec::new();
    let mut stack = vec![(tree.id.root(), 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(visit) = forest.visit(id) else {
            continue;
        };
        let waiting = forest.is_waiting(id);
        if forest.is_settled(id) || (visit.opened_at.is_none() && !waiting) {
            continue;
        }
        let mut marks = Vec::new();
        if forest.current() == Some(id) {
            marks.push("shown");
        }
        if seen.live.contains(&id) {
            marks.push("in memory");
        }
        if waiting {
            marks.push("waiting");
            if seen.loaded.contains(&id) {
                marks.push("loaded");
            }
        }
        pages.push(json!({
            "id": id.to_string(),
            "parent": visit.parent.map(|p| p.to_string()),
            "depth": depth,
            "title": visit.label(),
            "url": visit.url.as_str(),
            "marks": marks,
        }));
        stack.extend(forest.children(id).iter().rev().map(|&c| (c, depth + 1)));
    }
    pages
}

/// The session as text: the shown page, then each open tree with its pages indented.
pub fn render(session: &Value) -> String {
    let mut out = String::new();
    let line = |page: &Value| {
        format!(
            "{} — {}",
            page["title"].as_str().unwrap_or(""),
            page["url"].as_str().unwrap_or("")
        )
    };
    match &session["shown"] {
        Value::Null => out.push_str("Shown: nothing (New tree tab or start page)\n"),
        shown => {
            let _ = writeln!(out, "Shown: {}", line(shown));
            if shown["loading"] == true {
                out.push_str("  loading\n");
            }
        }
    }
    if let Some(pending) = session["pending"].as_object() {
        if let Some(url) = pending.get("new_tree").and_then(Value::as_str) {
            let _ = writeln!(out, "Opening: {url} as a new tree");
        } else if let Some(page) = pending.get("page").filter(|p| !p.is_null()) {
            let _ = writeln!(out, "Opening: {}", line(page));
        }
    }
    let tabs = session["tabs"].as_array().map_or(&[][..], Vec::as_slice);
    let _ = writeln!(out, "\nOpen trees ({}):", tabs.len());
    for (i, tree) in tabs.iter().enumerate() {
        let pages = tree["pages"].as_array().map_or(&[][..], Vec::as_slice);
        let mut about = vec![format!("{} pages", pages.len())];
        if let Some(waiting) = tree["waiting"].as_u64().filter(|&n| n > 0) {
            about.push(format!("{waiting} waiting"));
        }
        if tree["pinned"] == true {
            about.push("pinned".into());
        }
        let _ = writeln!(
            out,
            "{} {}. {} ({})",
            if tree["shown"] == true { "*" } else { " " },
            i + 1,
            tree["name"].as_str().unwrap_or(""),
            about.join(", ")
        );
        for page in pages {
            let depth = page["depth"].as_u64().unwrap_or(0) as usize;
            let marks: Vec<&str> = page["marks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let _ = write!(out, "    {}{}", "  ".repeat(depth), line(page));
            if !marks.is_empty() {
                let _ = write!(out, "  [{}]", marks.join(", "));
            }
            out.push('\n');
        }
    }
    let saved = session["saved"].as_array().map_or(&[][..], Vec::as_slice);
    if !saved.is_empty() {
        let _ = writeln!(out, "\nSaved for later ({}):", saved.len());
        for tree in saved {
            let _ = writeln!(
                out,
                "    {} ({} pages)",
                tree["name"].as_str().unwrap_or(""),
                tree["page_count"]
            );
        }
    }
    if let Some(text) = session["text"].as_str() {
        let _ = writeln!(out, "\nShown page text:\n{text}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, Spot, TreeId, TreeState, Visit, VisitState};
    use url::Url;

    fn visit(id: VisitId, parent: Option<VisitId>, at: i64, state: VisitState) -> Visit {
        Visit {
            dismissed_at: None,
            branch_settled_at: None,
            id,
            parent,
            url: Url::parse(&format!("https://example.org/{at}")).unwrap(),
            title: format!("Page {at}"),
            created_at: at,
            opened_at: (state == VisitState::Opened).then_some(at),
            state,
            source: None,
        }
    }

    fn ids(n: usize) -> Vec<VisitId> {
        (0..n)
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect()
    }

    /// One tab tree: 0 → (1 → 3 waiting, 2 settled) shown at 1, and a saved tree 4.
    fn forest(ids: &[VisitId]) -> ForestProjection {
        let mut settled = visit(ids[2], Some(ids[0]), 2, VisitState::Opened);
        settled.branch_settled_at = Some(5);
        let visits = vec![
            visit(ids[0], None, 0, VisitState::Opened),
            visit(ids[1], Some(ids[0]), 1, VisitState::Opened),
            settled,
            visit(ids[3], Some(ids[1]), 3, VisitState::Queued),
            visit(ids[4], None, 4, VisitState::Opened),
        ];
        let state = |root: VisitId, spot| TreeState {
            last_event_at: None,
            root: TreeId(root),
            last_visit: root,
            name: None,
            spot,
        };
        ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: Some(ids[1]),
            },
            vec![
                state(
                    ids[0],
                    Spot::Tab {
                        pinned: true,
                        position: 0,
                    },
                ),
                state(
                    ids[4],
                    Spot::Shelf {
                        position: 0,
                        saved_at: 4,
                    },
                ),
            ],
        )
        .unwrap()
    }

    #[test]
    fn the_session_lists_open_trees_with_their_visible_pages_and_marks() {
        let ids = ids(5);
        let forest = forest(&ids);
        let live = HashSet::from([ids[0]]);
        let loaded = HashSet::from([ids[3]]);
        let session = describe(&Seen {
            forest: &forest,
            live: &live,
            loaded: &loaded,
            pending: None,
            loading: true,
        });
        assert_eq!(session["shown"]["id"], ids[1].to_string());
        assert_eq!(session["shown"]["loading"], true);
        assert_eq!(session["shown"]["path"].as_array().unwrap().len(), 2);
        let tab = &session["tabs"][0];
        assert_eq!(tab["shown"], true);
        assert_eq!(tab["pinned"], true);
        assert_eq!(tab["waiting"], 1);
        let pages: Vec<(String, u64, Vec<String>)> = tab["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["title"].as_str().unwrap().to_owned(),
                    p["depth"].as_u64().unwrap(),
                    serde_json::from_value(p["marks"].clone()).unwrap(),
                )
            })
            .collect();
        assert_eq!(
            pages,
            [
                ("Page 0".into(), 0, vec!["in memory".into()]),
                ("Page 1".into(), 1, vec!["shown".into()]),
                ("Page 3".into(), 2, vec!["waiting".into(), "loaded".into()]),
            ],
            "the settled page is left out"
        );
        assert_eq!(session["saved"][0]["page_count"], 1);
        let text = render(&session);
        assert!(
            text.starts_with("Shown: Page 1 — https://example.org/1\n  loading\n"),
            "{text}"
        );
        assert!(text.contains("* 1. "), "{text}");
        assert!(text.contains("(3 pages, 1 waiting, pinned)"), "{text}");
        assert!(
            text.contains("        Page 3 — https://example.org/3  [waiting, loaded]"),
            "{text}"
        );
        assert!(text.contains("Saved for later (1):"), "{text}");
    }

    #[test]
    fn a_new_tree_on_its_way_is_reported_before_it_exists() {
        let ids = ids(5);
        let forest = forest(&ids);
        let url = Url::parse("https://example.com/").unwrap();
        let pending = PendingTarget::NewTree(url);
        let session = describe(&Seen {
            forest: &forest,
            live: &HashSet::new(),
            loaded: &HashSet::new(),
            pending: Some(&pending),
            loading: false,
        });
        assert_eq!(session["pending"]["new_tree"], "https://example.com/");
        assert!(render(&session).contains("Opening: https://example.com/ as a new tree"));
    }
}
