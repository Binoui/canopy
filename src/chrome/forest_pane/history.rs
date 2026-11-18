use super::super::{
    tree_view::{self, Fit},
    view::plural,
};
use super::index::Entry;
use super::*;
use crate::{
    browser::VisitId,
    forest::{ForestProjection, TreeSummary},
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(in crate::chrome) enum Input {
    Select(TreeId),
    Step(i8),
    Fold(bool),
    More { edge: TreeId, before: bool },
    Find(String),
    Site(String),
    Jump(String),
    DeleteDay(String),
    Page(VisitId),
}
impl Input {
    pub fn parse(m: &Value) -> Result<Option<Self>> {
        let text = |key: &str| -> Result<String> {
            Ok(m[key].as_str().context("Missing History input")?.to_owned())
        };
        Ok(Some(match m["do"].as_str() {
            Some("find") => Self::Find(text("query")?),
            Some("site") => Self::Site(text("site")?),
            Some("jump") => Self::Jump(text("bucket")?),
            Some("delete-day") => Self::DeleteDay(text("day")?),
            Some("page") => Self::Page(text("id")?.parse()?),
            Some("more") => Self::More {
                edge: TreeId(text("edge")?.parse()?),
                before: m["before"].as_bool().context("Missing window edge")?,
            },
            Some("select") => Self::Select(TreeId(text("tree")?.parse()?)),
            Some("fold") => Self::Fold(m["open"].as_bool().context("Missing fold direction")?),
            Some("step") => Self::Step(match m["by"].as_i64() {
                Some(-1) => -1,
                Some(1) => 1,
                _ => bail!("Unknown forest step"),
            }),
            _ => return Ok(None),
        }))
    }
}
pub(super) struct State {
    cache: index::Cache,
    query: String,
    site: Option<String>,
    anchor: Option<Anchor>,
    unfolded: Option<TreeId>,
    start: usize,
    end: usize,
    reveal: Option<(TreeId, u32)>,
}
#[derive(Clone, Copy)]
struct Anchor {
    tree: TreeId,
    at: usize,
}
impl Default for State {
    fn default() -> Self {
        Self {
            cache: index::Cache::default(),
            query: String::new(),
            site: None,
            anchor: None,
            unfolded: None,
            start: 0,
            end: PAGE,
            reveal: None,
        }
    }
}
impl State {
    pub fn index(&mut self, forest: &ForestProjection, calendar: &dyn Calendar) -> Index {
        Index::cached(forest, calendar, &mut self.cache)
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    fn reset(&mut self) {
        self.anchor = None;
        self.unfolded = None;
        self.start = 0;
        self.end = PAGE;
        self.reveal = None;
    }
    pub fn escape(&mut self) -> bool {
        let changed = !self.query.is_empty() || self.site.is_some();
        self.query.clear();
        self.site = None;
        if changed {
            self.reset();
        }
        changed
    }
    fn rows<'a>(&self, index: &'a Index, forest: &ForestProjection) -> Vec<&'a Entry> {
        let key = self.query.trim().to_lowercase();
        index
            .history
            .iter()
            .filter(|e| {
                self.site
                    .as_ref()
                    .is_none_or(|host| index.facts[&e.tree].sites.iter().any(|s| s.host == *host))
                    && (index.admits(e.tree, &key)
                        || index.page_hit(e.tree, &key, forest).is_some())
            })
            .collect()
    }
    pub fn input(
        &mut self,
        input: Input,
        index: &Index,
        forest: &ForestProjection,
        calendar: &dyn Calendar,
    ) -> Reply {
        match input {
            Input::Find(query) => {
                if query != self.query {
                    self.query = query;
                    self.reset();
                }
            }
            Input::Site(site) => {
                if !index
                    .facts
                    .values()
                    .any(|f| f.sites.iter().any(|s| s.host == site))
                {
                    return Reply::Unchanged;
                }
                self.site = (self.site.as_ref() != Some(&site)).then_some(site);
                self.reset();
            }
            Input::More { edge, before } => {
                let rows = self.rows(index, forest);
                if before && self.start > 0 && rows.get(self.start).is_some_and(|e| e.tree == edge)
                {
                    self.start = self.start.saturating_sub(PAGE);
                } else if !before
                    && self.end < rows.len()
                    && rows.get(self.end - 1).is_some_and(|e| e.tree == edge)
                {
                    self.end = self.end.saturating_add(PAGE);
                }
            }
            Input::Select(tree) => {
                let rows = self.rows(index, forest);
                if let Some(at) = rows
                    .iter()
                    .position(|e| e.tree == tree)
                    .filter(|&at| at >= self.start && at < self.end)
                {
                    self.anchor = Some(Anchor { tree, at });
                    self.unfolded = (self.unfolded != Some(tree)).then_some(tree);
                }
            }
            Input::Step(by) => {
                let rows = self.rows(index, forest);
                if let Some(at) = resolve(self.anchor, &rows) {
                    let at = if by < 0 {
                        at.saturating_sub(1)
                    } else {
                        (at + 1).min(rows.len() - 1)
                    };
                    self.start = self.start.min(at);
                    self.end = self.end.max(at + 1);
                    self.anchor = Some(Anchor {
                        tree: rows[at].tree,
                        at,
                    });
                }
            }
            Input::Fold(open) => {
                self.unfolded = if open {
                    self.selected(index, forest)
                } else {
                    None
                }
            }
            Input::Jump(bucket) => {
                let today = calendar.now();
                let rows = self.rows(index, forest);
                if let Some(at) = rows.iter().position(|e| e.at.bucket(&today).0 == bucket) {
                    let tree = rows[at].tree;
                    self.start = at;
                    self.end = at + PAGE;
                    self.anchor = Some(Anchor { tree, at });
                    self.unfolded = None;
                    self.reveal = Some((tree, self.reveal.map_or(0, |(_, n)| n.wrapping_add(1))));
                }
            }
            Input::DeleteDay(day) => {
                if self.query.trim().is_empty() && self.site.is_none() {
                    let trees: Vec<_> = index
                        .history
                        .iter()
                        .filter(|e| e.at.date == day)
                        .map(|e| e.tree)
                        .collect();
                    if !trees.is_empty() {
                        return Reply::Act(super::super::ChromeIntent::DeleteTrees(trees));
                    }
                }
            }
            Input::Page(id) => {
                if self
                    .unfolded
                    .is_some_and(|tree| forest.tree_of(id) == Some(tree))
                {
                    return Reply::Go(super::super::ChromeIntent::Restore(id));
                }
            }
        }
        Reply::Unchanged
    }
    pub fn selected(&self, index: &Index, forest: &ForestProjection) -> Option<TreeId> {
        let rows = self.rows(index, forest);
        resolve(self.anchor, &rows).map(|i| rows[i].tree)
    }
    pub fn value(
        &mut self,
        forest: &ForestProjection,
        index: &Index,
        calendar: &dyn Calendar,
    ) -> Value {
        let trace = std::time::Instant::now();
        let rows = self.rows(index, forest);
        self.anchor = resolve(self.anchor, &rows).map(|at| Anchor {
            tree: rows[at].tree,
            at,
        });
        self.start = self.start.min(rows.len().saturating_sub(1));
        if let Some(a) = self.anchor {
            self.start = self.start.min(a.at);
            self.end = self.end.max(a.at + 1);
        }
        let shown = &rows[self.start..self.end.min(rows.len())];
        if self
            .unfolded
            .is_some_and(|tree| !shown.iter().any(|e| e.tree == tree))
        {
            self.unfolded = None;
        }
        let today = calendar.now();
        let mut days = BTreeMap::<&str, usize>::new();
        for e in &rows {
            *days.entry(&e.at.date).or_default() += 1;
        }
        let key = self.query.trim().to_lowercase();
        let mut items = Vec::new();
        for run in shown.chunk_by(|a, b| a.at.date == b.at.date) {
            let e = run[0];
            let label = e.at.label(&today);
            let count = days[e.at.date.as_str()];
            items.push(json!({"kind":"day", "key":format!("day-{}",e.at.date), "label":label, "count":plural(count,"tree"),
                "delete":(key.is_empty() && self.site.is_none()).then(||json!({"day":e.at.date,"label":"Delete day","name":format!("Delete {} of {label}",plural(count,"tree"))}))}));
            for e in run {
                let tree = forest.tree(e.tree).unwrap();
                let facts = &index.facts[&e.tree];
                let (trail, more) = trail(
                    forest,
                    tree,
                    facts.pages(e.tree, forest),
                    index.page_hit(e.tree, &key, forest),
                    &key,
                );
                items.push(json!({"kind":"row","key":format!("row-{}",e.tree.root()),"id":e.tree.root().to_string(),"time":e.at.clock,
                    "title":tree.title,"titleRuns":segments(&tree.title,&key),"trail":trail,"more":more,
                    "mini":facts.mini.get_or_init(||tree_card::mini_value(forest,&tree_view::miniature(forest,tree,Fit::ROW))),
                    "meta":format!("{} · {}",tree_card::sites_line(&facts.sites,3),plural(tree.page_count,"page")),
                    "selected":self.anchor.is_some_and(|a|a.tree==e.tree),"unfolded":self.unfolded==Some(e.tree),
                    "name":format!("{}, {}, {}",tree.title,plural(tree.page_count,"page"),e.at.when(&today)),
                    "buttons":tree_card::buttons(tree,&[Act::Open,Act::Save,Act::Delete]),"menu":tree_card::menu(tree),
                    "outline":(self.unfolded==Some(e.tree)).then(||outline(forest,tree))}));
            }
        }
        let mut buckets: Vec<Value> = [
            ("today", "Today"),
            ("yesterday", "Yesterday"),
            ("week", "Earlier this week"),
            ("last-week", "Last week"),
        ]
        .into_iter()
        .map(|(key, label)| json!({"key":key,"label":label,"count":0,"disabled":true}))
        .collect();
        let mut sites = BTreeMap::<&str, (usize, &str)>::new();
        for e in &index.history {
            let (bucket, label) = e.at.bucket(&today);
            if let Some(b) = buckets.iter_mut().find(|b| b["key"] == bucket) {
                if b["count"] == 0 {
                    b["disabled"] = json!(!rows.iter().any(|r| r.at.bucket(&today).0 == bucket));
                }
                b["count"] = json!(b["count"].as_u64().unwrap() + 1);
            } else {
                buckets.push(json!({"key":bucket,"label":label,"count":1,"disabled":!rows.iter().any(|r|r.at.bucket(&today).0==bucket)}));
            }
            for s in &index.facts[&e.tree].sites {
                let entry = sites.entry(&s.host).or_insert((0, s.sample.as_str()));
                entry.0 += 1;
            }
        }
        let mut sites: Vec<_> = sites.into_iter().collect();
        sites.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
        let sites: Vec<_> = sites.into_iter().take(9).map(|(host,(count,icon))|json!({"site":host,"count":count,"icon":icon,"on":self.site.as_deref()==Some(host)})).collect();
        let edge = |before| {
            let e = if before { shown.first() } else { shown.last() }?;
            Some(
                json!({"edge":e.tree.root().to_string(),"before":before,"label":if before {"Show newer trees"} else {"Show older trees"}}),
            )
        };
        if std::env::var_os("CANOPY_HISTORY_TRACE").is_some() {
            eprintln!("history value {:?}", trace.elapsed());
        }
        json!({"items":items,"field":{"label":"Find a tree","placeholder":"Find a tree by name, site or page"},"query":self.query,"site":self.site,"count":format!("{} of {} trees",rows.len(),index.history.len()),
            "rail":{"whenLabel":"When","sitesLabel":"Sites","when":buckets,"sites":sites},"before":(self.start>0).then(||edge(true)).flatten(),"more":(self.end<rows.len()).then(||edge(false)).flatten(),
            "reveal":self.reveal.map(|(tree,n)|json!({"id":tree.root().to_string(),"n":n})),
            "empty":rows.is_empty().then_some(if index.history.is_empty(){"No settled trees yet. A tree you settle waits here."}else{"No settled tree matches."})})
    }
}
fn resolve(anchor: Option<Anchor>, rows: &[&Entry]) -> Option<usize> {
    (!rows.is_empty()).then(|| {
        anchor.map_or(0, |a| {
            rows.iter()
                .position(|e| e.tree == a.tree)
                .unwrap_or(a.at.min(rows.len() - 1))
        })
    })
}
fn trail(
    forest: &ForestProjection,
    tree: &TreeSummary,
    pages: &[VisitId],
    hit: Option<VisitId>,
    key: &str,
) -> (Value, Option<String>) {
    let mut shown: Vec<_> = pages.iter().skip(1).take(4).copied().collect();
    if let Some(hit) = hit.filter(|id| !shown.contains(id)) {
        shown.insert(0, hit);
        shown.truncate(4);
    }
    let remaining = pages.len() - 1 - shown.iter().filter(|&&id| id != tree.id.root()).count();
    let mut trail: Vec<_> = shown
        .iter()
        .map(|&id| {
            let short = super::super::graph::short_name(forest, id);
            let name = if hit == Some(id) && !short.to_lowercase().contains(key) {
                &forest.visit(id).unwrap().title
            } else {
                &short
            };
            segments(name, key)
        })
        .collect();
    if pages.len() == 1 {
        trail.push(segments("One page", ""));
    }
    (
        json!(trail),
        (remaining > 0).then(|| format!("+{remaining}")),
    )
}
fn segments(text: &str, key: &str) -> Value {
    if key.is_empty() {
        return json!([{"t":text}]);
    }
    let lower = text.to_lowercase();
    let hits: Vec<_> = lower
        .match_indices(key)
        .map(|(i, s)| i..i + s.len())
        .collect();
    let mut runs = Vec::<Value>::new();
    let mut offset = 0;
    for c in text.chars() {
        let end = offset + c.to_lowercase().map(char::len_utf8).sum::<usize>();
        let marked = hits.iter().any(|h| h.start < end && h.end > offset);
        if let Some(last) = runs
            .last_mut()
            .filter(|v| v["m"].as_bool().unwrap_or(false) == marked)
        {
            last["t"] = json!(format!("{}{c}", last["t"].as_str().unwrap()));
        } else {
            runs.push(json!({"t":c.to_string(),"m":marked}));
        }
        offset = end;
    }
    json!(runs)
}
const OUTLINE_MAX: usize = 40;
fn outline(forest: &ForestProjection, tree: &TreeSummary) -> Value {
    let mut rows = Vec::new();
    let mut stack = vec![(tree.id.root(), 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let name = super::super::graph::short_name(forest, id);
        rows.push(json!({"id":id.to_string(),"depth":depth,"url":forest.visit(id).unwrap().url.as_str(),"name":name,
            "last":id==tree.last_visit,"waiting":forest.is_waiting(id),"label":format!("{name}, depth {depth}{}",if id==tree.last_visit{", last page"}else{""})}));
        if rows.len() == OUTLINE_MAX {
            break;
        }
        stack.extend(forest.children(id).iter().rev().map(|&c| (c, depth + 1)));
    }
    json!({"cut":(tree.page_count>rows.len()).then(||format!("{} more pages",tree.page_count-rows.len())),"rows":rows})
}

#[cfg(test)]
mod tests {
    use super::super::tests::{change, fixture, rows, value};
    use super::*;
    fn input(p: &mut ForestPane, i: Input, f: &ChromeFrame) -> Reply {
        p.input(super::super::Input::History(i), f)
    }
    fn open(p: &mut ForestPane, f: &ChromeFrame) -> Value {
        value(p.input(super::super::Input::Open(View::History), f))
    }
    fn id(v: &Value, n: usize) -> TreeId {
        TreeId(rows(v)[n]["id"].as_str().unwrap().parse().unwrap())
    }
    #[test]
    fn rows_use_settle_day_trails_and_no_preview() {
        let mut f = fixture(132, 8);
        change(&mut f, |states| {
            for s in states {
                if !s.place().is_tab() {
                    s.spot = crate::browser::Spot::History {
                        settled_at: 1_790_000_000_000,
                    };
                }
            }
        });
        let mut p = ForestPane::default();
        let v = open(&mut p, &f);
        assert_eq!(rows(&v).len(), 60);
        assert_eq!(v["history"]["items"][0]["count"], "131 trees");
        assert_eq!(rows(&v)[0]["trail"].as_array().unwrap().len(), 4);
        assert_eq!(rows(&v)[0]["more"], "+3");
        assert!(v["history"].get("preview").is_none());
        assert!(!rows(&v)[0]["mini"]["nodes"].as_array().unwrap().is_empty());
    }
    #[test]
    fn click_toggles_one_outline_and_steps_only_select() {
        let f = fixture(4, 80);
        let mut p = ForestPane::default();
        let v = open(&mut p, &f);
        let a = id(&v, 0);
        let b = id(&v, 1);
        let v = value(input(&mut p, Input::Select(a), &f));
        let o = &rows(&v)[0]["outline"];
        assert_eq!(o["rows"].as_array().unwrap().len(), 40);
        assert_eq!(o["rows"][39]["depth"], 39);
        assert_eq!(o["cut"], "40 more pages");
        let v = value(input(&mut p, Input::Step(1), &f));
        assert_eq!(rows(&v)[1]["selected"], true);
        assert_eq!(rows(&v)[0]["unfolded"], true);
        assert_eq!(rows(&v)[1]["unfolded"], false);
        let v = value(input(&mut p, Input::Fold(true), &f));
        assert_eq!(rows(&v)[1]["unfolded"], true);
        assert_eq!(rows(&v)[0]["unfolded"], false);
        let v = value(input(&mut p, Input::Select(b), &f));
        assert_eq!(rows(&v)[1]["unfolded"], false);
    }
    #[test]
    fn find_matches_page_root_sites_titles_and_promotes_hidden_hit() {
        let f = fixture(4, 8);
        let mut p = ForestPane::default();
        open(&mut p, &f);
        let v = value(input(&mut p, Input::Find("Page 2 7".into()), &f));
        assert_eq!(rows(&v).len(), 1);
        assert_eq!(rows(&v)[0]["trail"][0], json!([{"t":"Page 2 7","m":true}]));
        assert_eq!(rows(&v)[0]["more"], "+3");
        let v = value(input(&mut p, Input::Find("Page 2 0".into()), &f));
        assert_eq!(rows(&v).len(), 1);
        assert_eq!(rows(&v)[0]["trail"][0], json!([{"t":"Page 2 0","m":true}]));
        assert_eq!(rows(&v)[0]["more"], "+4");
        let v = value(input(&mut p, Input::Find("TREE 2".into()), &f));
        assert_eq!(rows(&v)[0]["titleRuns"], json!([{"t":"Tree 2","m":true}]));
        let v = value(input(&mut p, Input::Find("site3".into()), &f));
        assert_eq!(rows(&v).len(), 3);
        assert_eq!(
            segments("İ<&É", "i\u{307}"),
            json!([{"t":"İ","m":true},{"t":"<&É","m":false}])
        );
    }
    #[test]
    fn root_only_trail_and_empty_history() {
        let f = fixture(2, 1);
        let mut p = ForestPane::default();
        let v = open(&mut p, &f);
        assert_eq!(rows(&v)[0]["trail"], json!([[{"t":"One page"}]]));
        let f = fixture(1, 1);
        let v = value(p.present(&f));
        assert!(rows(&v).is_empty());
        assert!(v["history"]["empty"]
            .as_str()
            .unwrap()
            .contains("No settled trees"));
    }
    #[test]
    fn site_toggle_escape_and_delete_day_guard() {
        let f = fixture(4, 4);
        let mut p = ForestPane::default();
        let v = open(&mut p, &f);
        let day = v["history"]["items"][0]["delete"]["day"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            matches!(input(&mut p,Input::DeleteDay(day.clone()),&f),Reply::Act(super::super::super::ChromeIntent::DeleteTrees(t)) if t.len()==1)
        );
        let v = value(input(&mut p, Input::Site("site1.example".into()), &f));
        assert!(v["history"]["items"][0]["delete"].is_null());
        assert!(matches!(
            input(&mut p, Input::DeleteDay(day.clone()), &f),
            Reply::Unchanged
        ));
        let v = value(input(&mut p, Input::Site("site1.example".into()), &f));
        assert!(v["history"]["site"].is_null());
        input(&mut p, Input::Find("absent".into()), &f);
        input(&mut p, Input::Site("site1.example".into()), &f);
        let v = value(p.input(super::super::Input::Escape, &f));
        assert_eq!(rows(&v).len(), 3);
        assert!(v["history"]["site"].is_null());
        assert!(matches!(
            p.input(super::super::Input::Escape, &f),
            Reply::Close
        ));
        let v = value(input(&mut p, Input::Find("Tree".into()), &f));
        assert!(v["history"]["items"][0]["delete"].is_null());
        assert!(matches!(
            input(&mut p, Input::DeleteDay(day), &f),
            Reply::Unchanged
        ));
    }
    #[test]
    fn jump_replaces_window_and_loads_both_edges_without_trimming() {
        let f = fixture(2001, 10);
        let index = Index::build(&f.forest, &LocalCalendar);
        let mut state = State::default();
        let target = &index.history[1500];
        let bucket = target.at.bucket(&LocalCalendar.now()).0;
        state.input(
            Input::Jump(bucket.clone()),
            &index,
            &f.forest,
            &LocalCalendar,
        );
        let first = index
            .history
            .iter()
            .position(|e| e.at.bucket(&LocalCalendar.now()).0 == bucket)
            .unwrap();
        assert_eq!(state.start, first);
        assert_eq!(state.end, first + 60);
        let edge = index.history[first].tree;
        state.input(
            Input::More { edge, before: true },
            &index,
            &f.forest,
            &LocalCalendar,
        );
        assert_eq!(state.start, first - 60);
        state.input(
            Input::More { edge, before: true },
            &index,
            &f.forest,
            &LocalCalendar,
        );
        assert_eq!(state.start, first - 60);
        let edge = index.history[state.end - 1].tree;
        state.input(
            Input::More {
                edge,
                before: false,
            },
            &index,
            &f.forest,
            &LocalCalendar,
        );
        assert_eq!(state.end, first + 120);
    }
    #[test]
    fn successor_and_top_sites_count_each_tree_once() {
        let mut f = fixture(4, 20);
        let mut p = ForestPane::default();
        let v = open(&mut p, &f);
        let a = id(&v, 0);
        let b = id(&v, 1);
        assert_eq!(v["history"]["rail"]["sites"].as_array().unwrap().len(), 4);
        assert_eq!(v["history"]["rail"]["sites"][0]["count"], 3);
        change(&mut f, |states| {
            states.iter_mut().find(|s| s.root == a).unwrap().spot = crate::browser::Spot::Shelf {
                position: 0,
                saved_at: 1,
            }
        });
        let v = value(p.present(&f));
        assert_eq!(id(&v, 0), b);
        assert_eq!(rows(&v)[0]["selected"], true);
    }
    #[test]
    fn page_activation_requires_the_unfolded_tree() {
        let f = fixture(3, 4);
        let index = Index::build(&f.forest, &LocalCalendar);
        let mut state = State::default();
        let tree = index.history[0].tree;
        let page = index.facts[&tree].pages(tree, &f.forest)[1];
        assert!(matches!(
            state.input(Input::Page(page), &index, &f.forest, &LocalCalendar),
            Reply::Unchanged
        ));
        state.input(Input::Fold(true), &index, &f.forest, &LocalCalendar);
        assert!(
            matches!(state.input(Input::Page(page),&index,&f.forest,&LocalCalendar),Reply::Go(super::super::super::ChromeIntent::Restore(id)) if id==page)
        );
    }
    #[test]
    fn top_nine_sites_and_their_toggle_use_tree_counts() {
        use crate::browser::{Place, Snapshot, Spot, TreeState};
        let mut f = fixture(13, 1);
        let visits = f
            .forest
            .trees()
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut visit = f.forest.visit(t.id.root()).unwrap().clone();
                visit.url = url::Url::parse(&format!("https://site{i:02}.example/")).unwrap();
                visit
            })
            .collect();
        let states = f
            .forest
            .trees()
            .iter()
            .map(|t| TreeState {
                root: t.id,
                last_visit: t.last_visit,
                name: Some(t.title.clone()),
                last_event_at: t.last_event_at,
                spot: match t.place {
                    Place::Tab { pinned } => Spot::Tab {
                        pinned,
                        position: 0,
                    },
                    Place::History { settled_at } => Spot::History { settled_at },
                    _ => unreachable!(),
                },
            })
            .collect();
        f.forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: f.forest.current(),
            },
            states,
        )
        .unwrap();
        let mut p = ForestPane::default();
        let v = open(&mut p, &f);
        let sites = v["history"]["rail"]["sites"].as_array().unwrap();
        assert_eq!(sites.len(), 9);
        assert!(sites.iter().all(|s| s["count"] == 1));
        assert_eq!(sites[0]["site"], "site01.example");
        assert_eq!(sites[8]["site"], "site09.example");
        let v = value(input(&mut p, Input::Site("site01.example".into()), &f));
        assert_eq!(rows(&v).len(), 1);
        assert_eq!(v["history"]["rail"]["sites"][0]["on"], true);
        let v = value(input(&mut p, Input::Site("site01.example".into()), &f));
        assert_eq!(rows(&v).len(), 12);
    }
    #[test]
    fn selected_open_save_and_delete_follow_the_keyboard_selection() {
        let f = fixture(4, 4);
        let mut p = ForestPane::default();
        let v = open(&mut p, &f);
        let first = id(&v, 0);
        input(&mut p, Input::Step(1), &f);
        input(&mut p, Input::Fold(true), &f);
        let v = value(input(&mut p, Input::Fold(false), &f));
        assert!(rows(&v)[1]["outline"].is_null());
        input(&mut p, Input::Step(-1), &f);
        let act = |act| super::super::Input::Act { tree: None, act };
        assert!(
            matches!(p.input(act(Act::Open),&f),Reply::Go(super::super::super::ChromeIntent::SelectTree(tree)) if tree==first)
        );
        assert!(
            matches!(p.input(act(Act::Save),&f),Reply::Act(super::super::super::ChromeIntent::PlaceTree{tree,to:crate::browser::Move::Shelf(crate::browser::Slot::First)}) if tree==first)
        );
        assert!(
            matches!(p.input(act(Act::Delete),&f),Reply::Act(super::super::super::ChromeIntent::DeleteTrees(trees)) if trees==vec![first])
        );
    }
}
