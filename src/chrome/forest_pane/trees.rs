use super::super::view::plural;
use super::*;
use crate::forest::ForestProjection;
#[derive(Default)]
pub(super) struct State {
    pub query: String,
}
#[derive(Debug)]
pub(in crate::chrome) enum Input {
    Find(String),
}
impl Input {
    pub fn parse(m: &Value) -> Result<Option<Self>> {
        Ok(match m["do"].as_str() {
            Some("trees-find") => Some(Self::Find(
                m["query"].as_str().context("Missing Find text")?.to_owned(),
            )),
            _ => None,
        })
    }
}
impl State {
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn escape(&mut self) -> bool {
        let changed = !self.query.is_empty();
        self.query.clear();
        changed
    }

    pub fn input(
        &mut self,
        input: Input,
        _forest: &ForestProjection,
        _calendar: &dyn Calendar,
    ) -> Reply {
        match input {
            Input::Find(q) => self.query = q,
        }
        Reply::Unchanged
    }
    pub fn value(
        &self,
        forest: &ForestProjection,
        index: &Index,
        _calendar: &dyn Calendar,
    ) -> Value {
        value(forest, index, &self.query)
    }
}

fn value(forest: &ForestProjection, index: &Index, query: &str) -> Value {
    let key = query.trim().to_lowercase();
    let current = forest.current().and_then(|id| forest.tree_of(id));
    let mut items = Vec::new();
    for (band, title, trees) in [
        ("open", "Tabs", forest.tabs().iter().collect::<Vec<_>>()),
        ("saved", "Saved for later", forest.shelf()),
    ] {
        items.push(json!({"kind":"band", "key":format!("band-{band}"), "title":title, "detail":plural(trees.len(), "tree")}));
        for tree in trees.iter().filter(|t| index.admits(t.id, &key)) {
            let mut card = tree_card::card(forest, tree, current == Some(tree.id));
            card["kind"] = json!("card");
            card["key"] = json!(format!("{band}-{}", tree.id.root()));
            items.push(card);
        }
        if band == "saved" && trees.is_empty() {
            items.push(json!({"kind":"hint", "key":"saved-hint", "text":"Choose Save for later in a tab’s menu to keep it here for later."}));
        }
    }
    let empty = (!items.iter().any(|i| i["kind"] == "card") && !key.is_empty())
        .then_some("No open or saved tree matches.");
    json!({"items":items, "empty":empty})
}
