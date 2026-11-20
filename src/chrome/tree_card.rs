//! Cards and menus for a tree outside the strip.

use serde_json::{json, Value};

use crate::forest::{ForestProjection, TreeSummary};

use super::{
    tree_view::{self, Fit},
    view::{ago, plural},
};

use super::ChromeIntent;
use crate::browser::{Move, Place, Slot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Act {
    Pin,
    Unpin,
    Save,
    Rename,
    Settle,
    Open,
    Delete,
}
impl Act {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pin" => Self::Pin,
            "unpin" => Self::Unpin,
            "save" => Self::Save,
            "rename" => Self::Rename,
            "settle" => Self::Settle,
            "open" => Self::Open,
            "delete" => Self::Delete,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Pin => "pin",
            Self::Unpin => "unpin",
            Self::Save => "save",
            Self::Rename => "rename",
            Self::Settle => "settle",
            Self::Open => "open",
            Self::Delete => "delete",
        }
    }
    pub fn label(self, place: Place) -> &'static str {
        match (self, place) {
            (Self::Pin, _) => "Pin tab",
            (Self::Unpin, _) => "Unpin tab",
            (Self::Save, _) => "Save for later",
            (Self::Rename, _) => "Rename",
            (Self::Settle, _) => "Settle tree",
            (Self::Open, Place::History { .. }) => "Reopen",
            (Self::Open, _) => "Open",
            (Self::Delete, Place::History { .. }) => "Delete",
            (Self::Delete, _) => "Delete tree",
        }
    }
}
pub(super) fn acts(place: Place) -> &'static [Act] {
    match place {
        Place::Tab { pinned: false } => {
            &[Act::Pin, Act::Save, Act::Rename, Act::Settle, Act::Delete]
        }
        Place::Tab { pinned: true } => {
            &[Act::Unpin, Act::Save, Act::Rename, Act::Settle, Act::Delete]
        }
        Place::Shelf { .. } => &[Act::Open, Act::Rename, Act::Settle, Act::Delete],
        Place::History { .. } => &[Act::Open, Act::Save, Act::Rename, Act::Delete],
    }
}
pub(super) fn menu(tree: &TreeSummary) -> Value {
    buttons(tree, acts(tree.place))
}
pub(super) fn buttons(tree: &TreeSummary, keep: &[Act]) -> Value {
    json!(acts(tree.place).iter().filter(|a|keep.contains(a)).map(|a|json!({"act":a.name(),"label":a.label(tree.place),"name":tree.title,"go":*a==Act::Open})).collect::<Vec<_>>())
}
pub(super) fn intent(tree: &TreeSummary, act: Act) -> Option<ChromeIntent> {
    if !matches!(act, Act::Open) && !acts(tree.place).contains(&act) {
        return None;
    }
    Some(match act {
        Act::Pin => ChromeIntent::PlaceTree {
            tree: tree.id,
            to: Move::Pin(true),
        },
        Act::Unpin => ChromeIntent::PlaceTree {
            tree: tree.id,
            to: Move::Pin(false),
        },
        Act::Save => ChromeIntent::PlaceTree {
            tree: tree.id,
            to: Move::Shelf(Slot::First),
        },
        Act::Settle => ChromeIntent::PlaceTree {
            tree: tree.id,
            to: Move::History,
        },
        Act::Open => ChromeIntent::SelectTree(tree.id),
        Act::Delete if matches!(tree.place, Place::History { .. }) => {
            ChromeIntent::DeleteTrees(vec![tree.id])
        }
        Act::Delete => ChromeIntent::DeleteBranch(tree.id.root()),
        Act::Rename => return None,
    })
}
pub(super) fn card(forest: &ForestProjection, tree: &TreeSummary, current: bool) -> Value {
    let pages = plural(tree.page_count, "page");
    let sites = sites(forest, tree);
    json!({"id":tree.id.root().to_string(),"title":tree.title,"tip":format!("{}\n{}",tree.title,forest.visit(tree.id.root()).unwrap().url),
        "name":format!("{}, {}{}",tree.title,pages,if tree.place.pinned() {", pinned"}else{""}),"current":current,
        "badge":tree.place.pinned().then_some("Pinned"),"page":super::graph::short_name(forest,tree.last_visit),
        "waiting":tree.pending_count,"waitingTip":format!("{} waiting",tree.pending_count),"meta":format!("{pages} · {}",sites_line(&sites,2)),
        "mini":mini_value(forest,&tree_view::miniature(forest,tree,Fit::CARD)),"menu":menu(tree)})
}
pub(super) fn shelf_card(forest: &ForestProjection, tree: &TreeSummary) -> Value {
    let mut value = card(forest, tree, false);
    value["page"] = Value::Null;
    value["meta"] = json!(tree.last_event_at.map_or_else(
        || plural(tree.page_count, "page"),
        |at| format!("{} · {}", plural(tree.page_count, "page"), ago(at))
    ));
    value
}

pub(super) struct Site {
    pub host: String,
    pub sample: url::Url,
    pages: usize,
}
pub(super) fn sites(forest: &ForestProjection, tree: &TreeSummary) -> Vec<Site> {
    let mut sites: std::collections::BTreeMap<String, Site> = Default::default();
    let mut stack = vec![tree.id.root()];
    while let Some(id) = stack.pop() {
        let v = forest.visit(id).unwrap();
        let host = v.url.host_str().unwrap_or(v.url.scheme()).to_lowercase();
        let site = sites.entry(host.clone()).or_insert_with(|| Site {
            host,
            sample: v.url.clone(),
            pages: 0,
        });
        site.pages += 1;
        stack.extend(forest.children(id).iter().rev());
    }
    let mut sites: Vec<_> = sites.into_values().collect();
    sites.sort_by(|a, b| b.pages.cmp(&a.pages).then(a.host.cmp(&b.host)));
    sites
}
pub(super) fn sites_line(sites: &[Site], names: usize) -> String {
    let mut line = sites
        .iter()
        .take(names)
        .map(|s| s.host.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if sites.len() > names {
        line.push_str(&format!(" +{}", sites.len() - names));
    }
    line
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
    #[test]
    fn menus_per_place_and_activation_are_separate() {
        for (place, expected) in [
            (
                Place::Tab { pinned: false },
                vec![
                    "Pin tab",
                    "Save for later",
                    "Rename",
                    "Settle tree",
                    "Delete tree",
                ],
            ),
            (
                Place::Tab { pinned: true },
                vec![
                    "Unpin tab",
                    "Save for later",
                    "Rename",
                    "Settle tree",
                    "Delete tree",
                ],
            ),
            (
                Place::Shelf { saved_at: 1 },
                vec!["Open", "Rename", "Settle tree", "Delete tree"],
            ),
            (
                Place::History { settled_at: 1 },
                vec!["Reopen", "Save for later", "Rename", "Delete"],
            ),
        ] {
            assert_eq!(
                acts(place)
                    .iter()
                    .map(|a| a.label(place))
                    .collect::<Vec<_>>(),
                expected
            );
            let tree = TreeSummary {
                id: crate::browser::TreeId(ulid::Ulid::new().to_string().parse().unwrap()),
                title: "Tree".into(),
                root_host: "example.org".into(),
                page_count: 1,
                pending_count: 0,
                last_visit: ulid::Ulid::new().to_string().parse().unwrap(),
                last_event_at: None,
                place,
            };
            assert!(matches!(
                intent(&tree, Act::Open),
                Some(ChromeIntent::SelectTree(_))
            ));
            assert_eq!(
                intent(&tree, Act::Delete),
                Some(if matches!(place, Place::History { .. }) {
                    ChromeIntent::DeleteTrees(vec![tree.id])
                } else {
                    ChromeIntent::DeleteBranch(tree.id.root())
                })
            );
        }
    }
}
