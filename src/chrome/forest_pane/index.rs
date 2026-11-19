use super::{
    super::tree_card::{self, Site},
    days::{Calendar, Civil},
};
use crate::{
    browser::{Place, TreeId},
    forest::ForestProjection,
};
use std::collections::HashMap;

pub(super) struct Index {
    pub facts: HashMap<TreeId, Facts>,
    pub history: Vec<Entry>,
}
pub(super) struct Facts {
    title: String,
    pub sites: Vec<Site>,
}
pub(super) struct Entry {
    pub tree: TreeId,
    pub at: Civil,
    used: i64,
}
impl Index {
    pub fn build(forest: &ForestProjection, calendar: &dyn Calendar) -> Self {
        let mut facts = HashMap::new();
        let mut history = Vec::new();
        for tree in forest.trees() {
            facts.insert(
                tree.id,
                Facts {
                    title: tree.title.to_lowercase(),
                    sites: tree_card::sites(forest, tree),
                },
            );
            if let Place::History { settled_at } = tree.place {
                if !tree.place.pinned() {
                    let used = settled_at;
                    history.push(Entry {
                        tree: tree.id,
                        at: calendar.civil(used),
                        used,
                    });
                }
            }
        }
        history.sort_by_key(|e| (std::cmp::Reverse(e.used), e.tree.root().to_string()));
        Self { facts, history }
    }
    pub fn admits(&self, tree: TreeId, key: &str) -> bool {
        let f = &self.facts[&tree];
        key.is_empty() || f.title.contains(key) || f.sites.iter().any(|s| s.host.contains(key))
    }
}
