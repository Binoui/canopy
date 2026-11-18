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
impl Item {
    pub fn key(&self) -> String {
        format!("{}:{}", if self.folds { "r" } else { "p" }, self.pages[0])
    }
    pub fn paint(&self, facts: &Facts<'_>, level: Level) -> serde_json::Value {
        use serde_json::json;
        let page = facts.forest.visit(self.go).unwrap();
        let site = site(facts.forest, self.go);
        let run = self.folds && level != Level::Near;
        let title = if level == Level::Far {
            None
        } else if run {
            Some(site.to_owned())
        } else if self.named {
            Some(page.label().to_owned())
        } else {
            None
        };
        let sub = match level {
            Level::Far => None,
            Level::Middle if run => {
                Some(super::super::super::view::plural(self.pages.len(), "page"))
            }
            Level::Middle => (self.shown > 1).then(|| format!("shown {}×", self.shown)),
            Level::Near => Some(format!(
                "{site} · {}{}",
                self.appearance.memory.label(),
                if self.shown > 1 {
                    format!(" · {}×", self.shown)
                } else {
                    String::new()
                }
            )),
        };
        let mut tip = format!(
            "{}\n{site} · {}",
            page.label(),
            self.appearance.memory.label()
        );
        if run {
            tip.push_str(&format!(
                "\n{} · A click opens {}",
                super::super::super::view::plural(self.pages.len(), "page"),
                page.label()
            ));
        }
        if self.shown > 1 {
            tip.push_str(&format!("\nshown {} times", self.shown));
        }
        json!({"k":self.key(), "kind":if run {"run"} else {"page"}, "go":self.go.to_string(), "page":self.go.to_string(),
            "pages":self.pages.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "st":self.appearance.memory.name(), "current":self.appearance.current, "previous":self.appearance.previous,
            "sz":size(self.shown, level.metrics().icon), "t":title, "sub":sub, "n":run.then_some(self.pages.len()),
            "tags": ([self.appearance.current.then_some("You are here"), (self.appearance.previous && level != Level::Far).then_some("Back")].into_iter().flatten().collect::<Vec<_>>()),
            "tip":tip,"url":page.url.as_str(),"e":if self.appearance.memory == Memory::Waiting {"waiting"} else {"plain"}})
    }
}
