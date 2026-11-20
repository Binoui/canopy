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

#[derive(Default)]
pub(crate) struct Waiting<'f> {
    pages: Vec<&'f Visit>,
    below: usize,
}

impl<'f> Waiting<'f> {
    pub(crate) fn of(forest: &'f ForestProjection) -> Self {
        let Some(current) = forest.current() else {
            return Self::default();
        };
        let Some(tree) = forest.tree_of(current) else {
            return Self::default();
        };
        let (mut below, mut elsewhere) = (Vec::new(), Vec::new());
        let mut stack = vec![(tree.root(), false)];
        while let Some((id, under_current)) = stack.pop() {
            let Some(visit) = forest.visit(id) else {
                continue;
            };
            if id != current && forest.is_waiting(id) {
                if under_current {
                    below.push(visit);
                } else {
                    elsewhere.push(visit);
                }
            }
            if forest.pending_descendant_count(id).unwrap_or(0) > 0 {
                let under = under_current || id == current;
                stack.extend(
                    forest
                        .children(id)
                        .iter()
                        .rev()
                        .map(|child| (*child, under)),
                );
            }
        }
        // Stable sorting keeps preorder for pages queued in the same millisecond.
        below.sort_by_key(|v| std::cmp::Reverse(v.created_at));
        elsewhere.sort_by_key(|v| std::cmp::Reverse(v.created_at));
        let below_count = below.len();
        below.extend(elsewhere);
        Self {
            pages: below,
            below: below_count,
        }
    }

    pub(crate) fn pages(&self) -> &[&'f Visit] {
        &self.pages
    }

    pub(crate) fn len(&self) -> usize {
        self.pages.len()
    }

    pub(crate) fn below(&self) -> usize {
        self.below
    }

    pub(crate) fn order_label() -> &'static str {
        "this page first, then newest"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, TreeId, TreeState};

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
                dismissed_at: (created_at == -2).then_some(54),
                branch_settled_at: (created_at == -1).then_some(54),
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
                last_event_at: None,
                root: TreeId(ids[i]),
                last_visit: ids[i],
                name: None,
                spot: if settled {
                    crate::browser::Spot::History { settled_at: 100 }
                } else {
                    crate::browser::Spot::Tab {
                        pinned: false,
                        position: 0,
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
    fn waiting_order_groups_descendants_and_keeps_preorder_on_ties() {
        let parents = [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(1),
            Some(1),
            Some(0),
            Some(0),
            None,
            Some(8),
            Some(0),
            Some(10),
            Some(1),
        ];
        let times = [0, 1, 2, 10, 20, 20, 30, 40, 0, 50, -1, 52, -2];
        let (mut forest, ids) = fixture(&times, &parents, Some(1), false);
        let order = |forest: &ForestProjection| {
            let waiting = Waiting::of(forest);
            (
                waiting.pages().iter().map(|v| v.id).collect::<Vec<_>>(),
                waiting.below(),
            )
        };
        assert_eq!(
            order(&forest),
            (vec![ids[4], ids[5], ids[3], ids[7], ids[6]], 3)
        );
        forest.show_current(ids[2]);
        assert_eq!(
            order(&forest),
            (vec![ids[3], ids[7], ids[6], ids[4], ids[5]], 1)
        );
        forest.show_current(ids[4]);
        assert_eq!(order(&forest), (vec![ids[7], ids[6], ids[5], ids[3]], 0));
        let (forest, _) = fixture(&times, &parents, None, false);
        assert_eq!(order(&forest), (vec![], 0));
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
        assert_eq!(Waiting::of(&forest).len(), 19_997);
    }
}
