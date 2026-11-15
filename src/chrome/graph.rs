use std::collections::{HashMap, HashSet};

use crate::{
    browser::{VisitId, VisitState},
    forest::ForestProjection,
};

pub(crate) const REST_HEIGHT: f64 = 52.;
pub(crate) const OPEN_HEIGHT: f64 = REST_HEIGHT / 2. + 94.;
pub(crate) const TRUNK_Y: f64 = REST_HEIGHT / 2.;
const PAD: f64 = 22.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeKind {
    Pill,
    Tile,
    Raw,
    Bud,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub s: f64,
    pub w: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlacedNode {
    pub id: VisitId,
    pub rect: Rect,
    pub kind: NodeKind,
    pub on_path: bool,
    pub hidden: Vec<VisitId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EdgeKind {
    Path,
    Pending,
    Plain,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Edge {
    pub from: VisitId,
    pub to: VisitId,
    pub kind: EdgeKind,
    pub from_bottom: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StripLayout {
    pub nodes: Vec<PlacedNode>,
    pub edges: Vec<Edge>,
    pub width: f64,
    pub height: f64,
    pub focus: f64,
}

impl StripLayout {
    pub fn node(&self, id: VisitId) -> Option<&PlacedNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// The trunk in reading order: ancestors, the current page, then its continuation.
    pub fn trunk(&self) -> Vec<VisitId> {
        let mut trunk: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| (node.rect.y - TRUNK_Y).abs() < 0.01)
            .collect();
        trunk.sort_by(|a, b| a.rect.x.total_cmp(&b.rect.x));
        trunk.into_iter().map(|node| node.id).collect()
    }
}

/// Port of the prototype's `layoutB(F, W, {pill: true, down: true, rest: 52})`.
/// The trunk keeps identical x coordinates at rest and open, so hovering never shifts it.
pub(crate) fn strip_layout(
    forest: &ForestProjection,
    current: VisitId,
    width: f64,
    open: bool,
    pill_width: f64,
) -> StripLayout {
    let path = forest.path(current);
    let Some(k) = path.len().checked_sub(1) else {
        return StripLayout::default();
    };
    let extra = pill_width - 32.;
    let half = ((width - 2. * PAD - 40. - extra) / (2 * k + 10) as f64).clamp(23., 36.);
    let x = |u: usize| PAD + 16. + u as f64 * half + if u > 2 * k { extra } else { 0. };
    let offsets: &[f64] = if open {
        &[0., 27., 46., 65., 84.]
    } else {
        &[0., REST_HEIGHT * 0.27, REST_HEIGHT * 0.42]
    };
    let bands = if open { 4 } else { 2 };
    let mut layout = Placement::default();
    for (i, &id) in path.iter().enumerate() {
        let rect = if i == k {
            Rect {
                x: x(2 * k) + extra / 2.,
                y: TRUNK_Y,
                s: if open { 32. } else { 30. },
                w: pill_width,
            }
        } else {
            square(x(2 * i), TRUNK_Y, if open { 28. } else { 26. })
        };
        let kind = if i == k {
            NodeKind::Pill
        } else {
            NodeKind::Tile
        };
        layout.place(id, rect, kind, true);
        if i > 0 {
            layout.edge(path[i - 1], id, EdgeKind::Path, false);
        }
    }

    let mut taken: HashSet<(usize, isize)> = HashSet::new();
    let take = |taken: &mut HashSet<(usize, isize)>, band: usize, u: usize| {
        taken.insert((band, u as isize - 1));
        taken.insert((band, u as isize));
    };
    let mut forward = forest.children(current).to_vec();
    forward.sort_by(|a, b| {
        let (a, b) = (forest.visit(*a).unwrap(), forest.visit(*b).unwrap());
        (a.state == VisitState::Queued)
            .cmp(&(b.state == VisitState::Queued))
            .then(b.created_at.cmp(&a.created_at))
    });
    let mut overflow: HashMap<VisitId, Vec<VisitId>> = HashMap::new();
    for (band, &child) in forward.iter().enumerate() {
        if band > bands {
            if open {
                overflow
                    .entry(current)
                    .or_default()
                    .extend(subtree(forest, child));
            }
            continue;
        }
        let mut chain = vec![child];
        while chain.len() < 4 {
            match forest.children(*chain.last().unwrap()).last() {
                Some(&next) => chain.push(next),
                None => break,
            }
        }
        for (r, &id) in chain.iter().enumerate() {
            let u = 2 * k + 2 + 2 * r;
            take(&mut taken, band, u);
            let (size, kind) = match (open, r) {
                (true, 0) => (22., NodeKind::Tile),
                (true, _) => (14., NodeKind::Raw),
                (false, _) if band == 0 && r == 0 => (10., NodeKind::Bud),
                (false, _) => (7., NodeKind::Bud),
            };
            layout.place(id, square(x(u), TRUNK_Y + offsets[band], size), kind, false);
            let from = if r == 0 { current } else { chain[r - 1] };
            layout.edge(from, id, edge_kind(forest, id), false);
        }
        if open {
            let shown: HashSet<_> = chain.iter().copied().collect();
            let hidden: Vec<_> = subtree(forest, child)
                .into_iter()
                .filter(|id| !shown.contains(id))
                .collect();
            if !hidden.is_empty() {
                overflow.insert(*chain.last().unwrap(), hidden);
            }
        }
    }

    for i in (0..k).rev() {
        let mut alternatives: Vec<_> = forest
            .children(path[i])
            .iter()
            .copied()
            .filter(|&child| child != path[i + 1])
            .collect();
        alternatives.sort_by_key(|id| std::cmp::Reverse(forest.visit(*id).unwrap().created_at));
        let u0 = 2 * i + 1;
        for branch in alternatives {
            let rows = rows(forest, branch);
            let mut best: Option<(isize, Vec<PlannedRow>)> = None;
            for band in 1..=bands {
                for row_count in (1..=rows.len().min(bands - band + 1)).rev() {
                    let mut plan: Vec<PlannedRow> = vec![];
                    let mut fits = true;
                    for (r, row) in rows.iter().enumerate().take(row_count) {
                        let row_band = band + r;
                        let parent_shown = plan.iter().any(|planned| {
                            row.parent.is_some_and(|p| {
                                rows[planned.row].ids[..planned.shown].contains(&p)
                            })
                        });
                        if r > 0 && !parent_shown {
                            break;
                        }
                        let mut n = 0;
                        while n < row.ids.len() {
                            let u = (u0 + 2 * (row.column + n)) as isize;
                            if taken.contains(&(row_band, u - 1)) || taken.contains(&(row_band, u))
                            {
                                break;
                            }
                            n += 1;
                        }
                        if n == 0 {
                            fits = r > 0;
                            break;
                        }
                        plan.push(PlannedRow {
                            row: r,
                            band: row_band,
                            shown: n,
                        });
                    }
                    if !fits || plan.is_empty() {
                        continue;
                    }
                    let score =
                        plan.iter().map(|p| p.shown as isize).sum::<isize>() * 10 - band as isize;
                    if best.as_ref().is_none_or(|(best, _)| score > *best) {
                        best = Some((score, plan));
                    }
                }
            }
            let Some((_, plan)) = best else {
                if open {
                    overflow
                        .entry(path[i])
                        .or_default()
                        .extend(subtree(forest, branch));
                }
                continue;
            };
            let mut shown = HashSet::new();
            for PlannedRow {
                row,
                band,
                shown: n,
            } in plan
            {
                let row = &rows[row];
                for (m, &id) in row.ids[..n].iter().enumerate() {
                    let column = row.column + m;
                    let u = u0 + 2 * column;
                    take(&mut taken, band, u);
                    let (size, kind) = match (open, column) {
                        (true, 0) => (20., NodeKind::Tile),
                        (true, _) => (14., NodeKind::Raw),
                        (false, 0) => (8., NodeKind::Bud),
                        (false, _) => (6., NodeKind::Bud),
                    };
                    layout.place(id, square(x(u), TRUNK_Y + offsets[band], size), kind, false);
                    let from_trunk = m == 0 && row.parent.is_none();
                    let from = if m > 0 {
                        row.ids[m - 1]
                    } else {
                        row.parent.unwrap_or(path[i])
                    };
                    layout.edge(from, id, edge_kind(forest, id), from_trunk);
                    shown.insert(id);
                }
            }
            if open {
                let hidden: Vec<_> = subtree(forest, branch)
                    .into_iter()
                    .filter(|id| !shown.contains(id))
                    .collect();
                if !hidden.is_empty() {
                    overflow.insert(branch, hidden);
                }
            }
        }
    }

    for (id, hidden) in overflow {
        if let Some(&index) = layout.index.get(&id) {
            layout.nodes[index].hidden.extend(hidden);
        }
    }
    let width = layout
        .nodes
        .iter()
        .map(|node| node.rect.x)
        .fold(f64::MIN, f64::max)
        + PAD
        + 20.;
    StripLayout {
        nodes: layout.nodes,
        edges: layout.edges,
        width,
        height: if open { OPEN_HEIGHT } else { REST_HEIGHT },
        focus: x(2 * k),
    }
}

#[derive(Default)]
struct Placement {
    nodes: Vec<PlacedNode>,
    edges: Vec<Edge>,
    index: HashMap<VisitId, usize>,
}

impl Placement {
    fn place(&mut self, id: VisitId, rect: Rect, kind: NodeKind, on_path: bool) {
        self.index.insert(id, self.nodes.len());
        self.nodes.push(PlacedNode {
            id,
            rect,
            kind,
            on_path,
            hidden: Vec::new(),
        });
    }

    fn edge(&mut self, from: VisitId, to: VisitId, kind: EdgeKind, from_bottom: bool) {
        self.edges.push(Edge {
            from,
            to,
            kind,
            from_bottom,
        });
    }
}

fn square(x: f64, y: f64, s: f64) -> Rect {
    Rect { x, y, s, w: s }
}

pub(crate) fn edge_kind(forest: &ForestProjection, id: VisitId) -> EdgeKind {
    match forest.visit(id).map(|visit| &visit.state) {
        Some(VisitState::Queued) => EdgeKind::Pending,
        _ => EdgeKind::Plain,
    }
}

fn subtree(forest: &ForestProjection, root: VisitId) -> Vec<VisitId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend(forest.children(id).iter().rev());
    }
    out
}

/// How many nodes of row `row` of a side branch go into band `band`.
struct PlannedRow {
    row: usize,
    band: usize,
    shown: usize,
}

struct Row {
    ids: Vec<VisitId>,
    column: usize,
    parent: Option<VisitId>,
}

/// Port of the prototype's `rowsOf`: the newest child continues its parent's row,
/// older children start rows of their own. Iterative so deep histories cannot overflow.
fn rows(forest: &ForestProjection, root: VisitId) -> Vec<Row> {
    let mut out = vec![Row {
        ids: vec![],
        column: 0,
        parent: None,
    }];
    let mut stack = vec![(root, 0, Some(0), None)];
    while let Some((id, column, row, parent)) = stack.pop() {
        let row = row.unwrap_or_else(|| {
            out.push(Row {
                ids: vec![],
                column,
                parent,
            });
            out.len() - 1
        });
        out[row].ids.push(id);
        let mut children = forest.children(id).to_vec();
        children.sort_by_key(|id| std::cmp::Reverse(forest.visit(*id).unwrap().created_at));
        for (j, child) in children.into_iter().enumerate().rev() {
            if j == 0 {
                stack.push((child, column + 1, Some(row), None));
            } else {
                stack.push((child, column + 1, None, Some(id)));
            }
        }
    }
    out
}

/// The prototype's `edgeD`, as cubic Bézier control points.
pub(crate) fn edge_curve(edge: &Edge, a: Rect, b: Rect) -> [(f64, f64); 4] {
    let (ex, ey) = (b.x - b.w / 2., b.y);
    if edge.from_bottom {
        let (sx, sy) = (a.x, a.y + a.s / 2.);
        return [(sx, sy), (sx, ey), (sx + (ex - sx) * 0.3, ey), (ex, ey)];
    }
    let (sx, sy) = (a.x + a.w / 2., a.y);
    let dx = ((ex - sx) / 2.).max(5.);
    [(sx, sy), (sx + dx, sy), (ex - dx, ey), (ex, ey)]
}

/// Horizontal pan that keeps the current page at two thirds of the strip, as the prototype does.
pub(crate) fn pan(layout: &StripLayout, strip_width: f64) -> f64 {
    if layout.width <= strip_width {
        return 0.;
    }
    (strip_width * 0.66 - layout.focus)
        .min(0.)
        .max(strip_width - layout.width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, TreeState, Visit};
    use javascriptcore::{ContextExt, ValueExt};
    use serde_json::Value;
    use url::Url;

    fn fixture() -> (ForestProjection, Vec<VisitId>, Value) {
        let json: Value =
            serde_json::from_str(include_str!("testdata/strip-fixture.json")).unwrap();
        let entries = json.as_array().unwrap();
        let ids: Vec<VisitId> = entries
            .iter()
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect();
        let index = |key: &Value| entries.iter().position(|e| &e["id"] == key);
        let visits: Vec<Visit> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let created_at = (e["at"].as_f64().unwrap() * 60_000.) as i64;
                let pending = e["pending"].as_bool().unwrap();
                Visit {
                    id: ids[i],
                    parent: index(&e["parent"]).map(|p| ids[p]),
                    url: Url::parse(&format!("https://{}/{i}", e["host"].as_str().unwrap()))
                        .unwrap(),
                    title: e["title"].as_str().unwrap().into(),
                    created_at,
                    opened_at: (!pending).then_some(created_at),
                    state: if pending {
                        VisitState::Queued
                    } else {
                        VisitState::Opened
                    },
                    source: None,
                }
            })
            .collect();
        let states = visits
            .iter()
            .filter(|v| v.parent.is_none())
            .map(|v| TreeState {
                pin: None,
                last_event_at: None,
                lifecycle: crate::browser::Lifecycle::Open,
                root: crate::browser::TreeId(v.id),
                last_visit: v.id,
                name: None,
            })
            .collect();
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: None,
            },
            states,
        )
        .unwrap();
        (forest, ids, json)
    }

    #[test]
    fn layout_matches_the_prototype_for_every_current_page() {
        let (forest, ids, json) = fixture();
        let script = include_str!("testdata/layout-b.js");
        let mut cases = 0;
        for width in [800., 1240.] {
            for open in [false, true] {
                for (current, &id) in ids.iter().enumerate() {
                    if forest.visit(id).unwrap().state == VisitState::Queued {
                        continue;
                    }
                    let rust = strip_layout(&forest, id, width, open, 250.);
                    let reference = format!(
                        r#"const ns={json}; ns.forEach(n=>{{n.state=n.pending?"pending":"opened";n.by="me";}});
                        const current={current}; const SITE={{}},PF='Inter'; const tw=()=>203;
                        const mk=(F,id,x,y,s,o={{}})=>({{key:id,x,y,s,w:o.w||s,cls:o.cls||[],...o}}); const ecls=()=>'';
                        const get=id=>ns.find(n=>n.id===id); const kids=id=>ns.filter(n=>n.parent===id);
                        const count=id=>kids(id).reduce((s,n)=>s+1+count(n.id),0);
                        const F={{get,dkids:kids,count,path:()=>{{let p=[ns[current].id];while(get(p[0]).parent)p.unshift(get(p[0]).parent);return p;}}}};
                        {script}
                        JSON.stringify(layoutB(F,{width},{{open:{open},pill:true,down:true,rest:52}}))"#
                    );
                    let context = javascriptcore::Context::new();
                    let value = context.evaluate(&reference).expect("prototype layout runs");
                    let expected: Value = serde_json::from_str(&value.to_str()).unwrap();
                    let expected_nodes: Vec<_> = expected["nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|n| n["key"] != "glow")
                        .collect();
                    assert_eq!(rust.nodes.len(), expected_nodes.len(), "count {current}");
                    for node in &expected_nodes {
                        let index = json
                            .as_array()
                            .unwrap()
                            .iter()
                            .position(|e| e["id"] == node["key"])
                            .unwrap();
                        let placed = rust.node(ids[index]).expect("node is visible in Rust");
                        for (key, value) in [
                            ("x", placed.rect.x),
                            ("y", placed.rect.y),
                            ("s", placed.rect.s),
                            ("w", placed.rect.w),
                        ] {
                            assert!(
                                (value - node[key].as_f64().unwrap()).abs() < 1e-4,
                                "{key} of node {index} with current {current}, open {open}, width {width}"
                            );
                        }
                        let badge = node["badge"]
                            .as_str()
                            .map(|b| b[1..].parse::<usize>().unwrap())
                            .unwrap_or(0);
                        assert_eq!(placed.hidden.len(), badge, "badge of node {index}");
                    }
                    assert!((rust.width - expected["width"].as_f64().unwrap()).abs() < 1e-4);
                    assert_eq!(
                        rust.edges.len(),
                        expected["edges"].as_array().unwrap().len()
                    );
                    cases += 1;
                }
            }
        }
        assert!(cases > 100);
    }

    #[test]
    fn trunk_keeps_its_coordinates_between_rest_and_open() {
        let (forest, ids, _) = fixture();
        for &id in &ids {
            if forest.visit(id).unwrap().state == VisitState::Queued {
                continue;
            }
            let rest = strip_layout(&forest, id, 1240., false, 180.);
            let open = strip_layout(&forest, id, 1240., true, 180.);
            for trunk in forest.path(id) {
                let (a, b) = (rest.node(trunk).unwrap(), open.node(trunk).unwrap());
                assert_eq!((a.rect.x, a.rect.y), (b.rect.x, b.rect.y));
            }
            assert!(rest.nodes.iter().all(|n| n.hidden.is_empty()));
        }
    }

    #[test]
    fn badges_name_every_hidden_page_once() {
        let (forest, ids, _) = fixture();
        for &id in &ids {
            if forest.visit(id).unwrap().state == VisitState::Queued {
                continue;
            }
            let open = strip_layout(&forest, id, 800., true, 250.);
            let shown: HashSet<_> = open.nodes.iter().map(|n| n.id).collect();
            let mut hidden = HashSet::new();
            for node in &open.nodes {
                for h in &node.hidden {
                    assert!(!shown.contains(h));
                    assert!(hidden.insert(*h), "hidden twice");
                }
            }
            let tree = forest.tree_of(id).unwrap().root();
            let total = forest.descendant_count(tree).unwrap() + 1;
            assert_eq!(shown.len() + hidden.len(), total);
            for edge in &open.edges {
                assert!(shown.contains(&edge.from) && shown.contains(&edge.to));
            }
        }
    }

    #[test]
    fn a_deep_history_lays_out_without_recursion() {
        let mut visits = Vec::new();
        let mut parent = None;
        for i in 0..20_000 {
            let id: VisitId = ulid::Ulid::new().to_string().parse().unwrap();
            visits.push(Visit {
                id,
                parent,
                url: Url::parse("https://example.org/").unwrap(),
                title: String::new(),
                created_at: i,
                opened_at: Some(i),
                state: VisitState::Opened,
                source: None,
            });
            parent = Some(id);
        }
        let root = visits[0].id;
        let leaf = parent.unwrap();
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: Some(leaf),
            },
            vec![TreeState {
                pin: None,
                last_event_at: None,
                lifecycle: crate::browser::Lifecycle::Open,
                root: crate::browser::TreeId(root),
                last_visit: leaf,
                name: None,
            }],
        )
        .unwrap();
        let layout = strip_layout(&forest, root, 1240., true, 120.);
        assert_eq!(layout.trunk().len(), 5);
        assert_eq!(
            layout.node(layout.trunk()[4]).unwrap().hidden.len(),
            20_000 - 5
        );
        let leaf_layout = strip_layout(&forest, leaf, 1240., false, 120.);
        assert_eq!(leaf_layout.trunk().len(), 20_000);
    }
}
