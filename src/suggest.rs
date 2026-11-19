use crate::{
    address::web_url,
    browser::{Arrival, ArrivalKind, Bookmark, ImportedPage, TreeId},
    forest::ForestProjection,
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
};
use url::Url;

pub const DAY: i64 = 86_400_000;
pub const WINDOW: i64 = 30 * DAY;

#[derive(Clone, Debug, PartialEq)]
pub enum Suggestion {
    Page {
        url: Url,
        title: String,
        why: PageWhy,
    },
    Tree {
        tree: TreeId,
        why: TreeWhy,
    },
    Path {
        first: Url,
        second: Url,
        times: usize,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageWhy {
    ThisWeek(usize),
    ThisMonth(usize),
    OftenInChrome,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TreeWhy {
    Waiting(usize),
    DaysInARow(usize),
    Yesterday,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SuggestionKey {
    Page(Url),
    Tree(TreeId),
    Path { from: String, to: String },
}

impl Suggestion {
    pub fn key(&self) -> SuggestionKey {
        match self {
            Self::Page { url, .. } => SuggestionKey::Page(url.clone()),
            Self::Tree { tree, .. } => SuggestionKey::Tree(*tree),
            Self::Path { first, second, .. } => SuggestionKey::Path {
                from: site(first).into(),
                to: site(second).into(),
            },
        }
    }
    pub fn why(&self) -> String {
        match self {
            Self::Page {
                why: PageWhy::ThisWeek(n),
                ..
            } => format!("{n} times this week"),
            Self::Page {
                why: PageWhy::ThisMonth(n),
                ..
            } => format!("{n} times this month"),
            Self::Page {
                why: PageWhy::OftenInChrome,
                ..
            } => "Often in Chrome".into(),
            Self::Tree {
                why: TreeWhy::Waiting(n),
                ..
            } => format!(
                "{n} {} still waiting",
                if *n == 1 { "page" } else { "pages" }
            ),
            Self::Tree {
                why: TreeWhy::DaysInARow(n),
                ..
            } => format!("Opened {n} days in a row"),
            Self::Tree {
                why: TreeWhy::Yesterday,
                ..
            } => "Last opened yesterday".into(),
            Self::Path { times, .. } => format!("{times} times"),
        }
    }
}
impl fmt::Display for SuggestionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Page(url) => write!(f, "page {url}"),
            Self::Tree(tree) => write!(f, "tree {}", tree.root()),
            Self::Path { from, to } => write!(f, "path {from} {to}"),
        }
    }
}
impl FromStr for SuggestionKey {
    type Err = anyhow::Error;
    fn from_str(text: &str) -> anyhow::Result<Self> {
        let (kind, rest) = text
            .split_once(' ')
            .ok_or_else(|| anyhow::anyhow!("Missing suggestion kind."))?;
        Ok(match kind {
            "page" => Self::Page(web_url(rest)?),
            "tree" => Self::Tree(TreeId(rest.parse()?)),
            "path" => {
                let (from, to) = rest
                    .split_once(' ')
                    .ok_or_else(|| anyhow::anyhow!("Missing path site."))?;
                anyhow::ensure!(
                    !from.is_empty() && !to.is_empty() && !to.contains(char::is_whitespace),
                    "Invalid path sites."
                );
                Self::Path {
                    from: from.into(),
                    to: to.into(),
                }
            }
            _ => anyhow::bail!("Unknown suggestion kind."),
        })
    }
}
pub fn site(url: &Url) -> &str {
    let host = url.host_str().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host)
}

pub struct Inputs<'a> {
    pub forest: &'a ForestProjection,
    pub bookmarks: &'a [Bookmark],
    pub declined: &'a HashSet<SuggestionKey>,
    pub arrivals: &'a [Arrival],
    pub chrome: &'a [ImportedPage],
    pub left: Option<TreeId>,
}

pub fn admits(
    forest: &ForestProjection,
    bookmarks: &[Bookmark],
    declined: &HashSet<SuggestionKey>,
    s: &Suggestion,
) -> bool {
    if declined.contains(&s.key()) {
        return false;
    }
    match s {
        Suggestion::Page { url, .. } | Suggestion::Path { first: url, .. } => {
            !bookmarks.iter().any(|b| b.url == *url)
        }
        Suggestion::Tree { tree, .. } => forest.trees().iter().any(|t| {
            t.id == *tree
                && matches!(t.place, crate::browser::Place::History { .. })
                && forest.current().and_then(|v| forest.tree_of(v)) != Some(*tree)
        }),
    }
}

pub fn rank(input: &Inputs<'_>, now: i64) -> Vec<Suggestion> {
    let events: Vec<_> = input
        .arrivals
        .iter()
        .filter(|a| a.at >= now - WINDOW && a.at <= now)
        .collect();
    let weight = |at| 0.5_f64.powf((now - at) as f64 / (7 * DAY) as f64);
    let mut trees: HashMap<TreeId, Vec<&Arrival>> = HashMap::new();
    let mut pages: HashMap<Url, Vec<&Arrival>> = HashMap::new();
    let mut paths: HashMap<(String, String), Vec<(&Arrival, Url)>> = HashMap::new();
    for a in &events {
        trees.entry(a.tree).or_default().push(a);
        pages.entry(a.url.clone()).or_default().push(a);
        if a.kind == ArrivalKind::New && a.from.is_some() && a.from == a.parent {
            if let Some(first) = a.from.and_then(|v| input.forest.visit(v)) {
                if site(&first.url) != site(&a.url) {
                    paths
                        .entry((site(&first.url).into(), site(&a.url).into()))
                        .or_default()
                        .push((a, first.url.clone()));
                }
            }
        }
    }
    let mut queues: [Vec<(f64, Suggestion)>; 3] = std::array::from_fn(|_| Vec::new());
    for (url, arrivals) in pages {
        if arrivals.len() < 2 {
            continue;
        }
        let newest = arrivals
            .iter()
            .max_by_key(|a| (a.at, a.visit.to_string()))
            .unwrap();
        let week = arrivals.iter().filter(|a| a.at >= now - 7 * DAY).count();
        queues[2].push((
            arrivals.iter().map(|a| weight(a.at)).sum(),
            Suggestion::Page {
                url,
                title: newest.title.clone(),
                why: if week >= 2 {
                    PageWhy::ThisWeek(week)
                } else {
                    PageWhy::ThisMonth(arrivals.len())
                },
            },
        ));
    }
    for arrivals in paths.into_values() {
        if arrivals.len() < 2 {
            continue;
        }
        let (last, first) = arrivals
            .iter()
            .max_by_key(|(a, _)| (a.at, a.visit.to_string()))
            .unwrap();
        queues[1].push((
            arrivals.iter().map(|(a, _)| weight(a.at)).sum(),
            Suggestion::Path {
                first: first.clone(),
                second: last.url.clone(),
                times: arrivals.len(),
            },
        ));
    }
    for tree in input.forest.history() {
        if Some(tree.id) == input.left {
            continue;
        }
        let arrivals = trees.get(&tree.id).map(Vec::as_slice).unwrap_or(&[]);
        let days: HashSet<_> = arrivals.iter().map(|a| (now - a.at) / DAY).collect();
        let start = if days.contains(&0) { 0 } else { 1 };
        let streak = (start..).take_while(|d| days.contains(d)).count();
        let why = if tree.pending_count > 0 {
            TreeWhy::Waiting(tree.pending_count)
        } else if streak >= 2 {
            TreeWhy::DaysInARow(streak)
        } else if days.contains(&1) {
            TreeWhy::Yesterday
        } else {
            continue;
        };
        let score = arrivals
            .iter()
            .filter(|a| a.from.and_then(|v| input.forest.tree_of(v)) != Some(tree.id))
            .map(|a| weight(a.at))
            .sum();
        queues[0].push((score, Suggestion::Tree { tree: tree.id, why }));
    }
    let admitted = |s: &Suggestion| admits(input.forest, input.bookmarks, input.declined, s);
    for (i, queue) in queues.iter_mut().enumerate() {
        queue.retain(|(_, s)| admitted(s));
        queue.sort_by(|a, b| {
            let waiting = |s: &Suggestion| {
                matches!(
                    s,
                    Suggestion::Tree {
                        why: TreeWhy::Waiting(_),
                        ..
                    }
                )
            };
            (if i == 0 {
                waiting(&b.1).cmp(&waiting(&a.1))
            } else {
                std::cmp::Ordering::Equal
            })
            .then_with(|| b.0.total_cmp(&a.0))
            .then_with(|| a.1.key().to_string().cmp(&b.1.key().to_string()))
        });
    }
    let mut result = Vec::new();
    while result.len() < 3 {
        let best = (0..3).filter(|i| !queues[*i].is_empty()).max_by(|a, b| {
            queues[*a][0]
                .0
                .total_cmp(&queues[*b][0].0)
                .then_with(|| b.cmp(a))
        });
        let Some(best) = best else { break };
        result.push(queues[best].remove(0).1);
    }
    let mut sites: HashSet<String> = result
        .iter()
        .filter_map(|s| {
            if let Suggestion::Page { url, .. } = s {
                Some(site(url).into())
            } else {
                None
            }
        })
        .collect();
    for page in input.chrome {
        if result.len() == 3 {
            break;
        }
        let s = Suggestion::Page {
            url: page.url.clone(),
            title: page.title.clone(),
            why: PageWhy::OftenInChrome,
        };
        if page.visits > 0 && admitted(&s) && sites.insert(site(&page.url).into()) {
            result.push(s);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Browser, Snapshot, SourceContext};

    fn browser() -> (tempfile::TempDir, Browser) {
        let d = tempfile::tempdir().unwrap();
        let b = Browser::open(&d.path().join("forest.sqlite3")).unwrap();
        (d, b)
    }
    const NOW: i64 = 100 * DAY;
    fn url(host: &str) -> Url {
        web_url(&format!("https://{host}/")).unwrap()
    }
    fn forest() -> ForestProjection {
        ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![],
                current: None,
            },
            vec![],
        )
        .unwrap()
    }
    fn arrival(host: &str, at: i64) -> Arrival {
        let visit = ulid::Ulid::new().to_string().parse().unwrap();
        Arrival {
            visit,
            tree: TreeId(visit),
            url: url(host),
            title: host.into(),
            from: None,
            parent: None,
            kind: ArrivalKind::Return,
            at,
        }
    }
    fn ranked(
        events: &[Arrival],
        chrome: &[ImportedPage],
        bookmarks: &[Bookmark],
        declined: &HashSet<SuggestionKey>,
    ) -> Vec<Suggestion> {
        rank(
            &Inputs {
                forest: &forest(),
                bookmarks,
                declined,
                arrivals: events,
                chrome,
                left: None,
            },
            NOW,
        )
    }
    #[test]
    fn recent_arrivals_outrank_old_ones_and_reasons_use_rolling_windows() {
        let events = [
            arrival("old.test", NOW - 20 * DAY),
            arrival("old.test", NOW - 21 * DAY),
            arrival("recent.test", NOW),
            arrival("recent.test", NOW - DAY),
        ];
        let result = ranked(&events, &[], &[], &HashSet::new());
        assert_eq!(result[0].key(), SuggestionKey::Page(url("recent.test")));
        assert_eq!(result[0].why(), "2 times this week");
        assert_eq!(result[1].why(), "2 times this month");
        assert_eq!(result, ranked(&events, &[], &[], &HashSet::new()));
    }
    #[test]
    fn chrome_fills_last_and_declined_keys_never_return() {
        let events = [arrival("own.test", NOW), arrival("own.test", NOW)];
        let chrome: Vec<_> = (0..12)
            .map(|i| ImportedPage {
                url: url(&format!("chrome{i}.test")),
                title: String::new(),
                visits: 100,
                typed: 10,
                last_visit: None,
            })
            .collect();
        let declined = chrome[..8]
            .iter()
            .map(|p| SuggestionKey::Page(p.url.clone()))
            .collect();
        let result = ranked(&events, &chrome, &[], &declined);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].key(), SuggestionKey::Page(url("own.test")));
        assert_eq!(result[1].why(), "Often in Chrome");
        let declined = result.iter().map(Suggestion::key).collect();
        assert!(ranked(&events, &[], &[], &declined).is_empty());
    }
    #[test]
    fn bookmarked_pages_and_path_roots_are_never_admitted() {
        let (_dir, mut b) = browser();
        b.bookmark(&url("chosen.test"), "Chosen").unwrap();
        let events = [arrival("chosen.test", NOW), arrival("chosen.test", NOW)];
        assert!(ranked(&events, &[], &b.bookmarks().unwrap(), &HashSet::new()).is_empty());
        let s = Suggestion::Path {
            first: url("chosen.test"),
            second: url("other.test"),
            times: 2,
        };
        assert!(!admits(
            &forest(),
            &b.bookmarks().unwrap(),
            &HashSet::new(),
            &s
        ));
    }
    #[test]
    fn keys_round_trip_and_path_identity_survives_page_changes() {
        let first = Suggestion::Path {
            first: web_url("https://www.a.test/old").unwrap(),
            second: url("b.test"),
            times: 2,
        };
        let second = Suggestion::Path {
            first: url("a.test"),
            second: web_url("https://b.test/new").unwrap(),
            times: 9,
        };
        assert_eq!(first.key(), second.key());
        for key in [
            first.key(),
            SuggestionKey::Page(url("a.test")),
            SuggestionKey::Tree(TreeId(ulid::Ulid::new().to_string().parse().unwrap())),
        ] {
            assert_eq!(key.to_string().parse::<SuggestionKey>().unwrap(), key);
        }
        for text in [
            "",
            "wrong abc",
            "page file:///etc/passwd",
            "tree x",
            "path a",
            "path a b c",
        ] {
            assert!(text.parse::<SuggestionKey>().is_err());
        }
    }
    #[test]
    fn same_site_pairs_never_make_a_path_and_cross_site_pairs_do() {
        let (_dir, mut b) = browser();
        let roots = b.add_roots(&[url("a.test")]).unwrap();
        let root = roots[0];
        let snapshot = b.snapshot().unwrap();
        let f = ForestProjection::from_snapshot(snapshot, b.tree_states().unwrap()).unwrap();
        let mut a = arrival("a.test", NOW);
        a.kind = ArrivalKind::New;
        a.from = Some(root);
        a.parent = Some(root);
        let mut c = a.clone();
        c.url = url("b.test");
        let input = |events: &[Arrival]| {
            rank(
                &Inputs {
                    forest: &f,
                    bookmarks: &[],
                    declined: &HashSet::new(),
                    arrivals: events,
                    chrome: &[],
                    left: None,
                },
                NOW,
            )
        };
        assert!(!input(&[a.clone(), a])
            .iter()
            .any(|s| matches!(s, Suggestion::Path { .. })));
        assert!(input(&[c.clone(), c])
            .iter()
            .any(|s| matches!(s, Suggestion::Path { .. })));
    }
    #[test]
    fn settled_waiting_trees_rank_first_and_open_pinned_and_left_trees_are_excluded() {
        let (_dir, mut b) = browser();
        let roots = b
            .add_roots(&[
                url("old.test"),
                url("waiting.test"),
                url("pinned.test"),
                url("open.test"),
                url("left.test"),
            ])
            .unwrap();
        let trees = b.tree_states().unwrap();
        for t in &trees[..3] {
            b.place_tree(t.root, crate::browser::Move::History).unwrap();
        }
        b.place_tree(trees[4].root, crate::browser::Move::History)
            .unwrap();
        b.place_tree(
            trees[2].root,
            crate::browser::Move::Shelf(crate::browser::Slot::End),
        )
        .unwrap();
        b.queue_link(
            roots[1],
            url("queued.test"),
            SourceContext {
                page_url: url("waiting.test"),
                citation_url: None,
                link_text: String::new(),
                excerpt: String::new(),
            },
        )
        .unwrap();
        let f = ForestProjection::from_snapshot(b.snapshot().unwrap(), b.tree_states().unwrap())
            .unwrap();
        let mut events = Vec::new();
        for (i, t) in trees.iter().enumerate() {
            let mut a = arrival(&format!("event{i}.test"), NOW - DAY);
            a.tree = t.root;
            events.push(a);
        }
        let result = rank(
            &Inputs {
                forest: &f,
                bookmarks: &[],
                declined: &HashSet::new(),
                arrivals: &events,
                chrome: &[],
                left: Some(trees[4].root),
            },
            NOW,
        );
        assert!(
            matches!(result[0],Suggestion::Tree { tree,why:TreeWhy::Waiting(1) } if tree==trees[1].root)
        );
        assert_eq!(result.len(), 2);
        assert!(matches!(
            result[1],
            Suggestion::Tree {
                why: TreeWhy::Yesterday,
                ..
            }
        ));
    }
    #[test]
    fn tree_reasons_prefer_waiting_then_consecutive_days_then_yesterday() {
        let (_dir, mut b) = browser();
        let roots = b.add_roots(&[url("streak.test")]).unwrap();
        let tree = b.tree_states().unwrap()[0].root;
        b.place_tree(tree, crate::browser::Move::History).unwrap();
        let f = ForestProjection::from_snapshot(b.snapshot().unwrap(), b.tree_states().unwrap())
            .unwrap();
        let mut events: Vec<_> = (1..=4)
            .map(|day| {
                let mut a = arrival("streak.test", NOW - day * DAY);
                a.tree = tree;
                a
            })
            .collect();
        let trees = |events: &[Arrival]| {
            rank(
                &Inputs {
                    forest: &f,
                    bookmarks: &[],
                    declined: &HashSet::new(),
                    arrivals: events,
                    chrome: &[],
                    left: None,
                },
                NOW,
            )
            .into_iter()
            .filter(|s| matches!(s, Suggestion::Tree { .. }))
            .collect::<Vec<_>>()
        };
        assert_eq!(trees(&events)[0].why(), "Opened 4 days in a row");
        events.truncate(1);
        assert_eq!(trees(&events)[0].why(), "Last opened yesterday");
        events[0].at = NOW - 31 * DAY;
        assert!(trees(&events).is_empty());
        assert_eq!(roots[0], tree.root());
    }
}
