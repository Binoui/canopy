//! The palette's results and what running one asks for. The HTML view draws the rows.

use url::Url;

use super::{ChromeIntent, PaletteScope};
use crate::{
    address::{destination, web_search, Destination},
    browser::{Bookmark, ImportedPage, NavigationTarget, SearchHit, VisitId, VisitState},
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
    Suggestion(crate::suggest::Suggestion),
    /// A page from imported browser history.
    Imported(ImportedPage),
    Bookmark(Bookmark),
    WebSearch {
        url: Url,
        query: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Group {
    Suggested,
    Pending,
    Forest,
    Bookmarks,
    Saved,
    Imported,
    Elsewhere,
}

impl Group {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Group::Suggested => "Suggested",
            Group::Pending => "Pending in this tree",
            Group::Forest => "In your forest",
            Group::Bookmarks => "Bookmarks",
            Group::Saved => "Saved text",
            Group::Imported => "From Chrome",
            Group::Elsewhere => "Elsewhere",
        }
    }
}

/// The palette's results for `query`, in display order, and the item Enter runs first.
/// Pages and saved text stay distinct: a saved-text hit opens the local reader. Imported
/// pages already in the forest are left out, so the forest's own visit wins.
pub(crate) fn results(
    query: &str,
    hits: Vec<SearchHit>,
    imported: Vec<ImportedPage>,
    bookmarks: &[Bookmark],
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
    let shown: std::collections::HashSet<&Url> = items
        .iter()
        .filter_map(|(_, item)| match item {
            PaletteItem::Page(id) => forest.visit(*id).map(|visit| &visit.url),
            _ => None,
        })
        .collect();
    let needle = query.to_lowercase();
    let matched: Vec<_> = bookmarks
        .iter()
        .filter(|b| {
            b.title.to_lowercase().contains(&needle)
                || b.url.as_str().to_lowercase().contains(&needle)
        })
        .take(GROUP_LIMIT)
        .collect();
    let history: Vec<_> = imported
        .into_iter()
        .filter(|p| !shown.contains(&p.url) && !matched.iter().any(|b| b.url == p.url))
        .collect();
    items.extend(
        matched
            .into_iter()
            .map(|b| (Group::Bookmarks, PaletteItem::Bookmark(b.clone()))),
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
    items.extend(
        history
            .into_iter()
            .take(GROUP_LIMIT)
            .map(|page| (Group::Imported, PaletteItem::Imported(page))),
    );
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
        PaletteItem::Suggestion(s) => super::start::open(s),
        PaletteItem::Bookmark(b) => ChromeIntent::Navigate {
            url: b.url.clone(),
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
        PaletteItem::Imported(page) => ChromeIntent::Navigate {
            url: page.url.clone(),
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
                pin: None,
                last_event_at: None,
                lifecycle: crate::browser::Lifecycle::Open,
                root: TreeId(root),
                last_visit: root,
                name: None,
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

    fn browser_bookmark_fixture() -> Vec<Bookmark> {
        let dir = tempfile::tempdir().unwrap();
        let mut b = crate::browser::Browser::open(&dir.path().join("db")).unwrap();
        b.import_pages(
            "test",
            &[],
            &[crate::browser::ImportedBookmark {
                url: Url::parse("https://example.org/bookmarked").unwrap(),
                title: "Example".into(),
                folder: "Bookmarks bar".into(),
                visits: 1,
            }],
        )
        .unwrap();
        b.bookmarks().unwrap()
    }

    #[test]
    fn pages_saved_text_then_address_then_web_search() {
        let (forest, root, _) = forest();
        let imported = |url: &str| ImportedPage {
            url: Url::parse(url).unwrap(),
            title: "Imported".into(),
            visits: 3,
            typed: 0,
            last_visit: None,
        };
        let (items, first) = results(
            "example.org",
            vec![hit(root, Some(7)), hit(root, None)],
            vec![
                imported("https://example.org/page"),
                imported("https://example.org/visited"),
                imported("https://example.org/bookmarked"),
            ],
            &browser_bookmark_fixture(),
            &forest,
        );
        let kinds: Vec<_> = items.iter().map(|(group, _)| *group).collect();
        assert_eq!(
            kinds,
            [
                Group::Forest,
                Group::Bookmarks,
                Group::Saved,
                Group::Imported,
                Group::Elsewhere,
                Group::Elsewhere
            ],
            "an imported page already in the forest is left out"
        );
        assert_eq!(items[0].1, PaletteItem::Page(root));
        assert!(
            matches!(&items[1].1, PaletteItem::Bookmark(page) if page.url.path() == "/bookmarked")
        );
        assert_eq!(
            intent(&items[1].1, PaletteScope::From(root), false, &forest),
            Some(ChromeIntent::Navigate {
                url: Url::parse("https://example.org/bookmarked").unwrap(),
                target: NavigationTarget::Root
            })
        );
        assert!(matches!(items[2].1, PaletteItem::Saved { capture: 7, .. }));
        assert_eq!(
            items[4].1,
            PaletteItem::Go(Url::parse("https://example.org/").unwrap())
        );
        assert!(matches!(items[5].1, PaletteItem::WebSearch { .. }));
        assert_eq!(first, 4, "Enter goes to a typed address");
        let PaletteItem::Imported(page) = &items[3].1 else {
            panic!("an imported page")
        };
        assert_eq!(
            intent(&items[3].1, PaletteScope::From(root), false, &forest),
            Some(ChromeIntent::Navigate {
                url: page.url.clone(),
                target: NavigationTarget::ChildOf(root),
            }),
            "an imported page opens like a typed address"
        );
    }

    #[test]
    fn words_only_offer_a_web_search_and_select_the_first_hit() {
        let (forest, root, _) = forest();
        let (items, first) = results("otter", vec![hit(root, None)], vec![], &[], &forest);
        assert_eq!(items.len(), 2);
        assert_eq!(first, 0);
        let (items, _) = results("javascript:alert(1)", vec![], vec![], &[], &forest);
        assert!(items.is_empty());
    }

    #[test]
    fn an_empty_query_lists_pending_pages_of_the_current_tree() {
        let (forest, _, queued) = forest();
        let (items, _) = results("  ", vec![], vec![], &[], &forest);
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
