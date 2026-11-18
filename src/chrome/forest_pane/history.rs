use super::super::{
    tree_view::{self, Fit},
    view::plural,
};
use super::index::Entry;
use super::*;
use crate::forest::ForestProjection;
#[derive(Debug)]
pub(in crate::chrome) enum Input {
    Select(TreeId),
    Step(i8),
    More { seen: usize },
    Find(String),
}
impl Input {
    pub fn parse(m: &Value) -> Result<Option<Self>> {
        Ok(Some(match m["do"].as_str() {
            Some("find") => {
                Input::Find(m["query"].as_str().context("Missing Find text")?.to_owned())
            }

            Some("more") => Input::More {
                seen: usize::try_from(m["seen"].as_u64().context("Missing row count")?)?,
            },
            Some("select") => {
                Input::Select(TreeId(m["tree"].as_str().context("Missing tree")?.parse()?))
            }
            Some("step") => Input::Step(match m["by"].as_i64() {
                Some(-1) => -1,
                Some(1) => 1,
                _ => bail!("Unknown forest step"),
            }),
            _ => return Ok(None),
        }))
    }
}
pub(super) struct State {
    pub query: String,
    pub anchor: Option<Anchor>,
    pub reach: usize,
}
#[derive(Clone, Copy)]
pub(super) struct Anchor {
    pub tree: TreeId,
    pub at: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            query: String::new(),
            anchor: None,
            reach: PAGE,
        }
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

    pub fn rows<'a>(&self, index: &'a Index) -> Vec<&'a Entry> {
        let key = self.query.trim().to_lowercase();
        index
            .history
            .iter()
            .filter(|e| index.admits(e.tree, &key))
            .collect()
    }
    pub fn input(
        &mut self,
        input: Input,
        index: &Index,
        _forest: &ForestProjection,
        _calendar: &dyn Calendar,
    ) -> Reply {
        match input {
            Input::Find(query) => {
                if query != self.query {
                    self.query = query;
                    self.anchor = None;
                    self.reach = PAGE;
                }
            }
            Input::More { seen } => {
                if seen == self.reach {
                    self.reach = self.reach.saturating_add(PAGE);
                }
            }
            Input::Select(tree) => {
                let rows = self.rows(index);
                if let Some(at) = rows.iter().take(self.reach).position(|e| e.tree == tree) {
                    if at + 1 >= self.reach && self.reach < rows.len() {
                        self.reach += PAGE;
                    }
                    self.anchor = Some(Anchor { tree, at });
                }
            }
            Input::Step(by) => {
                let rows = self.rows(index);
                if let Some(at) = resolve(self.anchor, &rows) {
                    let at = if by < 0 {
                        at.saturating_sub(1)
                    } else {
                        (at + 1).min(rows.len() - 1)
                    };
                    if at + 1 >= self.reach && self.reach < rows.len() {
                        self.reach += PAGE;
                    }
                    self.anchor = Some(Anchor {
                        tree: rows[at].tree,
                        at,
                    });
                }
            }
        }
        Reply::Unchanged
    }
    pub fn selected(&self, index: &Index) -> Option<TreeId> {
        let rows = self.rows(index);
        resolve(self.anchor, &rows).map(|i| rows[i].tree)
    }
    pub fn value(
        &mut self,
        forest: &ForestProjection,
        index: &Index,
        calendar: &dyn Calendar,
    ) -> Value {
        let rows = self.rows(index);
        self.anchor = resolve(self.anchor, &rows).map(|at| Anchor {
            tree: rows[at].tree,
            at,
        });
        if let Some(a) = self.anchor {
            self.reach = self.reach.max((a.at / PAGE + 1) * PAGE);
        }
        value(forest, index, &rows, self, calendar)
    }
}
pub(super) fn resolve(anchor: Option<Anchor>, rows: &[&Entry]) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    Some(anchor.map_or(0, |a| {
        rows.iter()
            .position(|e| e.tree == a.tree)
            .unwrap_or(a.at.min(rows.len() - 1))
    }))
}

fn value(
    forest: &ForestProjection,
    index: &Index,
    rows: &[&Entry],
    cursor: &State,
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
                "name":format!("{}, {}, {}",tree.title,plural(tree.page_count,"page"),e.at.when(&today)), "menu":tree_card::menu(tree)}));
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
        "actions":[{"kind":"reopen","label":"Reopen"},{"kind":"save","label":"Save for later"}],
        "cut":(tree.page_count>outline.len()).then(||format!("{} more pages",tree.page_count-outline.len())), "outline":outline})
}

#[cfg(test)]
mod tests {
    use super::super::tests::{change, fixture, rows, value};
    use super::super::{Input, State};
    use super::*;
    use crate::browser::Spot;
    #[test]
    fn history_starts_first_and_steps() {
        let f = fixture(4, 4);
        let mut p = ForestPane::default();
        assert!(matches!(p.present(&f), Reply::Unchanged));
        let v = value(p.input(Input::Open(View::History), &f));
        assert_eq!(rows(&v).len(), 3);
        assert_eq!(rows(&v)[0]["title"], "Tree 3");
        assert_eq!(rows(&v)[0]["selected"], true);
        let v = value(p.input(Input::History(super::Input::Step(1)), &f));
        assert_eq!(rows(&v)[1]["selected"], true);
        p.input(Input::Close, &f);
        let v = value(p.input(Input::Open(View::History), &f));
        assert_eq!(rows(&v)[0]["selected"], true);
    }
    #[test]
    fn removed_anchor_selects_successor_and_sites_are_ranked() {
        let f = fixture(4, 5);
        let index = Index::build(&f.forest, &LocalCalendar);
        let rows: Vec<_> = index.history.iter().collect();
        assert_eq!(
            super::resolve(
                Some(super::Anchor {
                    tree: rows[1].tree,
                    at: 1
                }),
                &[rows[0], rows[2]]
            ),
            Some(1)
        );
        assert_eq!(
            super::resolve(
                Some(super::Anchor {
                    tree: rows[2].tree,
                    at: 2
                }),
                &[rows[0]]
            ),
            Some(0)
        );
        assert_eq!(super::resolve(None, &[]), None);
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        assert_eq!(self::rows(&v)[0]["icons"].as_array().unwrap().len(), 3);
        assert_eq!(self::rows(&v)[0]["more"], "+1");
        assert!(self::rows(&v)[0]["sites"]
            .as_str()
            .unwrap()
            .starts_with("site0.example"));
    }
    #[test]
    fn preview_uses_true_depth_preorder_and_caps_the_outline() {
        let f = fixture(2, 80);
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        let preview = &v["history"]["preview"];
        let outline = preview["outline"].as_array().unwrap();
        assert_eq!(outline.len(), super::OUTLINE_MAX);
        assert_eq!(outline[0]["depth"], 0);
        assert_eq!(outline[39]["depth"], 39);
        assert_eq!(preview["cut"], "40 more pages");
        assert_eq!(
            preview["actions"],
            json!([{"kind":"reopen","label":"Reopen"},{"kind":"save","label":"Save for later"}])
        );
    }
    #[test]
    fn saving_selected_history_keeps_it_settled_and_selects_the_successor() {
        let mut f = fixture(4, 4);
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        let selected = rows(&v)[0]["id"].as_str().unwrap().to_owned();
        let successor = rows(&v)[1]["id"].clone();
        let spot = Spot::Shelf {
            position: 0,
            saved_at: 1,
        };
        change(&mut f, |states| {
            states
                .iter_mut()
                .find(|s| s.root.root().to_string() == selected)
                .unwrap()
                .spot = spot
        });
        let v = value(p.present(&f));
        assert_eq!(rows(&v)[0]["id"], successor);
        assert_eq!(rows(&v)[0]["selected"], true);
        assert_eq!(v["count"], "1 tabs · 1 saved for later · 2 in history");
        assert_eq!(f.forest.history().len(), 2);
    }
    #[test]
    fn two_thousand_trees_send_sixty_rows_and_one_preview_without_hidden_visits() {
        let f = fixture(2001, 10);
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        assert_eq!(rows(&v).len(), PAGE);
        assert_eq!(
            v["history"]["preview"]["outline"].as_array().unwrap().len(),
            10
        );
        assert!(v.to_string().len() < 64 * 1024);
        let base = crate::chrome::view::frame(
            &f,
            &[],
            (1240., 900.),
            &tree_view::Workbench::default(),
            Default::default(),
        );
        assert!(base.get("forest").is_none());
        assert_eq!(base["visits"].as_object().unwrap().len(), 10);
        let v = value(p.input(Input::History(super::Input::More { seen: 60 }), &f));
        assert_eq!(rows(&v).len(), 120);
        assert!(matches!(
            p.input(Input::History(super::Input::More { seen: 60 }), &f),
            Reply::Unchanged
        ));
        for _ in 0..119 {
            p.input(Input::History(super::Input::Step(1)), &f);
        }
        let State::Open(open) = &p.state else {
            panic!()
        };
        let Shown::History(c) = &open.shown else {
            panic!()
        };
        assert_eq!(c.reach, 180);
    }
    #[test]
    fn find_searches_titles_and_every_site_and_escape_clears_then_closes() {
        let f = fixture(4, 4);
        let mut p = ForestPane::default();
        p.input(Input::Open(View::History), &f);
        let v = value(p.input(Input::History(super::Input::Find("TREE 2".into())), &f));
        assert_eq!(rows(&v).len(), 1);
        assert_eq!(rows(&v)[0]["title"], "Tree 2");
        assert_eq!(v["count"], "1 tabs · 0 saved for later · 3 in history");
        let v = value(p.input(Input::History(super::Input::Find("site3".into())), &f));
        assert_eq!(rows(&v).len(), 3);
        let v = value(p.input(Input::History(super::Input::Find("absent".into())), &f));
        assert_eq!(v["history"]["empty"], "No settled tree matches.");
        let v = value(p.input(Input::Escape, &f));
        assert_eq!(v["query"], "");
        assert_eq!(rows(&v).len(), 3);
        assert!(matches!(p.input(Input::Escape, &f), Reply::Close));
        p.input(Input::History(super::Input::Find("unkept".into())), &f);
        p.input(Input::Close, &f);
    }
    #[test]
    fn history_dates_use_events_else_settle_and_day_counts_include_unloaded_rows() {
        let mut f = fixture(132, 1);
        change(&mut f, |states| {
            for s in states {
                s.last_event_at = None;
                if !s.spot.place().is_tab() {
                    s.spot = crate::browser::Spot::History {
                        settled_at: 1_790_000_000_000,
                    };
                }
            }
        });
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        assert_eq!(rows(&v).len(), 60);
        assert_eq!(v["history"]["items"][0]["count"], "131 trees");
        let tree = f.forest.history().last().unwrap().id;
        change(&mut f, |states| {
            states.iter_mut().find(|s| s.root == tree).unwrap().spot = Spot::History {
                settled_at: 1_800_000_000_000,
            }
        });
        let v = value(p.present(&f));
        assert_eq!(rows(&v)[0]["id"], tree.root().to_string());
        let f = fixture(1, 1);
        p.input(Input::Close, &f);
        let v = value(p.input(Input::Open(View::History), &f));
        assert_eq!(
            v["history"]["empty"],
            "No settled trees yet. A tree you settle waits here."
        );
        assert!(v["history"]["preview"].is_null());
    }
}
