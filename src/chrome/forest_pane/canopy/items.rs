use super::Level;
use crate::{
    browser::{Showings, VisitId, VisitState},
    forest::{ForestProjection, TreeSummary},
};
use std::collections::{HashMap, HashSet};

pub(super) struct Facts<'a> {
    pub forest: &'a ForestProjection,
    pub previous: Option<VisitId>,
    pub live: &'a HashSet<VisitId>,
    pub shown: &'a Showings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Memory {
    Open,
    Visited,
    Waiting,
}
impl Memory {
    pub fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Visited => "visited",
            Self::Waiting => "waiting",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "still open",
            Self::Visited => "visited",
            Self::Waiting => "waiting",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Appearance {
    pub current: bool,
    pub previous: bool,
    pub memory: Memory,
}
#[derive(Debug)]
pub(super) struct Item {
    pub pages: Vec<VisitId>,
    pub parent: Option<usize>,
    pub appearance: Appearance,
    pub shown: u32,
    pub go: VisitId,
    pub named: bool,
    folds: bool,
}
pub(super) fn site(forest: &ForestProjection, id: VisitId) -> &str {
    let url = &forest.visit(id).unwrap().url;
    let host = url.host_str().unwrap_or(url.scheme());
    host.strip_prefix("www.").unwrap_or(host)
}
pub(super) fn drawn(forest: &ForestProjection, id: VisitId) -> bool {
    !forest.is_settled(id)
        && forest
            .visit(id)
            .is_some_and(|v| v.state == VisitState::Opened || forest.is_waiting(id))
}
pub(super) fn fold(facts: &Facts<'_>, tree: &TreeSummary, level: Level) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut of = HashMap::new();
    let mut stack = vec![tree.id.root()];
    while let Some(id) = stack.pop() {
        if facts.forest.is_settled(id) {
            continue;
        }
        stack.extend(facts.forest.children(id).iter().rev());
        if !drawn(facts.forest, id) {
            continue;
        }
        let notable = Some(id) == facts.forest.current()
            || Some(id) == facts.previous
            || facts.shown.times(id) >= 3;
        let alone = level == Level::Near || (level == Level::Middle && notable);
        let parent: Option<usize> = facts
            .forest
            .visit(id)
            .unwrap()
            .parent
            .and_then(|p| of.get(&p).copied());
        let join = !alone
            && parent.is_some_and(|p| {
                items[p].folds && site(facts.forest, items[p].pages[0]) == site(facts.forest, id)
            });
        let index = if join {
            let index = parent.unwrap();
            items[index].pages.push(id);
            index
        } else {
            let index = items.len();
            items.push(Item {
                pages: vec![id],
                parent,
                appearance: Appearance {
                    current: false,
                    previous: false,
                    memory: Memory::Visited,
                },
                shown: 0,
                go: id,
                named: alone,
                folds: !alone,
            });
            index
        };
        of.insert(id, index);
    }
    for item in &mut items {
        item.appearance = Appearance {
            current: item
                .pages
                .iter()
                .any(|id| Some(*id) == facts.forest.current()),
            previous: item.pages.iter().any(|id| Some(*id) == facts.previous),
            memory: if item.pages.iter().all(|id| facts.forest.is_waiting(*id)) {
                Memory::Waiting
            } else if item
                .pages
                .iter()
                .any(|id| facts.live.contains(id) || Some(*id) == facts.forest.current())
            {
                Memory::Open
            } else {
                Memory::Visited
            },
        };
        item.shown = item
            .pages
            .iter()
            .map(|id| facts.shown.times(*id))
            .max()
            .unwrap_or(0);
        item.go = item
            .pages
            .iter()
            .copied()
            .max_by_key(|id| facts.shown.last_shown(*id))
            .filter(|id| facts.shown.last_shown(*id).is_some())
            .unwrap_or(item.pages[0]);
    }
    items
}
pub(super) fn size(shown: u32, base: f64) -> f64 {
    (base * (1. + 0.3 * f64::from(shown.max(1)).log2()).min(2.1)).round()
}
