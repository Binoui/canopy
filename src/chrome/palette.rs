//! The palette's results and what running one asks for. The HTML view draws the rows.

use url::Url;

use super::{ChromeIntent, PaletteScope};
use crate::{
    address::{destination, web_search, Destination},
    browser::{NavigationTarget, SearchHit, UsualPath, VisitId, VisitState},
    forest::ForestProjection,
};

const GROUP_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaletteItem {
    Page(VisitId),
    Saved {
        capture: i64,
        title: String,
        excerpt: String,
    },
    Go(Url),
    Shortcut(Url),
    Path(UsualPath),
    WebSearch {
        url: Url,
        query: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Group {
    Shortcuts,
    Paths,
    Pending,
    Forest,
    Saved,
    Elsewhere,
}

impl Group {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Group::Shortcuts => "Frequent pages",
            Group::Paths => "Usual paths",
            Group::Pending => "Pending in this tree",
            Group::Forest => "In your forest",
            Group::Saved => "Saved text",
            Group::Elsewhere => "Elsewhere",
        }
    }
}

/// The palette's results for `query`, in display order, and the item Enter runs first.
/// Pages and saved text stay distinct: a saved-text hit opens the local reader.
pub(crate) fn results(
    query: &str,
    hits: Vec<SearchHit>,
    forest: &ForestProjection,
) -> (Vec<(Group, PaletteItem)>, usize) {
    let query = query.trim();
    let mut items = Vec::new();
    if query.is_empty() {
        let tree = forest.current().and_then(|id| forest.tree_of(id));
        let mut stack: Vec<_> = tree.map(|tree| tree.root()).into_iter().collect();
        while let Some(id) = stack.pop() {
            if forest
                .visit(id)
                .is_some_and(|v| v.state == VisitState::Queued)
            {
                items.push((Group::Pending, PaletteItem::Page(id)));
            }
            stack.extend(forest.children(id).iter().rev());
        }
        items.truncate(GROUP_LIMIT);
        return (items, 0);
    }
    let (pages, saved): (Vec<_>, Vec<_>) = hits.into_iter().partition(|h| h.capture_id.is_none());
    items.extend(
        pages
            .into_iter()
            .filter(|hit| forest.visit(hit.visit).is_some())
            .take(GROUP_LIMIT)
            .map(|hit| (Group::Forest, PaletteItem::Page(hit.visit))),
    );
    items.extend(saved.into_iter().take(GROUP_LIMIT).map(|hit| {
        (
            Group::Saved,
            PaletteItem::Saved {
                capture: hit.capture_id.unwrap(),
                title: hit.title,
                excerpt: hit.excerpt,
            },
        )
    }));
    let mut first = 0;
    match destination(query) {
        Ok(Destination::Address(url)) => {
            first = items.len();
            items.push((Group::Elsewhere, PaletteItem::Go(url)));
            items.push((
                Group::Elsewhere,
                PaletteItem::WebSearch {
                    url: web_search(query),
                    query: query.to_owned(),
                },
            ));
        }
        Ok(Destination::Search(url)) => items.push((
            Group::Elsewhere,
            PaletteItem::WebSearch {
                url,
                query: query.to_owned(),
            },
        )),
        Err(_) => {}
    }
    (items, first)
}

/// What running `item` asks for. Alt+Enter plants a new tree instead of a branch.
pub(crate) fn intent(
    item: &PaletteItem,
    scope: PaletteScope,
    new_tree: bool,
    forest: &ForestProjection,
) -> Option<ChromeIntent> {
    let target = match (new_tree, scope) {
        (true, _) | (_, PaletteScope::NewTree) => NavigationTarget::Root,
        (false, PaletteScope::From(parent)) => NavigationTarget::ChildOf(parent),
    };
    Some(match item {
        PaletteItem::Path(path) => ChromeIntent::OpenPath(path.clone()),
        PaletteItem::Shortcut(url) => ChromeIntent::Navigate {
            url: url.clone(),
            target: NavigationTarget::Root,
        },
        PaletteItem::Page(visit) if new_tree => ChromeIntent::Navigate {
            url: forest.visit(*visit)?.url.clone(),
            target: NavigationTarget::Root,
        },
        PaletteItem::Page(visit) => ChromeIntent::Restore(*visit),
        PaletteItem::Saved { capture, .. } => ChromeIntent::ReadCapture(*capture),
        PaletteItem::Go(url) | PaletteItem::WebSearch { url, .. } => ChromeIntent::Navigate {
            url: url.clone(),
            target,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, TreeId, TreeState, Visit};

    fn forest() -> (ForestProjection, VisitId, VisitId) {
        let id = || -> VisitId { ulid::Ulid::new().to_string().parse().unwrap() };
        let (root, queued) = (id(), id());
        let visit = |id, parent, state: VisitState, at| Visit {
            id,
            parent,
            url: Url::parse("https://example.org/page").unwrap(),
            title: "Example".into(),
            created_at: at,
            opened_at: (state == VisitState::Opened).then_some(at),
            state,
            source: None,
        };
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![
                    visit(root, None, VisitState::Opened, 1),
                    visit(queued, Some(root), VisitState::Queued, 2),
                ],
                current: Some(root),
            },
            vec![TreeState {
                root: TreeId(root),
                last_visit: root,
            }],
        )
        .unwrap();
        (forest, root, queued)
    }

    fn hit(visit: VisitId, capture: Option<i64>) -> SearchHit {
        SearchHit {
            visit,
            capture_id: capture,
            title: "Saved".into(),
            excerpt: "amber otter".into(),
            captured_at: capture,
            source: None,
            state: VisitState::Opened,
        }
    }

    #[test]
    fn pages_saved_text_then_address_then_web_search() {
        let (forest, root, _) = forest();
        let (items, first) = results(
            "example.org",
            vec![hit(root, Some(7)), hit(root, None)],
            &forest,
        );
        let kinds: Vec<_> = items.iter().map(|(group, _)| *group).collect();
        assert_eq!(
            kinds,
            [
                Group::Forest,
                Group::Saved,
                Group::Elsewhere,
                Group::Elsewhere
            ]
        );
        assert_eq!(items[0].1, PaletteItem::Page(root));
        assert!(matches!(items[1].1, PaletteItem::Saved { capture: 7, .. }));
        assert_eq!(
            items[2].1,
            PaletteItem::Go(Url::parse("https://example.org/").unwrap())
        );
        assert!(matches!(items[3].1, PaletteItem::WebSearch { .. }));
        assert_eq!(first, 2, "Enter goes to a typed address");
    }

    #[test]
    fn words_only_offer_a_web_search_and_select_the_first_hit() {
        let (forest, root, _) = forest();
        let (items, first) = results("otter", vec![hit(root, None)], &forest);
        assert_eq!(items.len(), 2);
        assert_eq!(first, 0);
        let (items, _) = results("javascript:alert(1)", vec![], &forest);
        assert!(items.is_empty());
    }

    #[test]
    fn an_empty_query_lists_pending_pages_of_the_current_tree() {
        let (forest, _, queued) = forest();
        let (items, _) = results("  ", vec![], &forest);
        assert_eq!(items, [(Group::Pending, PaletteItem::Page(queued))]);
    }

    #[test]
    fn running_an_item_respects_scope_and_alt_enter() {
        let (forest, root, _) = forest();
        let go = PaletteItem::Go(Url::parse("https://example.org/next").unwrap());
        let branch = intent(&go, PaletteScope::From(root), false, &forest).unwrap();
        assert!(matches!(
            branch,
            ChromeIntent::Navigate { target: NavigationTarget::ChildOf(p), .. } if p == root
        ));
        let planted = intent(&go, PaletteScope::From(root), true, &forest).unwrap();
        assert!(matches!(
            planted,
            ChromeIntent::Navigate {
                target: NavigationTarget::Root,
                ..
            }
        ));
        let saved = PaletteItem::Saved {
            capture: 3,
            title: String::new(),
            excerpt: String::new(),
        };
        assert!(matches!(
            intent(&saved, PaletteScope::NewTree, false, &forest),
            Some(ChromeIntent::ReadCapture(3))
        ));
        assert!(matches!(
            intent(&PaletteItem::Page(root), PaletteScope::NewTree, false, &forest),
            Some(ChromeIntent::Restore(id)) if id == root
        ));
    }
}
