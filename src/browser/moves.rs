use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{params, Transaction};

use super::{forest_on, Browser, TreeId, VisitId, VisitState};
use crate::forest::ForestProjection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveTo {
    Page(VisitId),
    Tree(TreeId),
    OwnTree(TabPlacement),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabPlacement {
    Before(TreeId),
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveOutcome {
    Unchanged,
    TabReordered,
    BranchMoved { tree: TreeId },
}

#[derive(Debug, Eq, PartialEq)]
enum Refusal {
    Gone,
    IntoItself,
    AlreadyThere,
    PutAway,
    NotOpened,
    NothingOpened,
    NothingLeft,
    BadAnchor,
}

enum ResolvedMove {
    Attach {
        parent: VisitId,
        destination: TreeId,
        source: AttachSource,
    },
    Detach {
        source: SurvivingSource,
        place: TabPlacement,
    },
    Reorder {
        tree: TreeId,
        place: TabPlacement,
    },
}

enum AttachSource {
    Remaining(SurvivingSource),
    WholeTree(TreeId),
}

struct SurvivingSource {
    tree: TreeId,
    last: SourceLast,
}

enum SourceLast {
    Keep,
    Retarget,
}

pub(crate) struct BranchMoveChoices {
    pub parents: HashSet<VisitId>,
    pub trees: Vec<TreeMoveChoice>,
    pub own_tree: Option<OwnTreeMove>,
    pub warning: Option<String>,
}

pub(crate) struct TreeMoveChoice {
    pub tree: TreeId,
    pub title: String,
}

pub(crate) enum OwnTreeMove {
    Detach,
    Reorder(TreeId),
}

struct MoveContext<'a> {
    forest: &'a ForestProjection,
    head: VisitId,
    source: TreeId,
    parent: Option<VisitId>,
    branch: HashSet<VisitId>,
}

impl<'a> MoveContext<'a> {
    fn new(forest: &'a ForestProjection, head: VisitId) -> Option<Self> {
        let parent = forest.visit(head)?.parent;
        let source = forest.tree_of(head)?;
        let mut branch = HashSet::new();
        let mut stack = vec![head];
        while let Some(id) = stack.pop() {
            branch.insert(id);
            stack.extend(forest.children(id));
        }
        Some(Self {
            forest,
            head,
            source,
            parent,
            branch,
        })
    }

    fn surviving_source(&self, destination: Option<TreeId>) -> SurvivingSource {
        let last = self
            .forest
            .trees()
            .iter()
            .find(|t| t.id == self.source)
            .unwrap()
            .last_visit;
        SurvivingSource {
            tree: self.source,
            last: if destination != Some(self.source) && self.branch.contains(&last) {
                SourceLast::Retarget
            } else {
                SourceLast::Keep
            },
        }
    }

    fn resolve(&self, to: MoveTo) -> std::result::Result<ResolvedMove, Refusal> {
        match to {
            MoveTo::Page(parent) => self.attach(parent),
            MoveTo::Tree(tree) => {
                let target = self
                    .forest
                    .trees()
                    .iter()
                    .find(|t| t.id == tree)
                    .ok_or(Refusal::Gone)?;
                if !target.lifecycle.is_open() {
                    return Err(Refusal::PutAway);
                }
                self.attach(tree.root())
            }
            MoveTo::OwnTree(place) => {
                if let TabPlacement::Before(anchor) = place {
                    if !self.forest.open().iter().any(|t| t.id == anchor) {
                        return Err(Refusal::BadAnchor);
                    }
                }
                if self.parent.is_none() {
                    if !self.forest.open().iter().any(|t| t.id == self.source) {
                        return Err(Refusal::PutAway);
                    }
                    let order: Vec<_> = self.forest.open().iter().map(|t| t.id).collect();
                    if placed_order(&order, self.source, place) == order {
                        return Err(Refusal::AlreadyThere);
                    }
                    Ok(ResolvedMove::Reorder {
                        tree: self.source,
                        place,
                    })
                } else {
                    detach_allowed(self.forest, self.head)?;
                    Ok(ResolvedMove::Detach {
                        source: self.surviving_source(None),
                        place,
                    })
                }
            }
        }
    }

    fn attach(&self, parent: VisitId) -> std::result::Result<ResolvedMove, Refusal> {
        let target = self.forest.visit(parent).ok_or(Refusal::Gone)?;
        if self.branch.contains(&parent) {
            return Err(Refusal::IntoItself);
        }
        if self.parent == Some(parent) {
            return Err(Refusal::AlreadyThere);
        }
        if target.state != VisitState::Opened {
            return Err(Refusal::NotOpened);
        }
        let destination = self.forest.tree_of(target.id).ok_or(Refusal::Gone)?;
        if self.forest.is_settled(parent) || !self.forest.open().iter().any(|t| t.id == destination)
        {
            return Err(Refusal::PutAway);
        }
        let source = if self.parent.is_none() {
            AttachSource::WholeTree(self.source)
        } else {
            if destination != self.source {
                survivor_allowed(self.forest, self.head, self.source)?;
            }
            AttachSource::Remaining(self.surviving_source(Some(destination)))
        };
        Ok(ResolvedMove::Attach {
            parent,
            destination,
            source,
        })
    }
}

fn opened(forest: &ForestProjection, head: VisitId) -> usize {
    forest.opened_descendant_count(head)
        + usize::from(
            forest
                .visit(head)
                .is_some_and(|v| v.state == VisitState::Opened),
        )
}

fn survivor_allowed(
    forest: &ForestProjection,
    head: VisitId,
    source: TreeId,
) -> std::result::Result<(), Refusal> {
    if opened(forest, source.root()) <= opened(forest, head) {
        Err(Refusal::NothingLeft)
    } else {
        Ok(())
    }
}

fn detach_allowed(forest: &ForestProjection, head: VisitId) -> std::result::Result<(), Refusal> {
    let visit = forest.visit(head).ok_or(Refusal::Gone)?;
    if visit.parent.is_none() {
        return Err(Refusal::AlreadyThere);
    }
    if opened(forest, head) == 0 {
        return Err(Refusal::NothingOpened);
    }
    survivor_allowed(forest, head, forest.tree_of(head).ok_or(Refusal::Gone)?)
}

impl ForestProjection {
    pub(crate) fn can_detach_branch(&self, head: VisitId) -> bool {
        detach_allowed(self, head).is_ok()
    }

    pub(crate) fn branch_move_choices(&self, head: VisitId) -> BranchMoveChoices {
        let mut choices = BranchMoveChoices {
            parents: HashSet::new(),
            trees: Vec::new(),
            own_tree: None,
            warning: None,
        };
        let Some(context) = MoveContext::new(self, head) else {
            return choices;
        };
        let mut stack = self
            .current()
            .and_then(|id| self.tree_of(id))
            .map(|t| vec![t.root()])
            .unwrap_or_default();
        while let Some(id) = stack.pop() {
            if context.resolve(MoveTo::Page(id)).is_ok() {
                choices.parents.insert(id);
            }
            stack.extend(self.children(id));
        }
        for tree in self.open() {
            if context.resolve(MoveTo::Tree(tree.id)).is_ok() {
                choices.trees.push(TreeMoveChoice {
                    tree: tree.id,
                    title: tree.title.clone(),
                });
            }
        }
        choices.own_tree = if context.parent.is_none() {
            self.open()
                .iter()
                .any(|t| t.id == context.source)
                .then_some(OwnTreeMove::Reorder(context.source))
        } else {
            self.can_detach_branch(head).then_some(OwnTreeMove::Detach)
        };
        if context.parent.is_none() {
            let title = &self
                .trees()
                .iter()
                .find(|t| t.id == context.source)
                .unwrap()
                .title;
            choices.warning = Some(format!(
                "Merging \"{title}\" drops its name, saved state and tab. Its pages are kept."
            ));
        }
        choices
    }
}

impl Browser {
    pub fn move_branch(&mut self, head: VisitId, to: MoveTo) -> Result<MoveOutcome> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let before = forest_on(&tx)?;
        let Some(context) = MoveContext::new(&before, head) else {
            return Ok(MoveOutcome::Unchanged);
        };
        let Ok(action) = context.resolve(to) else {
            return Ok(MoveOutcome::Unchanged);
        };
        let current = before.current().filter(|id| context.branch.contains(id));
        let outcome = match action {
            ResolvedMove::Reorder { tree, place } => {
                place_tab(&tx, tree, place)?;
                MoveOutcome::TabReordered
            }
            ResolvedMove::Attach {
                parent,
                destination,
                source,
            } => {
                tx.execute(
                    "UPDATE visits SET parent_id=?2, branch_settled_at=NULL WHERE id=?1",
                    params![head.to_string(), parent.to_string()],
                )?;
                match source {
                    AttachSource::Remaining(source) => repair_source(&tx, source)?,
                    AttachSource::WholeTree(tree) => {
                        tx.execute(
                            "DELETE FROM tree_state WHERE root_id=?1",
                            [tree.root().to_string()],
                        )?;
                        tx.execute(
                            "DELETE FROM declined_suggestions WHERE key=?1",
                            [format!("tree {}", tree.root())],
                        )?;
                    }
                }
                if let Some(current) = current {
                    tx.execute(
                        "UPDATE tree_state SET last_visit_id=?2 WHERE root_id=?1",
                        params![destination.root().to_string(), current.to_string()],
                    )?;
                }
                MoveOutcome::BranchMoved { tree: destination }
            }
            ResolvedMove::Detach { source, place } => {
                let last = match current {
                    Some(id) => id,
                    None => newest_opened(&tx, head)?,
                };
                tx.execute(
                    "UPDATE visits SET parent_id=NULL, branch_settled_at=NULL WHERE id=?1",
                    [head.to_string()],
                )?;
                repair_source(&tx, source)?;
                tx.execute("INSERT INTO tree_state(root_id,last_visit_id,tab_order) VALUES(?1,?2,(SELECT COALESCE(MAX(tab_order), -1)+1 FROM tree_state))", params![head.to_string(),last.to_string()])?;
                place_tab(&tx, TreeId(head), place)?;
                MoveOutcome::BranchMoved { tree: TreeId(head) }
            }
        };
        forest_on(&tx)?;
        tx.commit()?;
        Ok(outcome)
    }
}

fn newest_opened(tx: &Transaction<'_>, head: VisitId) -> Result<VisitId> {
    let id: String = tx.query_row("WITH RECURSIVE branch(id) AS (SELECT ?1 UNION SELECT v.id FROM visits v JOIN branch b ON v.parent_id=b.id)
        SELECT v.id FROM visits v JOIN branch b ON v.id=b.id WHERE v.opened_at IS NOT NULL
        ORDER BY (SELECT MAX(e.id) FROM visit_events e WHERE e.visit_id=v.id) DESC, v.opened_at DESC, v.rowid DESC LIMIT 1", [head.to_string()], |r| r.get(0))?;
    Ok(id.parse()?)
}

fn repair_source(tx: &Transaction<'_>, source: SurvivingSource) -> Result<()> {
    if let SourceLast::Retarget = source.last {
        let last = newest_opened(tx, source.tree.root())?;
        tx.execute(
            "UPDATE tree_state SET last_visit_id=?2 WHERE root_id=?1",
            params![source.tree.root().to_string(), last.to_string()],
        )?;
    }
    Ok(())
}

fn placed_order(order: &[TreeId], tree: TreeId, place: TabPlacement) -> Vec<TreeId> {
    if place == TabPlacement::Before(tree) {
        return order.to_vec();
    }
    let mut next: Vec<_> = order.iter().copied().filter(|id| *id != tree).collect();
    let index = match place {
        TabPlacement::End => next.len(),
        TabPlacement::Before(anchor) => next.iter().position(|id| *id == anchor).unwrap(),
    };
    next.insert(index, tree);
    next
}

fn place_tab(tx: &Transaction<'_>, tree: TreeId, place: TabPlacement) -> Result<()> {
    let order = tx
        .prepare("SELECT root_id FROM tree_state WHERE settled_at IS NULL ORDER BY tab_order")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let ids: Vec<_> = order
        .iter()
        .map(|id| id.parse().map(TreeId))
        .collect::<std::result::Result<_, _>>()?;
    let next = placed_order(&ids, tree, place);
    if next != ids {
        write_tab_order(
            tx,
            &next
                .iter()
                .map(|t| t.root().to_string())
                .collect::<Vec<_>>(),
        )?;
    }
    Ok(())
}

pub(super) fn write_tab_order(tx: &Transaction<'_>, order: &[String]) -> Result<()> {
    for (index, id) in order.iter().enumerate() {
        tx.execute(
            "UPDATE tree_state SET tab_order=?2 WHERE root_id=?1 AND tab_order<>?2",
            params![id, index],
        )?;
    }
    Ok(())
}
