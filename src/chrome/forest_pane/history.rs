use super::*;

pub(super) fn value(
    forest: &ForestProjection,
    index: &Index,
    rows: &[&Entry],
    cursor: &Cursor,
    calendar: &dyn Calendar,
) -> Value {
    let today = calendar.now();
    let mut items = Vec::new();
    let shown = &rows[..cursor.reach.min(rows.len())];
    for run in shown.chunk_by(|a, b| a.at.date == b.at.date) {
        let e = run[0];
        items.push(json!({"kind":"day", "key":format!("day-{}",e.at.date), "label":e.at.label(&today), "count":plural(rows.iter().filter(|r| r.at.date == e.at.date).count(), "tree")}));
        for e in run {
            let tree = forest.trees().iter().find(|t| t.id == e.tree).unwrap();
            let sites = &index.facts[&e.tree].sites;
            let selected = cursor.anchor.is_some_and(|a| a.tree == e.tree);
            items.push(json!({"kind":"row", "key":format!("row-{}",e.tree.root()), "id":e.tree.root().to_string(), "time":e.at.clock,
                "title":tree.title, "icons":sites.iter().take(3).map(|s| json!({"url":s.sample.as_str()})).collect::<Vec<_>>(),
                "more": (sites.len()>3).then(|| format!("+{}",sites.len()-3)), "sites":tree_card::sites_line(sites,3),
                "pages":plural(tree.page_count,"page"), "selected":selected,
                "name":format!("{}, {}, {}",tree.title,plural(tree.page_count,"page"),e.at.when(&today)), "menu":tree_card::menu(tree,Place::History)}));
        }
    }
    json!({"items":items, "more":(shown.len()<rows.len()).then(|| json!({"seen":shown.len(), "label":format!("Show {} older trees",rows.len()-shown.len())})), "preview":cursor.anchor.and_then(|a| rows.iter().find(|e| e.tree == a.tree)).map(|e| preview(forest,e,&today)),
        "empty":rows.is_empty().then_some(if index.history.is_empty() { "No settled trees yet. A tree you settle waits here." } else { "No settled tree matches." })})
}

pub(super) const OUTLINE_MAX: usize = 40;
fn preview(forest: &ForestProjection, entry: &Entry, today: &days::Civil) -> Value {
    let tree = forest.trees().iter().find(|t| t.id == entry.tree).unwrap();
    let mut outline = Vec::new();
    let mut stack = vec![(tree.id.root(), 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let v = forest.visit(id).unwrap();
        let name = super::super::graph::short_name(forest, id);
        let label = format!(
            "{name}, depth {depth}{}{}",
            if id == tree.last_visit {
                ", last page"
            } else {
                ""
            },
            if forest.is_waiting(id) {
                ", waiting"
            } else {
                ""
            }
        );
        outline.push(json!({"id":id.to_string(), "depth":depth, "url":v.url.as_str(), "name":name, "label":label, "last":id==tree.last_visit, "waiting":forest.is_waiting(id)}));
        if outline.len() == OUTLINE_MAX {
            break;
        }
        stack.extend(forest.children(id).iter().rev().map(|&c| (c, depth + 1)));
    }
    json!({"id":tree.id.root().to_string(), "key":format!("pv-{}",tree.id.root()), "title":tree.title,
        "meta":format!("{} · last used {}",plural(tree.page_count,"page"),entry.at.when(today)),
        "mini":tree_card::mini_value(forest,&tree_view::miniature(forest,tree,Fit::PREVIEW)),
        "actions":[{"kind":"reopen","label":"Reopen"},{"kind":"save","label":"Save"}],
        "cut":(tree.page_count>outline.len()).then(||format!("{} more pages",tree.page_count-outline.len())), "outline":outline})
}
