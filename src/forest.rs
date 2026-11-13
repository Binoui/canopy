use std::collections::HashMap;

use anyhow::{bail, Context, Result};

use crate::browser::{Snapshot, TreeId, TreeState, Visit, VisitId, VisitState};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeSummary {
    pub id: TreeId,
    pub title: String,
    pub root_host: String,
    pub page_count: usize,
    pub pending_count: usize,
    pub last_visit: VisitId,
}

#[derive(Debug)]
struct Node {
    visit: Visit,
    children: Vec<VisitId>,
    tree: TreeId,
    descendants: usize,
    pending_below: usize,
}

#[derive(Debug)]
pub struct ForestProjection {
    nodes: HashMap<VisitId, Node>,
    trees: Vec<TreeSummary>,
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
                        pending_below: 0,
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
                let pending =
                    node.pending_below + usize::from(node.visit.state == VisitState::Queued);
                let parent = nodes.get_mut(&parent).unwrap();
                parent.descendants += descendants;
                parent.pending_below += pending;
            }
        }
        let mut states = HashMap::with_capacity(tree_states.len());
        for state in tree_states {
            if states.insert(state.root, state.last_visit).is_some() {
                bail!("Duplicate tree state.");
            }
        }
        let mut trees = Vec::with_capacity(roots.len());
        for root in roots {
            let node = &nodes[&root];
            let last_visit = states.remove(&node.tree).context("Missing tree state.")?;
            let last = nodes.get(&last_visit).context("Missing tree target.")?;
            if last.tree != node.tree || last.visit.state != VisitState::Opened {
                bail!("Tree target must be an opened page in its tree.");
            }
            trees.push(TreeSummary {
                id: node.tree,
                title: node.visit.label().to_owned(),
                root_host: node.visit.url.host_str().unwrap_or_default().to_owned(),
                page_count: node.descendants + 1,
                pending_count: node.pending_below
                    + usize::from(node.visit.state == VisitState::Queued),
                last_visit,
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
        Ok(Self {
            nodes,
            trees,
            current: snapshot.current,
        })
    }

    pub fn trees(&self) -> &[TreeSummary] {
        &self.trees
    }

    pub fn current(&self) -> Option<VisitId> {
        self.current
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
                root: TreeId(root_id),
                last_visit: leaf,
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
                root: TreeId(root_id),
                last_visit: root_id,
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
            root: TreeId(root.id),
            last_visit: root.id,
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
