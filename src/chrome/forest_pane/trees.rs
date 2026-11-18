use super::*;

pub(super) fn value(forest: &ForestProjection, index: &Index, query: &str) -> Value {
    let key = query.trim().to_lowercase();
    let current = forest.current().and_then(|id| forest.tree_of(id));
    let mut items = Vec::new();
    for (band, title, trees) in [
        ("open", "Open", forest.open().iter().collect::<Vec<_>>()),
        ("saved", "Saved", forest.pinned()),
    ] {
        items.push(json!({"kind":"band", "key":format!("band-{band}"), "title":title, "detail":plural(trees.len(), "tree")}));
        for tree in trees.iter().filter(|t| index.admits(t.id, &key)) {
            let place = if band == "open" {
                Place::OpenBand {
                    current: current == Some(tree.id),
                }
            } else {
                Place::SavedBand
            };
            let mut card = tree_card::card(forest, tree, place);
            card["kind"] = json!("card");
            card["key"] = json!(format!("{band}-{}", tree.id.root()));
            items.push(card);
        }
        if band == "saved" && trees.is_empty() {
            items.push(json!({"kind":"hint", "key":"saved-hint", "text":"Save a tree from its menu to keep it here for later."}));
        }
    }
    let empty = (!items.iter().any(|i| i["kind"] == "card") && !key.is_empty())
        .then_some("No open or saved tree matches.");
    json!({"items":items, "empty":empty})
}
