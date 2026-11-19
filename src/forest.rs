use std::collections::HashMap;

use anyhow::{bail, Context, Result};

use crate::{
    browser::{Place, Snapshot, TreeId, TreeState, Visit, VisitId, VisitState},
    naming::tree_name,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeSummary {
    pub last_event_at: Option<i64>,
    pub id: TreeId,
    pub title: String,
    pub root_host: String,
    pub page_count: usize,
    pub pending_count: usize,
    pub last_visit: VisitId,
    pub place: Place,
}

#[derive(Debug, Eq, PartialEq)]
struct Node {
    visit: Visit,
    children: Vec<VisitId>,
    tree: TreeId,
    descendants: usize,
    settled: bool,
    pending_below: usize,
    opened_below: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ForestProjection {
    nodes: HashMap<VisitId, Node>,
    trees: Vec<TreeSummary>,
    by_id: HashMap<TreeId, usize>,
    open: usize,
    shelf: usize,
    current: Option<VisitId>,
}

impl ForestProjection {
    pub fn from_snapshot(mut snapshot: Snapshot, tree_states: Vec<TreeState>) -> Result<Self> {
        snapshot.visits.sort_by_key(|visit| visit.created_at);
        let mut nodes = HashMap::with_capacity(snapshot.visits.len());
        let mut order = Vec::with_capacity(snapshot.visits.len());
        for visit in snapshot.visits {
            let id = visit.id;
            order.push(id);
            if nodes
                .insert(
                    id,
                    Node {
                        visit,
                        children: Vec::new(),
                        tree: TreeId(id),
                        descendants: 0,
                        settled: false,
                        pending_below: 0,
                        opened_below: 0,
                    },
                )
                .is_some()
            {
                bail!("Duplicate visit ID in forest.");
            }
        }
        let mut roots = Vec::new();
        for &id in &order {
            if let Some(parent) = nodes[&id].visit.parent {
                nodes
                    .get_mut(&parent)
                    .context("Missing parent in forest.")?
                    .children
                    .push(id);
            } else {
                roots.push(id);
            }
        }
        let mut traversal = Vec::with_capacity(nodes.len());
        let mut stack: Vec<_> = roots.iter().rev().map(|&id| (id, TreeId(id))).collect();
        while let Some((id, tree)) = stack.pop() {
            let node = nodes.get_mut(&id).unwrap();
            node.tree = tree;
            let parent = node.visit.parent;
            let settled =
                node.visit.branch_settled_at.is_some() || parent.is_some_and(|p| nodes[&p].settled);
            let node = nodes.get_mut(&id).unwrap();
            node.settled = settled;
            traversal.push(id);
            stack.extend(node.children.iter().rev().map(|&child| (child, tree)));
        }
        if traversal.len() != nodes.len() {
            bail!("Cycle in forest.");
        }
        for &id in traversal.iter().rev() {
            let node = &nodes[&id];
            if let Some(parent) = node.visit.parent {
                let descendants = node.descendants + 1;
                let pending = if node.settled {
                    0
                } else {
                    node.pending_below
                        + usize::from(
                            node.visit.state == VisitState::Queued
                                && node.visit.dismissed_at.is_none(),
                        )
                };
                let opened =
                    node.opened_below + usize::from(node.visit.state == VisitState::Opened);
                let parent = nodes.get_mut(&parent).unwrap();
                parent.descendants += descendants;
                parent.opened_below += opened;
                parent.pending_below += pending;
            }
        }
        let tree_order: HashMap<_, _> = tree_states
            .iter()
            .enumerate()
            .map(|(i, s)| (s.root, i))
            .collect();
        let mut states = HashMap::with_capacity(tree_states.len());
        for state in tree_states {
            if states.insert(state.root, state).is_some() {
                bail!("Duplicate tree state.");
            }
        }
        let mut trees = Vec::with_capacity(roots.len());
        for root in roots {
            let node = &nodes[&root];
            let TreeState {
                last_visit,
                name,
                spot,
                last_event_at,
                ..
            } = states.remove(&node.tree).context("Missing tree state.")?;
            let last = nodes.get(&last_visit).context("Missing tree target.")?;
            if last.tree != node.tree || last.visit.state != VisitState::Opened {
                bail!("Tree target must be an opened page in its tree.");
            }
            trees.push(TreeSummary {
                last_event_at,
                id: node.tree,
                title: name.unwrap_or_else(|| {
                    tree_name(
                        &node.visit,
                        node.children.iter().map(|child| &nodes[child].visit),
                    )
                }),
                root_host: node.visit.url.host_str().unwrap_or_default().to_owned(),
                page_count: node.descendants + 1,
                pending_count: node.pending_below
                    + usize::from(
                        node.visit.state == VisitState::Queued
                            && node.visit.dismissed_at.is_none()
                            && !node.settled,
                    ),
                last_visit,
                place: spot.place(),
            });
        }
        if !states.is_empty() {
            bail!("Tree state does not identify a root.");
        }
        if let Some(current) = snapshot.current {
            let node = nodes.get(&current).context("Missing current page.")?;
            if node.visit.state != VisitState::Opened {
                bail!("Current page has not been opened.");
            }
        }
        trees.sort_by_key(|tree| tree_order[&tree.id]);
        let open = trees.partition_point(|tree| tree.place.is_tab());
        let shelf = trees.partition_point(|tree| !matches!(tree.place, Place::History { .. }));
        let by_id = trees.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        Ok(Self {
            by_id,
            open,
            shelf,
            nodes,
            trees,
            current: snapshot.current,
        })
    }

    pub fn is_settled(&self, id: VisitId) -> bool {
        self.nodes.get(&id).is_some_and(|n| n.settled)
    }

    pub fn is_waiting(&self, id: VisitId) -> bool {
        self.visit(id)
            .is_some_and(|v| v.state == VisitState::Queued && v.dismissed_at.is_none())
            && !self.is_settled(id)
    }

    pub fn opened_descendant_count(&self, id: VisitId) -> usize {
        self.nodes.get(&id).map_or(0, |n| n.opened_below)
    }

    pub fn siblings(&self, id: VisitId) -> Vec<VisitId> {
        let mut siblings = self
            .visit(id)
            .and_then(|v| v.parent)
            .map_or_else(Vec::new, |p| self.children(p).to_vec());
        siblings.sort_by_key(|id| {
            std::cmp::Reverse((self.visit(*id).unwrap().created_at, id.to_string()))
        });
        siblings
    }

    pub fn trees(&self) -> &[TreeSummary] {
        &self.trees
    }

    pub fn shelf(&self) -> Vec<&TreeSummary> {
        self.trees[self.open..self.shelf].iter().collect()
    }
    pub fn tabs(&self) -> &[TreeSummary] {
        &self.trees[..self.open]
    }
    pub fn history(&self) -> &[TreeSummary] {
        &self.trees[self.shelf..]
    }
    pub fn tree(&self, id: TreeId) -> Option<&TreeSummary> {
        self.by_id.get(&id).map(|&i| &self.trees[i])
    }

    pub fn current(&self) -> Option<VisitId> {
        self.current
    }

    /// Shows `id` as the current page, for a page that is opening but has not committed.
    /// An id the forest does not hold changes nothing.
    pub fn show_current(&mut self, id: VisitId) {
        if self.nodes.contains_key(&id) {
            self.current = Some(id);
        }
    }

    pub fn visit(&self, id: VisitId) -> Option<&Visit> {
        self.nodes.get(&id).map(|node| &node.visit)
    }

    pub fn path(&self, current: VisitId) -> Vec<VisitId> {
        let mut path = Vec::new();
        let mut cursor = Some(current);
        while let Some(node) = cursor.and_then(|id| self.nodes.get(&id)) {
            path.push(node.visit.id);
            cursor = node.visit.parent;
        }
        path.reverse();
        path
    }

    pub fn children(&self, id: VisitId) -> &[VisitId] {
        self.nodes.get(&id).map_or(&[], |node| &node.children)
    }

    pub fn descendant_count(&self, id: VisitId) -> Option<usize> {
        self.nodes.get(&id).map(|node| node.descendants)
    }

    pub fn pending_descendant_count(&self, id: VisitId) -> Option<usize> {
        self.nodes.get(&id).map(|node| node.pending_below)
    }

    pub fn tree_of(&self, id: VisitId) -> Option<TreeId> {
        self.nodes.get(&id).map(|node| node.tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid;
    use url::Url;

    fn visit(parent: Option<VisitId>, time: i64, state: VisitState) -> Visit {
        Visit {
            dismissed_at: None,
            branch_settled_at: None,
            id: Ulid::new().to_string().parse().unwrap(),
            parent,
            url: Url::parse("https://example.org/repeated").unwrap(),
            title: String::new(),
            created_at: time,
            opened_at: (state == VisitState::Opened).then_some(time),
            state,
            source: None,
        }
    }

    #[test]
    fn a_current_page_inside_a_settled_tree_is_a_valid_forest() {
        let root = visit(None, 1, VisitState::Opened);
        let id = root.id;
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![root],
                current: Some(id),
            },
            vec![TreeState {
                last_event_at: None,
                root: TreeId(id),
                last_visit: id,
                name: None,
                spot: crate::browser::Spot::History { settled_at: 2 },
            }],
        )
        .unwrap();
        assert_eq!(forest.current(), Some(id));
        assert_eq!(forest.tree_of(id), Some(TreeId(id)));
        assert!(forest.tabs().is_empty());
        assert_eq!(forest.history().len(), 1);
    }

    #[test]
    fn projection_keeps_the_stores_order_of_tabs_and_history() {
        let visits: Vec<_> = (0..5)
            .map(|time| visit(None, time, VisitState::Opened))
            .collect();
        let ids: Vec<_> = visits.iter().map(|v| v.id).collect();
        let states = [0, 2, 4, 3, 1]
            .into_iter()
            .map(|i| {
                let id = ids[i];
                TreeState {
                    last_event_at: None,
                    root: TreeId(id),
                    last_visit: id,
                    name: None,
                    spot: match i {
                        1 => crate::browser::Spot::History { settled_at: 10 },
                        3 => crate::browser::Spot::History { settled_at: 20 },
                        _ => crate::browser::Spot::Tab {
                            pinned: false,
                            position: i as u32,
                        },
                    },
                }
            })
            .collect();
        let f = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: Some(ids[1]),
            },
            states,
        )
        .unwrap();
        assert_eq!(
            f.tabs().iter().map(|t| t.id.root()).collect::<Vec<_>>(),
            vec![ids[0], ids[2], ids[4]]
        );
        assert_eq!(
            f.history().iter().map(|t| t.id.root()).collect::<Vec<_>>(),
            vec![ids[3], ids[1]]
        );
        assert_eq!(
            f.current(),
            Some(ids[1]),
            "A current page inside a settled tree remains valid"
        );
    }

    #[test]
    fn deep_chain_uses_ids_and_iterative_traversal() {
        let root = visit(None, 0, VisitState::Opened);
        let root_id = root.id;
        let mut visits = vec![root];
        for i in 1..20_000 {
            visits.push(visit(
                Some(visits.last().unwrap().id),
                i,
                VisitState::Opened,
            ));
        }
        let ids: Vec<_> = visits.iter().map(|v| v.id).collect();
        let leaf = *ids.last().unwrap();
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: Some(leaf),
            },
            vec![TreeState {
                last_event_at: None,
                spot: crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0,
                },
                root: TreeId(root_id),
                last_visit: leaf,
                name: None,
            }],
        )
        .unwrap();
        assert_eq!(forest.path(leaf), ids);
        assert_eq!(forest.descendant_count(root_id), Some(19_999));
        assert_eq!(forest.tree_of(leaf), Some(TreeId(root_id)));
        assert_eq!(forest.trees()[0].page_count, 20_000);
        assert_eq!(forest.children(root_id), &ids[1..2]);
        assert_eq!(forest.current(), Some(leaf));
    }

    #[test]
    fn roots_and_siblings_keep_creation_order_with_queued_leaves() {
        let mut visits = Vec::new();
        let mut states = Vec::new();
        for i in 0..1_000 {
            let mut root = visit(None, i, VisitState::Opened);
            root.title = format!("Tree {i}");
            let root_id = root.id;
            visits.push(root);
            let first = visit(Some(root_id), i, VisitState::Queued);
            let second = visit(Some(root_id), i, VisitState::Queued);
            visits.extend([first, second]);
            states.push(TreeState {
                last_event_at: None,
                spot: crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0,
                },
                root: TreeId(root_id),
                last_visit: root_id,
                name: None,
            });
        }
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits: visits.clone(),
                current: None,
            },
            states.clone(),
        )
        .unwrap();
        assert_eq!(forest.trees().len(), 1_000);
        for (i, tree) in forest.trees().iter().enumerate() {
            assert_eq!(tree.id, states[i].root);
            assert_eq!(tree.title, format!("Tree {i}"));
            assert_eq!(tree.root_host, "example.org");
            assert_eq!(tree.page_count, 3);
            assert_eq!(tree.pending_count, 2);
            assert_eq!(tree.last_visit, tree.id.root());
            let children = [visits[i * 3 + 1].id, visits[i * 3 + 2].id];
            assert_eq!(forest.children(tree.id.root()), children);
            assert_eq!(forest.pending_descendant_count(tree.id.root()), Some(2));
            assert_eq!(forest.descendant_count(children[0]), Some(0));
            assert_ne!(children[0], children[1]);
            assert_eq!(
                forest.visit(children[0]).unwrap().url,
                forest.visit(children[1]).unwrap().url
            );
        }
    }

    #[test]
    fn malformed_forests_and_targets_are_rejected() {
        let root = visit(None, 0, VisitState::Opened);
        let child = visit(Some(root.id), 1, VisitState::Queued);
        let state = TreeState {
            last_event_at: None,
            spot: crate::browser::Spot::Tab {
                pinned: false,
                position: 0,
            },
            root: TreeId(root.id),
            last_visit: root.id,
            name: None,
        };
        let snapshot = Snapshot {
            visits: vec![root.clone(), child.clone()],
            current: Some(root.id),
        };
        assert!(ForestProjection::from_snapshot(snapshot.clone(), vec![]).is_err());
        assert!(ForestProjection::from_snapshot(
            snapshot.clone(),
            vec![state.clone(), state.clone()]
        )
        .is_err());
        assert!(ForestProjection::from_snapshot(
            snapshot,
            vec![TreeState {
                last_event_at: None,
                spot: crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0
                },
                last_visit: child.id,
                ..state.clone()
            }]
        )
        .is_err());
        let mut cycle = root.clone();
        cycle.parent = Some(child.id);
        assert!(ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![cycle, child],
                current: None
            },
            vec![state]
        )
        .is_err());
        assert!(ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![root.clone(), root],
                current: None
            },
            vec![]
        )
        .is_err());
        assert!(ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![],
                current: None
            },
            vec![]
        )
        .unwrap()
        .trees()
        .is_empty());
    }
}
