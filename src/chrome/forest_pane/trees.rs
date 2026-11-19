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

#[cfg(test)]
mod tests {
    use super::super::tests::{change, fixture, value};
    use super::super::Input;
    use super::*;
    #[test]
    fn tabs_and_shelf_are_disjoint_ordered_bands() {
        let mut f = fixture(4, 4);
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::Trees), &f));
        assert_eq!(v["count"], "1 tabs · 0 saved for later · 3 in history");
        assert_eq!(
            v["trees"]["items"][1]["id"],
            f.forest.tabs()[0].id.root().to_string()
        );
        assert_eq!(v["trees"]["items"][1]["current"], true);
        let saved = f.forest.history()[0].id;
        change(&mut f, |states| {
            states.iter_mut().find(|s| s.root == saved).unwrap().spot =
                crate::browser::Spot::Shelf {
                    position: 0,
                    saved_at: 1,
                }
        });
        let v = value(p.present(&f));
        let items = v["trees"]["items"].as_array().unwrap();
        let card = items
            .iter()
            .find(|c| c["key"].as_str().is_some_and(|k| k.starts_with("saved-")))
            .unwrap();
        assert_eq!(card["id"], saved.root().to_string());
        assert!(card["badge"].is_null());
        let v = value(p.input(Input::Trees(super::Input::Find("absent".into())), &f));
        assert_eq!(v["trees"]["empty"], "No open or saved tree matches.");
    }
}
