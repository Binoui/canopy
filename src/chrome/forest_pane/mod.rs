const PAGE: usize = 60;

mod days;
mod history;
mod index;
mod trees;

use super::{
    tree_card::{self, Act},
    ChromeFrame,
};
use crate::browser::TreeId;
use anyhow::{bail, Context, Result};
use days::{Calendar, LocalCalendar};
use index::Index;
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
    Trees(trees::Input),
    History(history::Input),
    Escape,
    Toggle(View),
    Act { tree: Option<TreeId>, act: Act },
    Undo(crate::browser::DeleteId),
}
impl Input {
    pub fn parse(m: &Value) -> Result<Self> {
        let view = || match m["view"].as_str() {
            Some("trees") => Ok(View::Trees),
            Some("history") => Ok(View::History),
            _ => bail!("Unknown forest view"),
        };
        Ok(match m["do"].as_str() {
            Some("act") => Self::Act {
                tree: m["tree"]
                    .as_str()
                    .map(|s| s.parse().map(TreeId))
                    .transpose()?,
                act: Act::parse(m["act"].as_str().unwrap_or_default())
                    .context("Unknown tree action")?,
            },
            Some("undo") => Self::Undo(
                m["id"]
                    .as_str()
                    .context("Missing delete identity")?
                    .parse()?,
            ),
            Some("open") => Self::Open(view()?),
            Some("close") => Self::Close,
            Some("show") => Self::Show(view()?),
            Some("toggle") => Self::Toggle(view()?),
            Some("escape") => Self::Escape,
            _ => {
                if let Some(i) = trees::Input::parse(m)? {
                    Self::Trees(i)
                } else if let Some(i) = history::Input::parse(m)? {
                    Self::History(i)
                } else {
                    bail!("Unknown forest input")
                }
            }
        })
    }
}
pub(super) enum Reply {
    Unchanged,
    Show(Value),
    Close,
    Act(super::ChromeIntent),
    Go(super::ChromeIntent),
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
    shown: Shown,
    sent: Option<Value>,
    undo: Option<(crate::browser::DeleteId, String)>,
}
enum Shown {
    Trees(trees::State),
    History(history::State),
}
impl Shown {
    fn new(view: View) -> Self {
        match view {
            View::Trees => Self::Trees(trees::State::default()),
            View::History => Self::History(history::State::default()),
        }
    }
    fn query(&self) -> &str {
        match self {
            Self::Trees(t) => t.query(),
            Self::History(h) => h.query(),
        }
    }
    fn view(&self) -> View {
        match self {
            Self::Trees(_) => View::Trees,
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
        if let Input::Act { tree, act } = input {
            let selected = match &self.state {
                State::Open(Open {
                    shown: Shown::History(c),
                    ..
                }) => c.selected(&Index::build(&frame.forest, &*self.calendar)),
                _ => None,
            };
            return tree
                .or(selected)
                .and_then(|id| frame.forest.tree(id))
                .and_then(|tree| tree_card::intent(tree, act))
                .map_or(Reply::Unchanged, |intent| {
                    if act == Act::Open {
                        Reply::Go(intent)
                    } else {
                        Reply::Act(intent)
                    }
                });
        }
        if let Input::Undo(id) = input {
            return Reply::Act(super::ChromeIntent::UndoDelete(id));
        }
        match input {
            Input::Close => {
                self.state = State::Closed;
                return Reply::Act(super::ChromeIntent::ForgetDeleted);
            }
            Input::Open(view) if matches!(self.state, State::Closed) => {
                self.state = State::Open(Open {
                    shown: Shown::new(view),
                    sent: None,
                    undo: None,
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
                            (View::History, Shown::Trees(_)) => {
                                open.shown = Shown::new(View::History)
                            }
                            _ => return Reply::Close,
                        },
                        Input::Escape => {
                            if !match &mut open.shown {
                                Shown::Trees(t) => t.escape(),
                                Shown::History(h) => h.escape(),
                            } {
                                return Reply::Close;
                            }
                        }
                        Input::Trees(i) => {
                            if let Shown::Trees(t) = &mut open.shown {
                                let reply = t.input(i, &frame.forest, &*self.calendar);
                                if !matches!(reply, Reply::Unchanged) {
                                    return reply;
                                }
                            }
                        }
                        Input::History(i) => {
                            if let Shown::History(h) = &mut open.shown {
                                let index = Index::build(&frame.forest, &*self.calendar);
                                let reply = h.input(i, &index, &frame.forest, &*self.calendar);
                                if !matches!(reply, Reply::Unchanged) {
                                    return reply;
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
    pub fn is_open(&self) -> bool {
        matches!(self.state, State::Open(_))
    }
    pub fn deleted(&mut self, id: Option<crate::browser::DeleteId>, text: String) {
        if let State::Open(open) = &mut self.state {
            open.undo = id.map(|id| (id, text));
        }
    }
    pub fn present(&mut self, frame: &ChromeFrame) -> Reply {
        let State::Open(open) = &mut self.state else {
            return Reply::Unchanged;
        };
        let index = Index::build(&frame.forest, &*self.calendar);
        let value = json!({ "view": open.shown.view().name(), "query": open.shown.query(),
            "undo":open.undo.as_ref().map(|(id,text)|json!({"id":id.to_string(),"text":text,"label":"Undo"})),
            "count": format!("{} tabs · {} saved for later · {} in history", frame.forest.tabs().len(), frame.forest.shelf().len(), index.history.len()),
            "trees": match &open.shown {Shown::Trees(t)=>Some(t.value(&frame.forest,&index,&*self.calendar)),_=>None},
            "history": match &mut open.shown { Shown::History(c) => Some(c.value(&frame.forest, &index, &*self.calendar)), _ => None } });
        if open.sent.as_ref() == Some(&value) {
            return Reply::Unchanged;
        }
        open.sent = Some(value.clone());
        Reply::Show(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Browser, Snapshot, Spot, TreeState, VisitId};
    use crate::forest::ForestProjection;
    pub(super) fn fixture(count: usize, pages: usize) -> ChromeFrame {
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

                last_event_at: Some(t as i64 * 86_400_000),
                name: Some(format!("Tree {t}")),
                spot: if t == 0 {
                    crate::browser::Spot::Tab {
                        pinned: false,
                        position: 0,
                    }
                } else {
                    crate::browser::Spot::History {
                        settled_at: t as i64 * 86_400_000,
                    }
                },
            });
        }
        if states.len() > 1 {
            states[1..].reverse();
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
            pending: None,
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
    pub(super) fn value(reply: Reply) -> Value {
        match reply {
            Reply::Show(v) => v,
            _ => panic!("Expected a paint"),
        }
    }
    pub(super) fn rows(v: &Value) -> Vec<&Value> {
        v["history"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "row")
            .collect()
    }
    pub(super) fn change(frame: &mut ChromeFrame, mutate: impl FnOnce(&mut Vec<TreeState>)) {
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
                last_event_at: tree.last_event_at,
                name: Some(tree.title.clone()),
                spot: match tree.place {
                    crate::browser::Place::Tab { pinned } => Spot::Tab {
                        pinned,
                        position: 0,
                    },
                    crate::browser::Place::Shelf { saved_at } => Spot::Shelf {
                        saved_at,
                        position: 0,
                    },
                    crate::browser::Place::History { settled_at } => Spot::History { settled_at },
                },
            });
        }
        mutate(&mut states);
        states.sort_by_key(|s| s.spot.order());
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
}
