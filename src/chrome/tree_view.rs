//! Both sides of the current-tree sheet, independent of the viewport.
use std::collections::HashMap;

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

pub(crate) fn tree_view(forest: &ForestProjection) -> Option<TreeView> {
    let path = forest.path(forest.current()?);
    let grid = place(forest, &path);
    let rows = list(forest, &path);
    let tiles = rows
        .iter()
        .map(|row| {
            let &(col, y) = &grid.cells[&row.id];
            Tile {
                id: row.id,
                rect: cell_rect(col, y),
                on_path: row.on_path,
            }
        })
        .collect();
    let edges = grid
        .edges
        .iter()
        .map(|edge| {
            let (ac, ar) = grid.cells[&edge.from];
            let (bc, br) = grid.cells[&edge.to];
            PlacedEdge {
                from: edge.from,
                to: edge.to,
                kind: edge.kind,
                curve: graph::edge_curve(edge, cell_rect(ac, ar), cell_rect(bc, br)),
            }
        })
        .collect();
    Some(TreeView {
        tiles,
        edges,
        rows,
        width: PAD * 2. + (grid.columns - 1) as f64 * COLUMN + TILE_WIDTH,
        height: PAD * 2. + (grid.rows - 1) as f64 * ROW + TILE_HEIGHT,
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

fn list(forest: &ForestProjection, path: &[VisitId]) -> Vec<ListRow> {
    let mut rows = vec![];
    for (i, &id) in path.iter().enumerate() {
        rows.push(ListRow {
            id,
            depth: 0,
            on_path: true,
        });
        let mut stack: Vec<_> = forest
            .children(id)
            .iter()
            .rev()
            .filter(|&&child| path.get(i + 1) != Some(&child))
            .map(|&child| (child, 1))
            .collect();
        while let Some((id, depth)) = stack.pop() {
            rows.push(ListRow {
                id,
                depth,
                on_path: false,
            });
            if forest.is_settled(id) {
                continue;
            }
            stack.extend(
                forest
                    .children(id)
                    .iter()
                    .rev()
                    .map(|&child| (child, depth + 1)),
            );
        }
    }
    rows
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
            view.rows
                .iter()
                .map(|r| (r.id, r.depth))
                .collect::<Vec<_>>(),
            [
                (ids[0], 0),
                (ids[4], 1),
                (ids[5], 2),
                (ids[6], 3),
                (ids[1], 0),
                (ids[7], 1),
                (ids[2], 0),
                (ids[8], 1),
                (ids[3], 0),
                (ids[9], 1),
                (ids[10], 1)
            ]
        );
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
