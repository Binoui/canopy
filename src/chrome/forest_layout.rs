//! The forest view's layout: every tree, the current one first, as rows of nodes.

use std::collections::HashMap;

use super::graph::Rect;
use crate::{browser::VisitId, forest::ForestProjection};

#[derive(Default)]
pub(crate) struct ForestLayout {
    pub nodes: Vec<(VisitId, Rect)>,
    pub heads: Vec<(usize, f64)>,
    pub current_width: f64,
}

pub(crate) fn layout(
    forest: &ForestProjection,
    labels: bool,
    measure: impl Fn(&str) -> f64,
) -> ForestLayout {
    let mut result = ForestLayout::default();
    let current = forest.current().and_then(|id| forest.tree_of(id));
    let mut order: Vec<_> = (0..forest.trees().len()).collect();
    order.sort_by_key(|&i| Some(forest.trees()[i].id) != current);
    let mut y = 34.;
    for index in order {
        let tree = &forest.trees()[index];
        result.heads.push((index, y));
        y += 78.;
        let mut rows = HashMap::new();
        let mut stack = vec![(tree.id.root(), 0, false)];
        let mut row = 0.;
        let mut width: f64 = 0.;
        while let Some((id, depth, visited)) = stack.pop() {
            let children = forest.children(id);
            if !visited && !children.is_empty() {
                stack.push((id, depth, true));
                stack.extend(children.iter().rev().map(|&id| (id, depth + 1, false)));
                continue;
            }
            let r = if children.is_empty() {
                let r = row;
                row += 1.;
                r
            } else {
                (rows[&children[0]] + rows[children.last().unwrap()]) / 2.
            };
            rows.insert(id, r);
            let w = if labels {
                (43. + measure(forest.visit(id).unwrap().label())).clamp(58., 180.)
            } else {
                30.
            };
            let left = 34. + depth as f64 * if labels { 218. } else { 58. };
            result.nodes.push((
                id,
                Rect {
                    x: left + w / 2.,
                    y: y + r * if labels { 40. } else { 38. } + 15.,
                    s: 30.,
                    w,
                },
            ));
            width = width.max(left + w + 34.);
        }
        if result.current_width == 0. {
            result.current_width = width;
        }
        y += row * if labels { 40. } else { 38. } + 34.;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, TreeId, TreeState, Visit, VisitState};
    fn fixture(trees: usize, depth: usize) -> ForestProjection {
        let mut visits = Vec::new();
        let mut states = Vec::new();
        let mut current = None;
        for _ in 0..trees {
            let root: VisitId = ulid::Ulid::new().to_string().parse().unwrap();
            let mut parent = None;
            for d in 0..depth {
                let id = if d == 0 {
                    root
                } else {
                    ulid::Ulid::new().to_string().parse().unwrap()
                };
                visits.push(Visit {
                    id,
                    parent,
                    url: url::Url::parse("https://example.org/").unwrap(),
                    title: "Example".into(),
                    created_at: visits.len() as i64,
                    opened_at: Some(0),
                    state: VisitState::Opened,
                    source: None,
                });
                parent = Some(id);
            }
            current = parent;
            states.push(TreeState {
                root: TreeId(root),
                last_visit: parent.unwrap(),
                name: None,
            });
        }
        ForestProjection::from_snapshot(Snapshot { visits, current }, states).unwrap()
    }
    #[test]
    fn deep_layout_is_iterative_and_keeps_every_node() {
        let f = fixture(1, 20_000);
        let l = layout(&f, true, |_| 50.);
        assert_eq!(l.nodes.len(), 20_000);
        assert_eq!(l.nodes.first().unwrap().1.x, 34. + 19_999. * 218. + 46.5);
    }
    #[test]
    fn all_trees_are_ordered_current_first_in_both_modes() {
        let f = fixture(1000, 3);
        for labels in [true, false] {
            let l = layout(&f, labels, |_| 50.);
            assert_eq!(l.heads.len(), 1000);
            assert_eq!(l.nodes.len(), 3000);
            assert_eq!(l.heads[0].0, 999);
            assert!(l.heads.windows(2).all(|w| w[0].1 < w[1].1));
            assert_eq!(l.nodes[0].1.w, if labels { 93. } else { 30. });
            assert_eq!(l.current_width, if labels { 597. } else { 214. });
        }
    }
}
