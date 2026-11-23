use crate::{browser::VisitId, forest::ForestProjection};

use super::waiting::Waiting;

const WAITING_LIMIT: usize = 5;
pub(super) const READ_LIMIT: usize = 8;

pub(super) struct SwitchChoices {
    pub(super) here: Option<VisitId>,
    pub(super) waiting: Vec<VisitId>,
    pub(super) read: Vec<VisitId>,
    pub(super) more_waiting: usize,
}

impl SwitchChoices {
    pub(super) fn of(forest: &ForestProjection, recent: &[VisitId]) -> Self {
        let waiting = Waiting::of(forest);
        Self {
            here: forest.current(),
            waiting: waiting
                .pages()
                .iter()
                .take(WAITING_LIMIT)
                .map(|v| v.id)
                .collect(),
            read: recent
                .iter()
                .copied()
                .filter(|id| forest.visit(*id).is_some())
                .take(READ_LIMIT)
                .collect(),
            more_waiting: waiting.len().saturating_sub(WAITING_LIMIT),
        }
    }

    pub(super) fn first(&self) -> Option<VisitId> {
        self.waiting.first().or_else(|| self.read.first()).copied()
    }
}
