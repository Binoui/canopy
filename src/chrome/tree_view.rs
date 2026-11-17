//! Both sides of the current-tree sheet, independent of the viewport.
use std::collections::{HashMap, HashSet};

use super::graph::{self, Edge, EdgeKind, Rect};
use crate::{browser::VisitId, forest::ForestProjection};

const TILE_WIDTH: f64 = 124.;
const TILE_HEIGHT: f64 = 40.;
const COLUMN: f64 = TILE_WIDTH + 22.;
const ROW: f64 = TILE_HEIGHT + 10.;
const PAD: f64 = 14.;

#[derive(Debug, PartialEq)]
pub(crate) struct TreeView {
    pub tiles: Vec<Tile>,
    pub edges: Vec<PlacedEdge>,
    pub rows: Vec<ListRow>,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, PartialEq)]
pub(crate) struct Tile {
    pub id: VisitId,
    pub rect: Rect,
    pub on_path: bool,
}
#[derive(Debug, PartialEq)]
pub(crate) struct PlacedEdge {
    pub from: VisitId,
    pub to: VisitId,
    pub kind: EdgeKind,
    pub curve: [(f64, f64); 4],
}
#[derive(Debug, PartialEq)]
pub(crate) struct ListRow {
    pub id: VisitId,
    pub depth: usize,
    pub on_path: bool,
}

#[derive(Default)]
pub(crate) struct Workbench {
    pub folds: HashMap<(VisitId, Option<i64>), bool>,
    pub query: String,
    pub waiting_only: bool,
}

impl Workbench {
    pub fn folded(&self, forest: &ForestProjection, id: VisitId) -> bool {
        let settled_at = forest.visit(id).and_then(|v| v.branch_settled_at);
        self.folds
            .get(&(id, settled_at))
            .copied()
            .unwrap_or(settled_at.is_some())
    }
}

#[cfg(test)]
pub(crate) fn tree_view(forest: &ForestProjection) -> Option<TreeView> {
    workbench(forest, &Workbench::default())
}

pub(crate) fn workbench(forest: &ForestProjection, state: &Workbench) -> Option<TreeView> {
    let path = forest.path(forest.current()?);
    let root = *path.first()?;
    let path: HashSet<_> = path.into_iter().collect();
    let filtering = !state.query.is_empty() || state.waiting_only;
    let query = state.query.to_lowercase();
    let mut outline = Vec::new();
    let mut stack = vec![(root, 0)];
    while let Some((id, depth)) = stack.pop() {
        outline.push(ListRow {
            id,
            depth,
            on_path: path.contains(&id),
        });
        let mut children = forest.children(id).to_vec();
        children.sort_by_key(|id| {
            std::cmp::Reverse((forest.visit(*id).unwrap().created_at, id.to_string()))
        });
        stack.extend(
            children
                .into_iter()
                .enumerate()
                .map(|(n, id)| (id, depth + usize::from(n > 0))),
        );
    }
    let mut admitted = HashSet::new();
    if filtering {
        for row in outline.iter().rev() {
            let v = forest.visit(row.id).unwrap();
            if ((graph::short_name(forest, row.id)
                .to_lowercase()
                .contains(&query)
                || super::view::short_host(&v.url)
                    .to_lowercase()
                    .contains(&query))
                && (!state.waiting_only || forest.is_waiting(row.id)))
                || admitted.contains(&row.id)
            {
                admitted.insert(row.id);
                if let Some(parent) = v.parent {
                    admitted.insert(parent);
                }
            }
        }
    }
    let mut hidden = HashSet::new();
    let mut rows = Vec::new();
    for row in outline {
        let parent = forest.visit(row.id).unwrap().parent;
        if parent.is_some_and(|p| hidden.contains(&p)) {
            hidden.insert(row.id);
            continue;
        }
        if filtering && !admitted.contains(&row.id) {
            continue;
        }
        if !filtering && state.folded(forest, row.id) {
            hidden.insert(row.id);
        }
        rows.push(row);
    }
    let mut cells = HashMap::new();
    let mut tiles = Vec::new();
    let mut edges = Vec::new();
    let mut positions = HashMap::new();
    let mut last_row = 0;
    let mut columns = 1;
    for row in &rows {
        let (column, y) = forest
            .visit(row.id)
            .unwrap()
            .parent
            .and_then(|id| positions.get(&id).copied())
            .map_or((0, 0), |(column, y, depth)| {
                let y = if row.depth == depth { y } else { last_row + 1 };
                (column + 1, y)
            });
        positions.insert(row.id, (column, y, row.depth));
        last_row = last_row.max(y);
        columns = columns.max(column + 1);
        let rect = cell_rect(column, y);
        cells.insert(row.id, rect);
        tiles.push(Tile {
            id: row.id,
            rect,
            on_path: row.on_path,
        });
        if let Some(parent) = forest.visit(row.id).unwrap().parent {
            if let Some(&from) = cells.get(&parent) {
                let kind = if row.on_path {
                    EdgeKind::Path
                } else {
                    graph::edge_kind(forest, row.id)
                };
                let edge = Edge {
                    from: parent,
                    to: row.id,
                    kind,
                    from_bottom: from.y != rect.y,
                };
                edges.push(PlacedEdge {
                    from: parent,
                    to: row.id,
                    kind,
                    curve: graph::edge_curve(&edge, from, rect),
                });
            }
        }
    }
    Some(TreeView {
        width: PAD * 2. + (columns - 1) as f64 * COLUMN + TILE_WIDTH,
        height: PAD * 2. + last_row as f64 * ROW + TILE_HEIGHT,
        tiles,
        edges,
        rows,
    })
}

#[derive(Clone, Copy)]
pub(crate) struct Fit {
    pub columns: usize,
    pub rows: usize,
    pub icon: f64,
    pub step_x: f64,
    pub step_y: f64,
}

impl Fit {
    pub const CHIP: Self = Self {
        columns: 3,
        rows: 2,
        icon: 12.,
        step_x: 20.,
        step_y: 18.,
    };
    pub const CARD: Self = Self {
        columns: 7,
        rows: 3,
        icon: 14.,
        step_x: 26.,
        step_y: 22.,
    };
}

pub(crate) struct Miniature {
    pub width: f64,
    pub height: f64,
    pub icon: f64,
    pub cut: bool,
    pub nodes: Vec<MiniNode>,
    pub edges: Vec<PlacedEdge>,
}

pub(crate) struct MiniNode {
    pub id: VisitId,
    pub x: f64,
    pub y: f64,
    pub last: bool,
    pub waiting: bool,
}

pub(crate) fn miniature(
    forest: &ForestProjection,
    tree: &crate::forest::TreeSummary,
    fit: Fit,
) -> Miniature {
    let path = forest.path(tree.last_visit);
    let grid = place(forest, &path);
    let first = path.len().saturating_sub(fit.columns);
    let mut cells: Vec<_> = grid
        .cells
        .iter()
        .map(|(&id, &(col, row))| (id, col, row))
        .collect();
    cells.sort_by_key(|&(_, col, row)| (col, row));
    let mut kept = HashMap::new();
    for (id, col, row) in cells {
        if col < first || col >= first + fit.columns || row >= fit.rows {
            continue;
        }
        if id == path[first]
            || forest
                .visit(id)
                .and_then(|v| v.parent)
                .is_some_and(|p| kept.contains_key(&p))
        {
            kept.insert(
                id,
                ((col - first) as f64 * fit.step_x, row as f64 * fit.step_y),
            );
        }
    }
    let mut nodes: Vec<_> = kept
        .iter()
        .map(|(&id, &(x, y))| MiniNode {
            id,
            x,
            y,
            last: id == tree.last_visit,
            waiting: forest.is_waiting(id),
        })
        .collect();
    nodes.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));
    let rect = |id: &VisitId| {
        let &(x, y) = &kept[id];
        Rect {
            x: x + fit.icon / 2.,
            y: y + fit.icon / 2.,
            w: fit.icon,
            s: fit.icon,
        }
    };
    let edges: Vec<_> = grid
        .edges
        .iter()
        .filter(|e| kept.contains_key(&e.from) && kept.contains_key(&e.to))
        .map(|e| PlacedEdge {
            from: e.from,
            to: e.to,
            kind: e.kind,
            curve: graph::edge_curve(e, rect(&e.from), rect(&e.to)),
        })
        .collect();
    Miniature {
        width: (fit.columns - 1) as f64 * fit.step_x + fit.icon,
        height: (fit.rows - 1) as f64 * fit.step_y + fit.icon,
        icon: fit.icon,
        cut: first > 0,
        nodes,
        edges,
    }
}

struct Grid {
    cells: HashMap<VisitId, (usize, usize)>,
    edges: Vec<Edge>,
    columns: usize,
    rows: usize,
}

impl Grid {
    fn insert(&mut self, id: VisitId, col: usize, row: usize) {
        self.cells.insert(id, (col, row));
        self.columns = self.columns.max(col + 1);
        self.rows = self.rows.max(row + 1);
    }
}

// Each row in a block belongs to a first-child chain. Column floors keep later blocks
// below earlier blocks and their drop lines. Fanned edges stay in the column gaps.
fn place(forest: &ForestProjection, path: &[VisitId]) -> Grid {
    let mut grid = Grid {
        cells: HashMap::new(),
        edges: vec![],
        columns: 0,
        rows: 1,
    };
    let mut floor = vec![1; path.len()];
    for (col, &id) in path.iter().enumerate() {
        grid.insert(id, col, 0);
        if col > 0 {
            grid.edges.push(Edge {
                from: path[col - 1],
                to: id,
                kind: EdgeKind::Path,
                from_bottom: false,
            });
        }
    }
    for i in (0..path.len()).rev() {
        for &child in forest.children(path[i]) {
            if path.get(i + 1) == Some(&child) {
                continue;
            }
            let mut cells = Vec::new();
            let mut edges = vec![Edge {
                from: path[i],
                to: child,
                kind: graph::edge_kind(forest, child),
                from_bottom: i + 1 < path.len(),
            }];
            let mut stack = vec![(child, i + 1, true)];
            let mut row = 0;
            let mut last_col = i + 1;
            while let Some((id, col, first)) = stack.pop() {
                if !first {
                    row += 1;
                }
                cells.push((id, col, row));
                last_col = last_col.max(col);
                if forest.is_settled(id) {
                    continue;
                }
                for (n, &next) in forest.children(id).iter().enumerate().rev() {
                    edges.push(Edge {
                        from: id,
                        to: next,
                        kind: graph::edge_kind(forest, next),
                        from_bottom: false,
                    });
                    stack.push((next, col + 1, n == 0));
                }
            }
            if floor.len() <= last_col {
                floor.resize(last_col + 1, 0);
            }
            let start_col = if i + 1 == path.len() { i + 1 } else { i };
            let offset = *floor[start_col..=last_col].iter().max().unwrap();
            for (id, col, r) in cells {
                grid.insert(id, col, r + offset);
            }
            floor[start_col..=last_col].fill(offset + row + 1);
            grid.edges.extend(edges);
        }
    }
    grid
}

fn cell_rect(column: usize, row: usize) -> Rect {
    Rect {
        x: PAD + column as f64 * COLUMN + TILE_WIDTH / 2.,
        y: PAD + row as f64 * ROW + TILE_HEIGHT / 2.,
        w: TILE_WIDTH,
        s: TILE_HEIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Lifecycle, Snapshot, TreeId, TreeState, Visit, VisitState};
    use std::collections::HashSet;

    #[test]
    fn outline_stays_put_when_current_moves_and_only_indents_forks() {
        let parents = [None, Some(0), Some(1), Some(1), Some(3)];
        let (mut forest, ids) = fixture(&parents, 2);
        let before = tree_view(&forest).unwrap();
        let mut snapshot = Snapshot {
            visits: ids
                .iter()
                .map(|&id| forest.visit(id).unwrap().clone())
                .collect(),
            current: forest.current(),
        };
        snapshot.current = Some(ids[3]);
        forest = ForestProjection::from_snapshot(
            snapshot,
            vec![TreeState {
                root: TreeId(ids[0]),
                last_visit: ids[3],
                lifecycle: Lifecycle::Open,
                name: None,
                pin: None,
                last_event_at: None,
            }],
        )
        .unwrap();
        let after = tree_view(&forest).unwrap();
        assert_eq!(
            before
                .rows
                .iter()
                .map(|r| (r.id, r.depth))
                .collect::<Vec<_>>(),
            after
                .rows
                .iter()
                .map(|r| (r.id, r.depth))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            before.rows.iter().map(|r| r.depth).collect::<Vec<_>>(),
            [0, 0, 1, 0, 0]
        );
        assert_eq!(
            before.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [ids[0], ids[1], ids[2], ids[3], ids[4]]
        );
        let parent_row = before.rows.iter().position(|r| r.id == ids[1]).unwrap();
        assert_eq!(before.rows[parent_row + 1].id, ids[2]);
        assert_eq!(
            before.tiles.iter().map(|t| t.rect).collect::<Vec<_>>(),
            after.tiles.iter().map(|t| t.rect).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_ten_page_chain_and_its_side_branch_use_two_rows() {
        let mut parents = vec![None];
        parents.extend((0..8).map(Some));
        parents.push(Some(3));
        let (forest, ids) = fixture(&parents, 8);
        let view = tree_view(&forest).unwrap();
        assert_eq!(view.height, 118.);
        let root = view.tiles.iter().find(|t| t.id == ids[0]).unwrap().rect;
        let newest = view.tiles.iter().find(|t| t.id == ids[9]).unwrap().rect;
        let older = view.tiles.iter().find(|t| t.id == ids[4]).unwrap().rect;
        assert_eq!(newest.y, root.y);
        assert!(newest.x > root.x);
        assert!(older.y > newest.y);
        assert_eq!(older.x, newest.x);
        assert!(view
            .tiles
            .iter()
            .all(|t| t.rect.w == 124. && t.rect.s == 40.));
    }

    #[test]
    fn a_new_settlement_starts_folded_after_an_earlier_unfold() {
        let (forest, ids) = fixture(&[None, Some(0), Some(1)], 0);
        let mut state = Workbench::default();
        state.folds.insert((ids[1], None), false);
        let settled = |stamp| {
            let visits = ids
                .iter()
                .map(|&id| {
                    let mut v = forest.visit(id).unwrap().clone();
                    if id == ids[1] {
                        v.branch_settled_at = Some(stamp);
                    }
                    v
                })
                .collect();
            ForestProjection::from_snapshot(
                Snapshot {
                    visits,
                    current: Some(ids[0]),
                },
                vec![TreeState {
                    root: TreeId(ids[0]),
                    last_visit: ids[0],
                    lifecycle: Lifecycle::Open,
                    name: None,
                    pin: None,
                    last_event_at: None,
                }],
            )
            .unwrap()
        };
        let first = settled(10);
        assert!(state.folded(&first, ids[1]));
        state.folds.insert((ids[1], Some(10)), false);
        assert!(!state.folded(&first, ids[1]));
        assert!(state.folded(&settled(20), ids[1]));
    }

    #[test]
    fn folds_hide_descendants_and_filters_reveal_matching_pages_with_ancestors() {
        let (forest, ids) = fixture(&[None, Some(0), Some(1), Some(0)], 1);
        let mut state = Workbench::default();
        state.folds.insert((ids[0], None), true);
        assert_eq!(workbench(&forest, &state).unwrap().rows.len(), 1);
        state.query = "page 2".into();
        assert_eq!(
            workbench(&forest, &state)
                .unwrap()
                .rows
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            ids[..3]
        );
        state.query.clear();
        state.waiting_only = true;
        assert_eq!(
            workbench(&forest, &state)
                .unwrap()
                .rows
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            [ids[0], ids[3]]
        );
        state.query = "absent".into();
        assert!(workbench(&forest, &state).unwrap().rows.is_empty());
    }

    #[test]
    fn miniature_shows_a_child_of_the_last_page_on_the_first_row_and_marks_waiting() {
        let (forest, ids) = fixture(&[None, Some(0), Some(1)], 1);
        let mini = miniature(&forest, &forest.trees()[0], Fit::CARD);
        let child = mini.nodes.iter().find(|n| n.id == ids[2]).unwrap();
        assert_eq!(child.y, 0.);
        assert!(child.waiting);
        assert_eq!(
            mini.edges.iter().find(|e| e.to == child.id).unwrap().kind,
            EdgeKind::Pending
        );
    }

    #[test]
    fn miniature_crops_without_squeezing_and_keeps_the_last_page_and_gap_mark() {
        let mut parents = vec![None];
        parents.extend((0..18).map(Some));
        let (forest, ids) = fixture(&parents, 16);
        for fit in [Fit::CARD, Fit::CHIP] {
            let mini = miniature(&forest, &forest.trees()[0], fit);
            assert!(mini.cut);
            let last = mini.nodes.iter().find(|n| n.last).unwrap();
            assert_eq!(last.id, ids[16]);
            assert_eq!(last.y, 0.);
            assert!(mini
                .nodes
                .iter()
                .all(|n| n.x < mini.width && n.y < mini.height));
            assert!(mini.nodes.len() <= fit.rows * fit.columns);
            assert!(mini.nodes.iter().all(|n| n.id != ids[0]));
        }
    }

    #[test]
    fn miniature_places_branches_below_the_path_and_never_orphans_a_page() {
        let (forest, ids) = fixture(
            &[None, Some(0), Some(1), Some(0), Some(3), Some(4), Some(1)],
            2,
        );
        let mini = miniature(&forest, &forest.trees()[0], Fit::CARD);
        assert!(!mini.cut);
        for id in &ids[..3] {
            assert_eq!(mini.nodes.iter().find(|n| n.id == *id).unwrap().y, 0.);
        }
        assert!(mini
            .nodes
            .iter()
            .filter(|n| !ids[..3].contains(&n.id))
            .all(|n| n.y > 0.));
        for node in &mini.nodes {
            if let Some(parent) = forest.visit(node.id).unwrap().parent {
                assert!(mini.nodes.iter().any(|n| n.id == parent));
            }
        }
    }

    fn fixture(parents: &[Option<usize>], current: usize) -> (ForestProjection, Vec<VisitId>) {
        let ids: Vec<VisitId> = parents
            .iter()
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect();
        let visits = parents
            .iter()
            .enumerate()
            .map(|(i, p)| Visit {
                dismissed_at: None,
                branch_settled_at: None,
                id: ids[i],
                parent: p.map(|p| ids[p]),
                url: url::Url::parse("https://example.org/").unwrap(),
                title: format!("Page {i}"),
                created_at: i as i64,
                opened_at: (i != parents.len() - 1 || i == current).then_some(i as i64),
                state: if i == parents.len() - 1 && i != current {
                    VisitState::Queued
                } else {
                    VisitState::Opened
                },
                source: None,
            })
            .collect();
        (
            ForestProjection::from_snapshot(
                Snapshot {
                    visits,
                    current: Some(ids[current]),
                },
                vec![TreeState {
                    pin: None,
                    last_event_at: None,
                    root: TreeId(ids[0]),
                    last_visit: ids[current],
                    lifecycle: Lifecycle::Open,
                    name: None,
                }],
            )
            .unwrap(),
            ids,
        )
    }
    #[test]
    fn path_branches_rows_marks_and_minimum_sizes() {
        let (forest, ids) = fixture(
            &[
                None,
                Some(0),
                Some(1),
                Some(2),
                Some(0),
                Some(4),
                Some(5),
                Some(1),
                Some(2),
                Some(3),
                Some(3),
            ],
            3,
        );
        let grid = place(&forest, &forest.path(ids[3]));
        for (i, id) in ids[..4].iter().enumerate() {
            assert_eq!(grid.cells[id], (i, 0));
        }
        assert_eq!(grid.cells[&ids[8]], (3, 1));
        assert_eq!(grid.cells[&ids[7]], (2, 2));
        assert_eq!(grid.cells[&ids[4]], (1, 3));
        let view = tree_view(&forest).unwrap();
        assert!(view
            .tiles
            .iter()
            .all(|t| t.rect.w >= 124. && t.rect.s >= 40.));
        assert_eq!(
            view.rows.iter().map(|r| r.id).collect::<HashSet<_>>(),
            ids.iter().copied().collect()
        );
        assert_eq!(view.rows.iter().find(|r| r.id == ids[6]).unwrap().depth, 0);
        assert!(view
            .edges
            .iter()
            .filter(|e| ids[1..4].contains(&e.to))
            .all(|e| e.kind == EdgeKind::Path));
        assert_eq!(
            view.edges.iter().find(|e| e.to == ids[10]).unwrap().kind,
            EdgeKind::Pending
        );
    }
    #[test]
    fn miniature_generated_forests_keep_connected_pages_and_waiting_marks() {
        for seed in 1..80u64 {
            let mut rng = seed;
            let parents: Vec<_> = (0..400)
                .map(|i| {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                    (i > 0).then(|| rng as usize % i)
                })
                .collect();
            let (forest, _) = fixture(&parents, seed as usize);
            for fit in [Fit::CARD, Fit::CHIP] {
                let mini = miniature(&forest, &forest.trees()[0], fit);
                let path = forest.path(forest.trees()[0].last_visit);
                let first = path[path.len().saturating_sub(fit.columns)];
                assert_eq!(mini.nodes.iter().filter(|n| n.last).count(), 1);
                for node in &mini.nodes {
                    assert!(node.x + fit.icon <= mini.width && node.y + fit.icon <= mini.height);
                    assert_eq!(
                        node.waiting,
                        forest.visit(node.id).unwrap().state == VisitState::Queued
                    );
                    if node.id != first {
                        let parent = forest.visit(node.id).unwrap().parent.unwrap();
                        assert!(mini
                            .edges
                            .iter()
                            .any(|e| e.from == parent && e.to == node.id));
                    }
                }
            }
        }
    }

    #[test]
    fn every_page_once_without_tile_or_drop_line_collisions() {
        for seed in 1..80u64 {
            let mut rng = seed;
            let parents: Vec<_> = (0..400)
                .map(|i| {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                    (i > 0).then(|| rng as usize % i)
                })
                .collect();
            let (forest, ids) = fixture(&parents, seed as usize);
            let grid = place(&forest, &forest.path(ids[seed as usize]));
            assert_eq!(grid.cells.len(), ids.len());
            assert_eq!(grid.cells.values().collect::<HashSet<_>>().len(), ids.len());
            for e in grid.edges.iter().filter(|e| e.from_bottom) {
                let (col, _) = grid.cells[&e.from];
                let (_, row) = grid.cells[&e.to];
                assert!(!grid
                    .cells
                    .values()
                    .any(|&(c, r)| c == col && r > 0 && r <= row));
            }
        }
    }
    #[test]
    fn root_current_and_deep_history_are_iterative() {
        let parents: Vec<_> = (0..20_000usize).map(|i| i.checked_sub(1)).collect();
        for current in [0, 10_000] {
            let (forest, _) = fixture(&parents, current);
            let view = tree_view(&forest).unwrap();
            assert_eq!(view.tiles.len(), 20_000);
            assert_eq!(view.rows.len(), 20_000);
            assert_eq!(view.tiles.iter().filter(|t| t.on_path).count(), current + 1);
        }
    }
}
