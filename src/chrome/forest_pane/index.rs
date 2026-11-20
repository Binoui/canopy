use super::{
    super::tree_card::{self, Site},
    days::{Calendar, Civil},
};
use crate::{
    browser::{Place, TreeId, VisitId},
    forest::{ForestProjection, TreeSummary},
};
use std::{cell::OnceCell, collections::HashMap, rc::Rc};

pub(super) struct Index {
    pub facts: HashMap<TreeId, Facts>,
    pub history: Vec<Entry>,
}
pub(super) struct Facts {
    title: String,
    day: Option<Rc<Civil>>,
    pub sites: Vec<Site>,
    pages: OnceCell<Vec<VisitId>>,
    titles: OnceCell<Vec<String>>,
}
impl Facts {
    pub fn pages(&self, tree: TreeId, forest: &ForestProjection) -> &[VisitId] {
        self.pages.get_or_init(|| {
            let mut pages = Vec::new();
            let mut stack = vec![tree.root()];
            while let Some(id) = stack.pop() {
                pages.push(id);
                stack.extend(forest.children(id).iter().rev());
            }
            pages
        })
    }
    fn new(
        tree: &TreeSummary,
        forest: &ForestProjection,
        calendar: &dyn Calendar,
        days: &mut HashMap<i64, Rc<Civil>>,
    ) -> Self {
        let at = match tree.place {
            Place::History { settled_at } => Some(settled_at),
            _ => None,
        };
        Self {
            title: tree.title.to_lowercase(),
            day: at.map(|at| {
                days.entry(at.div_euclid(1000))
                    .or_insert_with(|| Rc::new(calendar.civil(at)))
                    .clone()
            }),
            sites: tree_card::sites(forest, tree),
            pages: OnceCell::new(),
            titles: OnceCell::new(),
        }
    }
}
pub(super) struct Entry {
    pub tree: TreeId,
    pub at: Rc<Civil>,
}
impl Index {
    pub fn build(forest: &ForestProjection, calendar: &dyn Calendar) -> Self {
        let mut facts = HashMap::with_capacity(forest.trees().len());
        let mut days = HashMap::new();
        for tree in forest.trees() {
            let fact = Facts::new(tree, forest, calendar, &mut days);
            facts.insert(tree.id, fact);
        }
        let history = forest
            .history()
            .iter()
            .map(|t| Entry {
                tree: t.id,
                at: facts[&t.id].day.clone().unwrap(),
            })
            .collect();
        Self { facts, history }
    }
    pub fn admits(&self, tree: TreeId, key: &str) -> bool {
        let f = &self.facts[&tree];
        key.is_empty() || f.title.contains(key) || f.sites.iter().any(|s| s.host.contains(key))
    }
    pub fn page_hit(&self, tree: TreeId, key: &str, forest: &ForestProjection) -> Option<VisitId> {
        if key.is_empty() {
            return None;
        }
        let f = &self.facts[&tree];
        let pages = f.pages(tree, forest);
        let titles = f.titles.get_or_init(|| {
            pages
                .iter()
                .map(|&id| forest.visit(id).unwrap().title.to_lowercase())
                .collect()
        });
        titles
            .iter()
            .position(|title| title.contains(key))
            .map(|i| pages[i])
    }
}
