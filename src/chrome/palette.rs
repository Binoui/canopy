//! The palette's results and what running one asks for. The HTML view draws the rows.

use url::Url;

use super::{view::short_host, ChromeIntent, PaletteScope};
use crate::{
    address::{destination, web_search, Destination},
    browser::{Bookmark, ImportedPage, NavigationTarget, SearchHit, VisitId},
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
    Web,
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
            Group::Pending => "Waiting in this tree",
            Group::Web => "On the web",
            Group::Forest => "In your forest",
            Group::Bookmarks => "Bookmarks",
            Group::Saved => "Saved text",
            Group::Imported => "From Chrome",
            Group::Elsewhere => "Elsewhere",
        }
    }
}

/// How well a bookmark answers the typed words, best first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Answer {
    /// The words are its title.
    Named,
    /// Its title or its site starts with the words.
    Starts,
    /// Its title or its address contains the words.
    Mentions,
}

fn answer(bookmark: &Bookmark, needle: &str) -> Option<Answer> {
    let title = bookmark.title.trim().to_lowercase();
    if title == needle {
        Some(Answer::Named)
    } else if title.starts_with(needle) || short_host(&bookmark.url).starts_with(needle) {
        Some(Answer::Starts)
    } else if title.contains(needle) || bookmark.url.as_str().to_lowercase().contains(needle) {
        Some(Answer::Mentions)
    } else {
        None
    }
}

/// The palette's results for `query`, in display order, and the item Enter runs first.
/// Pages and saved text stay distinct: a saved-text hit opens the local reader. Imported
/// pages already in the forest are left out, so the forest's own visit wins.
///
/// Enter runs a typed address, or else the first row. From a page, the palette finds what
/// the forest already holds, so its pages come first. A new tree starts from what was typed,
/// so the address or the web search comes first, and words that name a bookmark open it.
pub(crate) fn results(
    query: &str,
    hits: Vec<SearchHit>,
    imported: Vec<ImportedPage>,
    bookmarks: &[Bookmark],
    forest: &ForestProjection,
    scope: PaletteScope,
) -> (Vec<(Group, PaletteItem)>, usize) {
    let query = query.trim();
    let mut items = Vec::new();
    if query.is_empty() {
        let tree = forest.current().and_then(|id| forest.tree_of(id));
        let mut stack: Vec<_> = tree.map(|tree| tree.root()).into_iter().collect();
        while let Some(id) = stack.pop() {
            if forest.is_waiting(id) {
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
    let mut matched: Vec<_> = bookmarks
        .iter()
        .filter_map(|b| Some((answer(b, &needle)?, b)))
        .collect();
    matched.sort_by_key(|(answer, _)| *answer);
    matched.truncate(GROUP_LIMIT);
    // One word can be the start of a name. A few words that start a title are a search.
    let named = matched.first().is_some_and(|(answer, _)| match answer {
        Answer::Named => true,
        Answer::Starts => !query.contains(char::is_whitespace),
        Answer::Mentions => false,
    });
    let history: Vec<_> = imported
        .into_iter()
        .filter(|p| !shown.contains(&p.url) && !matched.iter().any(|(_, b)| b.url == p.url))
        .collect();
    items.extend(
        matched
            .into_iter()
            .map(|(_, b)| (Group::Bookmarks, PaletteItem::Bookmark(b.clone()))),
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
    let (order, typed) = match scope {
        PaletteScope::NewTree => (
            [
                Group::Web,
                Group::Bookmarks,
                Group::Forest,
                Group::Saved,
                Group::Imported,
            ],
            Group::Web,
        ),
        PaletteScope::From(_) => (
            [
                Group::Forest,
                Group::Bookmarks,
                Group::Saved,
                Group::Imported,
                Group::Elsewhere,
            ],
            Group::Elsewhere,
        ),
    };
    let enter = match destination(query) {
        Ok(Destination::Address(url)) => {
            items.push((typed, PaletteItem::Go(url)));
            items.push((
                typed,
                PaletteItem::WebSearch {
                    url: web_search(query),
                    query: query.to_owned(),
                },
            ));
            Some(typed)
        }
        Ok(Destination::Search(url)) => {
            items.push((
                typed,
                PaletteItem::WebSearch {
                    url,
                    query: query.to_owned(),
                },
            ));
            (named && scope == PaletteScope::NewTree).then_some(Group::Bookmarks)
        }
        Err(_) => None,
    };
    items.sort_by_key(|(group, _)| order.iter().position(|listed| listed == group));
    let first = enter
        .and_then(|group| items.iter().position(|(listed, _)| *listed == group))
        .unwrap_or(0);
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
    use crate::browser::{Snapshot, TreeId, TreeState, Visit, VisitState};

    fn forest() -> (ForestProjection, VisitId, VisitId) {
        let id = || -> VisitId { ulid::Ulid::new().to_string().parse().unwrap() };
        let (root, queued) = (id(), id());
        let visit = |id, parent, state: VisitState, at| Visit {
            dismissed_at: None,
            branch_settled_at: None,
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
                last_event_at: None,
                spot: crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0,
                },
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

    /// Bookmarks titled and addressed as given, in that order.
    fn bookmarks(pages: &[(&str, &str)]) -> Vec<Bookmark> {
        let dir = tempfile::tempdir().unwrap();
        let mut b = crate::browser::Browser::open(&dir.path().join("db")).unwrap();
        for (title, url) in pages {
            b.bookmark(&Url::parse(url).unwrap(), title).unwrap();
        }
        b.bookmarks().unwrap()
    }

    fn imported(url: &str) -> ImportedPage {
        ImportedPage {
            url: Url::parse(url).unwrap(),
            title: "Imported".into(),
            visits: 3,
            typed: 0,
            last_visit: None,
        }
    }

    /// The row Enter runs for `query` in a new tree, and every row's group.
    fn new_tree(query: &str, bookmarks: &[Bookmark]) -> (PaletteItem, Vec<Group>) {
        let (forest, root, _) = forest();
        let (items, first) = results(
            query,
            vec![hit(root, Some(7)), hit(root, None)],
            vec![imported("https://imported.example/")],
            bookmarks,
            &forest,
            PaletteScope::NewTree,
        );
        let groups = items.iter().map(|(group, _)| *group).collect();
        (items[first].1.clone(), groups)
    }

    #[test]
    fn pages_saved_text_then_address_then_web_search() {
        let (forest, root, _) = forest();
        let (items, first) = results(
            "example.org",
            vec![hit(root, Some(7)), hit(root, None)],
            vec![
                imported("https://example.org/page"),
                imported("https://example.org/visited"),
                imported("https://example.org/bookmarked"),
            ],
            &bookmarks(&[("Example", "https://example.org/bookmarked")]),
            &forest,
            PaletteScope::From(root),
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
        let from = PaletteScope::From(root);
        let (items, first) = results("otter", vec![hit(root, None)], vec![], &[], &forest, from);
        assert_eq!(items.len(), 2);
        assert_eq!(first, 0);
        let (items, _) = results("javascript:alert(1)", vec![], vec![], &[], &forest, from);
        assert!(items.is_empty());
    }

    #[test]
    fn a_new_tree_lists_what_was_typed_first_and_enter_searches_the_web_for_words() {
        let mentions = bookmarks(&[("A page about the amber otter", "https://b.example/")]);
        let (enter, groups) = new_tree("amber otter", &mentions);
        assert_eq!(
            groups,
            [
                Group::Web,
                Group::Bookmarks,
                Group::Forest,
                Group::Saved,
                Group::Imported
            ]
        );
        assert!(
            matches!(enter, PaletteItem::WebSearch { query, .. } if query == "amber otter"),
            "pages and bookmarks that mention the words do not take Enter"
        );
        let elsewhere = bookmarks(&[("example.org", "https://elsewhere.example/")]);
        let (enter, groups) = new_tree("example.org", &elsewhere);
        assert_eq!(groups[..3], [Group::Web, Group::Web, Group::Bookmarks]);
        assert_eq!(
            enter,
            PaletteItem::Go(Url::parse("https://example.org/").unwrap()),
            "a typed address goes there, whatever a bookmark is called"
        );
    }

    #[test]
    fn a_new_tree_opens_the_bookmark_the_words_name() {
        let saved = bookmarks(&[
            ("A list of assets", "https://assets.example/reddit"),
            ("All of it", "https://www.reddit.example/r/all"),
            ("reddit", "https://elsewhere.example/"),
        ]);
        let title = |query: &str| match new_tree(query, &saved).0 {
            PaletteItem::Bookmark(b) => b.title,
            PaletteItem::WebSearch { .. } => "a web search".into(),
            other => panic!("Enter runs {other:?}"),
        };
        assert_eq!(title("Reddit"), "reddit", "a title outranks a site");
        assert_eq!(
            title("redd"),
            "All of it",
            "one word starts a site, and that outranks an earlier mention"
        );
        assert_eq!(title("a list"), "a web search", "a few words are a search");
        assert_eq!(title("a list of assets"), "A list of assets");
        assert_eq!(title("list"), "a web search", "a mention is not a name");
        let (items, first) = results(
            "reddit",
            vec![],
            vec![],
            &saved,
            &forest().0,
            PaletteScope::NewTree,
        );
        let rows: Vec<_> = items
            .iter()
            .map(|(_, item)| match item {
                PaletteItem::Bookmark(b) => b.title.as_str(),
                PaletteItem::WebSearch { .. } => "a web search",
                other => panic!("an unexpected row {other:?}"),
            })
            .collect();
        assert_eq!(
            rows,
            ["a web search", "reddit", "All of it", "A list of assets"]
        );
        assert_eq!(rows[first], "reddit");
    }

    #[test]
    fn the_bookmark_the_words_name_survives_the_cap_on_listed_bookmarks() {
        let mut pages: Vec<_> = (0..GROUP_LIMIT + 1)
            .map(|i| {
                (
                    "Notes on reddit".to_owned(),
                    format!("https://notes.example/{i}"),
                )
            })
            .collect();
        pages.push(("reddit".to_owned(), "https://elsewhere.example/".to_owned()));
        let pages: Vec<_> = pages
            .iter()
            .map(|(t, u)| (t.as_str(), u.as_str()))
            .collect();
        let (items, first) = results(
            "reddit",
            vec![],
            vec![],
            &bookmarks(&pages),
            &forest().0,
            PaletteScope::NewTree,
        );
        let listed = items
            .iter()
            .filter(|(group, _)| *group == Group::Bookmarks)
            .count();
        assert_eq!(listed, GROUP_LIMIT);
        assert!(matches!(&items[first].1, PaletteItem::Bookmark(b) if b.title == "reddit"));
    }

    #[test]
    fn an_empty_query_lists_pending_pages_of_the_current_tree() {
        let (forest, _, queued) = forest();
        for scope in [PaletteScope::From(queued), PaletteScope::NewTree] {
            let (items, _) = results("  ", vec![], vec![], &[], &forest, scope);
            assert_eq!(items, [(Group::Pending, PaletteItem::Page(queued))]);
        }
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
