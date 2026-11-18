const PAGE: usize = 60;

mod days;
mod index;

use super::{
    tree_card::{self, Place},
    tree_view::{self, Fit},
    view::plural,
    ChromeFrame,
};
use crate::{browser::TreeId, forest::ForestProjection};
use anyhow::{bail, Context, Result};
use days::{Calendar, LocalCalendar};
use index::{Entry, Index};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Trees,
    History,
}
impl View {
    pub fn name(self) -> &'static str {
        match self {
            Self::Trees => "trees",
            Self::History => "history",
        }
    }
}
#[derive(Debug)]
pub(super) enum Input {
    Open(View),
    Close,
    Show(View),
    Select(TreeId),
    Step(i8),
    More { seen: usize },
    Find(String),
    Escape,
    Toggle(View),
}
impl Input {
    pub fn parse(m: &Value) -> Result<Self> {
        let view = || match m["view"].as_str() {
            Some("trees") => Ok(View::Trees),
            Some("history") => Ok(View::History),
            _ => bail!("Unknown forest view"),
        };
        Ok(match m["do"].as_str() {
            Some("open") => Self::Open(view()?),
            Some("close") => Self::Close,
            Some("show") => Self::Show(view()?),
            Some("toggle") => Self::Toggle(view()?),
            Some("find") => {
                Self::Find(m["query"].as_str().context("Missing Find text")?.to_owned())
            }
            Some("escape") => Self::Escape,
            Some("more") => Self::More {
                seen: usize::try_from(m["seen"].as_u64().context("Missing row count")?)?,
            },
            Some("select") => {
                Self::Select(TreeId(m["tree"].as_str().context("Missing tree")?.parse()?))
            }
            Some("step") => Self::Step(match m["by"].as_i64() {
                Some(-1) => -1,
                Some(1) => 1,
                _ => bail!("Unknown forest step"),
            }),
            _ => bail!("Unknown forest input"),
        })
    }
}
pub(super) enum Reply {
    Unchanged,
    Show(Value),
    Close,
}
pub(super) struct ForestPane {
    calendar: Box<dyn Calendar>,
    state: State,
}
enum State {
    Closed,
    Open(Open),
}
struct Open {
    query: String,
    shown: Shown,
    sent: Option<Value>,
}
enum Shown {
    Trees,
    History(Cursor),
}
struct Cursor {
    anchor: Option<Anchor>,
    reach: usize,
}
#[derive(Clone, Copy)]
struct Anchor {
    tree: TreeId,
    at: usize,
}
impl Shown {
    fn new(view: View) -> Self {
        match view {
            View::Trees => Self::Trees,
            View::History => Self::History(Cursor {
                anchor: None,
                reach: PAGE,
            }),
        }
    }
    fn view(&self) -> View {
        match self {
            Self::Trees => View::Trees,
            Self::History(_) => View::History,
        }
    }
}
impl Default for ForestPane {
    fn default() -> Self {
        Self {
            calendar: Box::new(LocalCalendar),
            state: State::Closed,
        }
    }
}
impl ForestPane {
    pub fn input(&mut self, input: Input, frame: &ChromeFrame) -> Reply {
        match input {
            Input::Close => {
                self.state = State::Closed;
                return Reply::Unchanged;
            }
            Input::Open(view) if matches!(self.state, State::Closed) => {
                self.state = State::Open(Open {
                    query: String::new(),
                    shown: Shown::new(view),
                    sent: None,
                })
            }
            Input::Open(_) => {}
            _ => {
                if let State::Open(open) = &mut self.state {
                    match input {
                        Input::Show(view) => {
                            if open.shown.view() != view {
                                open.shown = Shown::new(view);
                            }
                        }
                        Input::Toggle(view) => match (view, &open.shown) {
                            (View::History, Shown::Trees) => open.shown = Shown::new(View::History),
                            _ => return Reply::Close,
                        },
                        Input::Find(query) => {
                            if query != open.query {
                                open.query = query;
                                if matches!(open.shown, Shown::History(_)) {
                                    open.shown = Shown::new(View::History);
                                }
                            }
                        }
                        Input::Escape => {
                            if open.query.is_empty() {
                                return Reply::Close;
                            } else {
                                open.query.clear();
                                if matches!(open.shown, Shown::History(_)) {
                                    open.shown = Shown::new(View::History);
                                }
                            }
                        }
                        Input::More { seen } => {
                            if let Shown::History(c) = &mut open.shown {
                                if seen == c.reach {
                                    c.reach = c.reach.saturating_add(PAGE);
                                }
                            }
                        }
                        Input::Select(tree) => {
                            let index = Index::build(&frame.forest, &*self.calendar);
                            let rows = open.rows(&index);
                            if let Shown::History(c) = &mut open.shown {
                                if let Some(at) =
                                    rows.iter().take(c.reach).position(|e| e.tree == tree)
                                {
                                    if at + 1 >= c.reach && c.reach < rows.len() {
                                        c.reach += PAGE;
                                    }
                                    c.anchor = Some(Anchor { tree, at });
                                }
                            }
                        }
                        Input::Step(by) => {
                            let index = Index::build(&frame.forest, &*self.calendar);
                            let rows = open.rows(&index);
                            if let Shown::History(c) = &mut open.shown {
                                if let Some(at) = resolve(c.anchor, &rows) {
                                    let at = if by < 0 {
                                        at.saturating_sub(1)
                                    } else {
                                        (at + 1).min(rows.len() - 1)
                                    };
                                    if at + 1 >= c.reach && c.reach < rows.len() {
                                        c.reach += PAGE;
                                    }
                                    c.anchor = Some(Anchor {
                                        tree: rows[at].tree,
                                        at,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        self.present(frame)
    }
    pub fn present(&mut self, frame: &ChromeFrame) -> Reply {
        let State::Open(open) = &mut self.state else {
            return Reply::Unchanged;
        };
        let index = Index::build(&frame.forest, &*self.calendar);
        let rows = open.rows(&index);
        if let Shown::History(c) = &mut open.shown {
            c.anchor = resolve(c.anchor, &rows).map(|at| Anchor {
                tree: rows[at].tree,
                at,
            });
            if let Some(a) = c.anchor {
                c.reach = c.reach.max((a.at / PAGE + 1) * PAGE);
            }
        }
        let value = json!({ "view": open.shown.view().name(), "query": open.query,
            "count": format!("{} open · {} saved · {} in history", frame.forest.open().len(), frame.forest.pinned().len(), index.history.len()),
            "trees": matches!(open.shown, Shown::Trees).then(|| trees(&frame.forest, &index, &open.query)),
            "history": match &open.shown { Shown::History(c) => Some(history(&frame.forest, &index, &rows, c, &*self.calendar)), _ => None } });
        if open.sent.as_ref() == Some(&value) {
            return Reply::Unchanged;
        }
        open.sent = Some(value.clone());
        Reply::Show(value)
    }
}
impl Open {
    fn rows<'a>(&self, index: &'a Index) -> Vec<&'a Entry> {
        let key = self.query.trim().to_lowercase();
        index
            .history
            .iter()
            .filter(|e| index.admits(e.tree, &key))
            .collect()
    }
}
fn resolve(anchor: Option<Anchor>, rows: &[&Entry]) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    Some(anchor.map_or(0, |a| {
        rows.iter()
            .position(|e| e.tree == a.tree)
            .unwrap_or(a.at.min(rows.len() - 1))
    }))
}
fn trees(forest: &ForestProjection, index: &Index, query: &str) -> Value {
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
fn history(
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

const OUTLINE_MAX: usize = 40;
fn preview(forest: &ForestProjection, entry: &Entry, today: &days::Civil) -> Value {
    let tree = forest.trees().iter().find(|t| t.id == entry.tree).unwrap();
    let mut outline = Vec::new();
    let mut stack = vec![(tree.id.root(), 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let v = forest.visit(id).unwrap();
        outline.push(json!({"id":id.to_string(), "depth":depth, "url":v.url.as_str(), "name":super::graph::short_name(forest,id), "last":id==tree.last_visit, "waiting":forest.is_waiting(id)}));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Browser, Lifecycle, Snapshot, TreeState, VisitId};
    fn fixture(count: usize, pages: usize) -> ChromeFrame {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(&dir.path().join("db")).unwrap();
        let id = b
            .add_roots(&[url::Url::parse("https://one.example/root").unwrap()])
            .unwrap()[0];
        let template = b.visit(id).unwrap();
        let mut visits = vec![];
        let mut states = vec![];
        let id =
            |n: usize| -> VisitId { ulid::Ulid::from(n as u128 + 1).to_string().parse().unwrap() };
        for t in 0..count {
            let root = id(t * pages);
            for p in 0..pages {
                let mut v = template.clone();
                v.id = id(t * pages + p);
                v.parent = (p > 0).then(|| id(t * pages + p - 1));
                v.created_at = (t * pages + p) as i64;
                v.title = format!("Page {t} {p}");
                v.url = url::Url::parse(&format!("https://site{}.example/{t}/{p}", p % 4)).unwrap();
                visits.push(v);
            }
            states.push(TreeState {
                root: TreeId(root),
                last_visit: id(t * pages + pages - 1),
                pin: None,
                last_event_at: Some(t as i64 * 86_400_000),
                name: Some(format!("Tree {t}")),
                lifecycle: if t == 0 {
                    Lifecycle::Open
                } else {
                    Lifecycle::Settled { since: 0 }
                },
            });
        }
        ChromeFrame {
            forest: ForestProjection::from_snapshot(
                Snapshot {
                    visits,
                    current: Some(id(pages - 1)),
                },
                states,
            )
            .unwrap(),
            mail: Default::default(),
            loaded: Default::default(),
            live: Default::default(),
            capture_enabled: true,
            trees_help: true,
            theme: crate::browser::ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            files: vec![],
        }
    }
    fn value(reply: Reply) -> Value {
        match reply {
            Reply::Show(v) => v,
            _ => panic!("Expected a paint"),
        }
    }
    fn rows(v: &Value) -> Vec<&Value> {
        v["history"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "row")
            .collect()
    }
    #[test]
    fn trees_follow_tabs_and_history_starts_first_and_steps() {
        let f = fixture(4, 4);
        let mut p = ForestPane::default();
        assert!(matches!(p.present(&f), Reply::Unchanged));
        let v = value(p.input(Input::Open(View::Trees), &f));
        assert_eq!(v["count"], "1 open · 0 saved · 3 in history");
        assert_eq!(
            v["trees"]["items"][1]["id"],
            f.forest.open()[0].id.root().to_string()
        );
        assert_eq!(v["trees"]["items"][1]["current"], true);
        assert!(matches!(p.present(&f), Reply::Unchanged));
        let v = value(p.input(Input::Show(View::History), &f));
        assert_eq!(rows(&v).len(), 3);
        assert_eq!(rows(&v)[0]["title"], "Tree 3");
        assert_eq!(rows(&v)[0]["selected"], true);
        let v = value(p.input(Input::Step(1), &f));
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
            resolve(
                Some(Anchor {
                    tree: rows[1].tree,
                    at: 1
                }),
                &[rows[0], rows[2]]
            ),
            Some(1)
        );
        assert_eq!(
            resolve(
                Some(Anchor {
                    tree: rows[2].tree,
                    at: 2
                }),
                &[rows[0]]
            ),
            Some(0)
        );
        assert_eq!(resolve(None, &[]), None);
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
    fn input_rejects_unknown_actions_and_malformed_ids() {
        for m in [
            json!({"do":"wat"}),
            json!({"do":"show","view":"wat"}),
            json!({"do":"select","tree":"bad"}),
            json!({"do":"step","by":0}),
        ] {
            assert!(Input::parse(&m).is_err());
        }
    }
    #[test]
    fn preview_uses_true_depth_preorder_and_caps_the_outline() {
        let f = fixture(2, 80);
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        let preview = &v["history"]["preview"];
        let outline = preview["outline"].as_array().unwrap();
        assert_eq!(outline.len(), OUTLINE_MAX);
        assert_eq!(outline[0]["depth"], 0);
        assert_eq!(outline[39]["depth"], 39);
        assert_eq!(preview["cut"], "40 more pages");
        assert_eq!(
            preview["actions"],
            json!([{"kind":"reopen","label":"Reopen"},{"kind":"save","label":"Save"}])
        );
    }
    fn change(frame: &mut ChromeFrame, mutate: impl FnOnce(&mut Vec<TreeState>)) {
        let mut visits = vec![];
        let mut states = vec![];
        for tree in frame.forest.trees() {
            let mut stack = vec![tree.id.root()];
            while let Some(id) = stack.pop() {
                visits.push(frame.forest.visit(id).unwrap().clone());
                stack.extend(frame.forest.children(id));
            }
            states.push(TreeState {
                root: tree.id,
                last_visit: tree.last_visit,
                pin: tree.pin,
                last_event_at: tree.last_event_at,
                name: Some(tree.title.clone()),
                lifecycle: tree.lifecycle,
            });
        }
        mutate(&mut states);
        frame.forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: frame.forest.current(),
            },
            states,
        )
        .unwrap();
    }
    #[test]
    fn saving_selected_history_keeps_it_settled_and_selects_the_successor() {
        let mut f = fixture(4, 4);
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        let selected = rows(&v)[0]["id"].as_str().unwrap().to_owned();
        let successor = rows(&v)[1]["id"].clone();
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(&dir.path().join("db")).unwrap();
        let root = b
            .add_roots(&[url::Url::parse("https://saved.example").unwrap()])
            .unwrap()[0];
        b.set_pinned(TreeId(root), true).unwrap();
        let pin = b.tree_states().unwrap()[0].pin;
        change(&mut f, |states| {
            states
                .iter_mut()
                .find(|s| s.root.root().to_string() == selected)
                .unwrap()
                .pin = pin
        });
        let v = value(p.present(&f));
        assert_eq!(rows(&v)[0]["id"], successor);
        assert_eq!(rows(&v)[0]["selected"], true);
        assert_eq!(v["count"], "1 open · 1 saved · 2 in history");
        assert_eq!(f.forest.settled().len(), 3);
        let v = value(p.input(Input::Show(View::Trees), &f));
        let saved = v["trees"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["key"].as_str().is_some_and(|s| s.starts_with("saved-")))
            .unwrap();
        assert_eq!(saved["id"], selected);
        assert!(saved["badge"].is_null());
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
        let base = super::super::view::frame(
            &f,
            &[],
            (1240., 900.),
            &tree_view::Workbench::default(),
            Default::default(),
        );
        assert!(base.get("forest").is_none());
        assert_eq!(base["visits"].as_object().unwrap().len(), 10);
        let v = value(p.input(Input::More { seen: 60 }, &f));
        assert_eq!(rows(&v).len(), 120);
        assert!(matches!(
            p.input(Input::More { seen: 60 }, &f),
            Reply::Unchanged
        ));
        for _ in 0..119 {
            p.input(Input::Step(1), &f);
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
        let v = value(p.input(Input::Find("TREE 2".into()), &f));
        assert_eq!(rows(&v).len(), 1);
        assert_eq!(rows(&v)[0]["title"], "Tree 2");
        assert_eq!(v["count"], "1 open · 0 saved · 3 in history");
        let v = value(p.input(Input::Find("site3".into()), &f));
        assert_eq!(rows(&v).len(), 3);
        let v = value(p.input(Input::Find("absent".into()), &f));
        assert_eq!(v["history"]["empty"], "No settled tree matches.");
        let v = value(p.input(Input::Escape, &f));
        assert_eq!(v["query"], "");
        assert_eq!(rows(&v).len(), 3);
        assert!(matches!(p.input(Input::Escape, &f), Reply::Close));
        p.input(Input::Find("unkept".into()), &f);
        p.input(Input::Close, &f);
        let v = value(p.input(Input::Open(View::Trees), &f));
        assert_eq!(v["query"], "");
        let v = value(p.input(Input::Find("absent".into()), &f));
        assert_eq!(v["trees"]["empty"], "No open or saved tree matches.");
    }
    #[test]
    fn forest_shortcuts_switch_to_history_and_otherwise_close() {
        let f = fixture(3, 1);
        let mut p = ForestPane::default();
        p.input(Input::Open(View::Trees), &f);
        let v = value(p.input(Input::Toggle(View::History), &f));
        assert_eq!(v["view"], "history");
        assert!(matches!(
            p.input(Input::Toggle(View::History), &f),
            Reply::Close
        ));
        assert!(matches!(
            p.input(Input::Toggle(View::Trees), &f),
            Reply::Close
        ));
        p.input(Input::Show(View::Trees), &f);
        assert!(matches!(
            p.input(Input::Toggle(View::Trees), &f),
            Reply::Close
        ));
    }
    #[test]
    fn history_dates_use_events_else_settle_and_day_counts_include_unloaded_rows() {
        let mut f = fixture(132, 1);
        change(&mut f, |states| {
            for s in states {
                s.last_event_at = None;
                if !s.lifecycle.is_open() {
                    s.lifecycle = crate::browser::Lifecycle::Settled {
                        since: 1_790_000_000_000,
                    };
                }
            }
        });
        let mut p = ForestPane::default();
        let v = value(p.input(Input::Open(View::History), &f));
        assert_eq!(rows(&v).len(), 60);
        assert_eq!(v["history"]["items"][0]["count"], "131 trees");
        let tree = f.forest.settled().last().unwrap().id;
        change(&mut f, |states| {
            states
                .iter_mut()
                .find(|s| s.root == tree)
                .unwrap()
                .last_event_at = Some(1_800_000_000_000)
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
