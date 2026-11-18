//! Waiting state and the current tree's ordered waiting pages.

use std::collections::HashSet;

use crate::{
    browser::{Visit, VisitId, VisitState},
    forest::ForestProjection,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WaitState {
    NotLoaded,
    Loaded,
}

impl WaitState {
    pub(crate) fn of(visit: &Visit, loaded: &HashSet<VisitId>) -> Option<Self> {
        if visit.dismissed_at.is_some() {
            return None;
        }
        match visit.state {
            VisitState::Opened => None,
            VisitState::Queued if loaded.contains(&visit.id) => Some(Self::Loaded),
            VisitState::Queued => Some(Self::NotLoaded),
        }
    }
}

#[allow(dead_code, reason = "changing CHOSEN switches every consumer's order")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WaitingOrder {
    NewestFirst,
    OldestFirst,
}

impl WaitingOrder {
    pub(crate) const CHOSEN: Self = Self::NewestFirst;

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NewestFirst => "newest first",
            Self::OldestFirst => "oldest first",
        }
    }

    fn sort(self, pages: &mut [&Visit]) {
        // Stable sorting keeps preorder for pages queued in the same millisecond.
        match self {
            Self::NewestFirst => pages.sort_by_key(|v| std::cmp::Reverse(v.created_at)),
            Self::OldestFirst => pages.sort_by_key(|v| v.created_at),
        }
    }
}

pub(crate) fn pages(forest: &ForestProjection) -> Vec<&Visit> {
    pages_in(forest, WaitingOrder::CHOSEN)
}

fn pages_in(forest: &ForestProjection, order: WaitingOrder) -> Vec<&Visit> {
    let Some(tree) = forest.current().and_then(|id| forest.tree_of(id)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![tree.root()];
    while let Some(id) = stack.pop() {
        let Some(visit) = forest.visit(id) else {
            continue;
        };
        if forest.is_waiting(id) {
            out.push(visit);
        }
        if forest.pending_descendant_count(id).unwrap_or(0) > 0 {
            stack.extend(forest.children(id).iter().rev().copied());
        }
    }
    order.sort(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Lifecycle, Snapshot, TreeId, TreeState};

    fn fixture(
        times: &[i64],
        parents: &[Option<usize>],
        current: Option<usize>,
        settled: bool,
    ) -> (ForestProjection, Vec<VisitId>) {
        let ids: Vec<VisitId> = times
            .iter()
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect();
        let visits = times
            .iter()
            .enumerate()
            .map(|(i, &created_at)| Visit {
                dismissed_at: None,
                branch_settled_at: None,
                id: ids[i],
                parent: parents[i].map(|p| ids[p]),
                url: url::Url::parse("https://example.org").unwrap(),
                title: format!("Page {i}"),
                created_at,
                opened_at: if i < 3 || parents[i].is_none() {
                    Some(created_at)
                } else {
                    None
                },
                state: if i < 3 || parents[i].is_none() {
                    VisitState::Opened
                } else {
                    VisitState::Queued
                },
                source: None,
            })
            .collect();
        let trees = parents
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_none())
            .map(|(i, _)| TreeState {
                pin: None,
                last_event_at: None,
                root: TreeId(ids[i]),
                last_visit: ids[i],
                name: None,
                lifecycle: if settled {
                    Lifecycle::Settled { since: 100 }
                } else {
                    Lifecycle::Open {
                        tab: crate::browser::TabOrder::new(0),
                    }
                },
            })
            .collect();
        (
            ForestProjection::from_snapshot(
                Snapshot {
                    visits,
                    current: current.map(|i| ids[i]),
                },
                trees,
            )
            .unwrap(),
            ids,
        )
    }

    #[test]
    fn waiting_order_is_current_tree_only_stable_and_reversible() {
        let parents = [
            None,
            Some(0),
            Some(0),
            Some(1),
            Some(1),
            Some(2),
            Some(0),
            None,
            Some(7),
        ];
        for settled in [false, true] {
            let (forest, ids) = fixture(
                &[0, 1, 2, 10, 30, 20, 40, 0, 50],
                &parents,
                Some(0),
                settled,
            );
            assert_eq!(
                pages(&forest).iter().map(|v| v.id).collect::<Vec<_>>(),
                [ids[6], ids[4], ids[5], ids[3]]
            );
            assert_eq!(
                pages_in(&forest, WaitingOrder::OldestFirst)
                    .iter()
                    .map(|v| v.id)
                    .collect::<Vec<_>>(),
                [ids[3], ids[5], ids[4], ids[6]]
            );
        }
        let (forest, ids) = fixture(&[0, 1, 2, 10, 30, 10, 40, 0, 50], &parents, Some(0), false);
        assert_eq!(
            pages(&forest).iter().map(|v| v.id).collect::<Vec<_>>(),
            [ids[6], ids[4], ids[3], ids[5]]
        );
        let (forest, _) = fixture(&[0, 1, 2, 10, 30, 20, 40, 0, 50], &parents, None, false);
        assert!(pages(&forest).is_empty());
    }

    #[test]
    fn loaded_set_only_changes_queued_pages() {
        let (forest, ids) = fixture(
            &[0, 1, 2, 3],
            &[None, Some(0), Some(1), Some(2)],
            Some(0),
            false,
        );
        assert_eq!(
            WaitState::of(forest.visit(ids[3]).unwrap(), &HashSet::new()),
            Some(WaitState::NotLoaded)
        );
        let loaded = ids.iter().copied().collect();
        assert_eq!(
            WaitState::of(forest.visit(ids[3]).unwrap(), &loaded),
            Some(WaitState::Loaded)
        );
        assert_eq!(WaitState::of(forest.visit(ids[0]).unwrap(), &loaded), None);
    }

    #[test]
    fn deep_history_is_walked_iteratively() {
        let times: Vec<i64> = (0..20_000).collect();
        let parents: Vec<_> = (0usize..20_000).map(|i| i.checked_sub(1)).collect();
        let (forest, _) = fixture(&times, &parents, Some(0), false);
        assert_eq!(pages(&forest).len(), 19_997);
    }
}
