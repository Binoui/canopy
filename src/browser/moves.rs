use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{params, Transaction};

use super::{forest_on, now_ms, places, Browser, Move, Place, Slot, TreeId, VisitId, VisitState};
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
    Detach(SurvivingSource),
    Reorder {
        tree: TreeId,
        slot: Slot,
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
        let last = self.forest.tree(self.source).unwrap().last_visit;
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
                let target = self.forest.tree(tree).ok_or(Refusal::Gone)?;
                if !target.place.is_tab() {
                    return Err(Refusal::PutAway);
                }
                self.attach(tree.root())
            }
            MoveTo::OwnTree(place) if self.parent.is_some() => {
                if place != TabPlacement::End {
                    return Err(Refusal::BadAnchor);
                }
                detach_allowed(self.forest, self.head)?;
                Ok(ResolvedMove::Detach(self.surviving_source(None)))
            }
            MoveTo::OwnTree(place) => {
                let tabs = self.forest.tabs();
                let index = tabs
                    .iter()
                    .position(|tab| tab.id == self.source)
                    .ok_or(Refusal::PutAway)?;
                let before = match place {
                    TabPlacement::End => None,
                    TabPlacement::Before(anchor) => {
                        if !tabs.iter().any(|tab| tab.id == anchor) {
                            return Err(Refusal::BadAnchor);
                        }
                        Some(anchor)
                    }
                };
                if before == Some(self.source) || before == tabs.get(index + 1).map(|tab| tab.id) {
                    return Err(Refusal::AlreadyThere);
                }
                Ok(ResolvedMove::Reorder {
                    tree: self.source,
                    slot: before.map_or(Slot::End, Slot::Before),
                })
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
        if self.forest.is_settled(parent) || !self.forest.tree(destination).unwrap().place.is_tab()
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
        for tree in self.tabs() {
            if context.resolve(MoveTo::Tree(tree.id)).is_ok() {
                choices.trees.push(TreeMoveChoice {
                    tree: tree.id,
                    title: tree.title.clone(),
                });
            }
        }
        choices.own_tree = if context.parent.is_none() {
            self.tabs()
                .iter()
                .any(|t| t.id == context.source)
                .then_some(OwnTreeMove::Reorder(context.source))
        } else {
            self.can_detach_branch(head).then_some(OwnTreeMove::Detach)
        };
        if context.parent.is_none() {
            let tree = self.tree(context.source).unwrap();
            let place = match tree.place {
                Place::Tab { pinned: true } => "pinned tab",
                Place::Tab { pinned: false } => "tab",
                Place::Shelf { .. } => "place on the shelf",
                Place::History { .. } => "History entry",
            };
            choices.warning = Some(format!(
                "Merging \"{}\" drops its name and {place}. Its pages are kept.",
                tree.title
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
            ResolvedMove::Reorder { tree, slot } => {
                places::place_in(&tx, tree, Move::Tabs(slot), now_ms()?)?;
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
            ResolvedMove::Detach(source) => {
                let last = match current {
                    Some(id) => id,
                    None => newest_opened(&tx, head)?,
                };
                tx.execute(
                    "UPDATE visits SET parent_id=NULL, branch_settled_at=NULL WHERE id=?1",
                    [head.to_string()],
                )?;
                repair_source(&tx, source)?;
                places::append_tab(&tx, head, last, now_ms()?)?;
                MoveOutcome::BranchMoved { tree: TreeId(head) }
            }
        };
        places::compact_tabs(&tx)?;
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
