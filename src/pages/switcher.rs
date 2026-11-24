use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::browser::VisitId;

use super::{LivePage, Pages};

#[derive(Clone)]
pub(crate) struct SwitcherSource {
    owner: Weak<Pages>,
    page: Weak<LivePage>,
    visit: VisitId,
    epoch: u64,
    sequence: u64,
}

impl Pages {
    fn switcher_page(&self, visit: VisitId) -> Option<Rc<LivePage>> {
        let live = self.branches.live.borrow();
        let hidden = self.branches.hidden.borrow();
        let background = self.branches.background.borrow();
        let hold = self.hold.borrow();
        live.iter()
            .chain(hidden.iter().map(|h| &h.page))
            .chain(background.iter().map(|h| &h.page))
            .chain(hold.iter().map(|h| &h.outgoing))
            .find(|page| page.current_visit() == Some(visit))
            .cloned()
    }

    pub(crate) fn switcher_source(&self, visit: VisitId) -> Option<SwitcherSource> {
        let page = self.switcher_page(visit)?;
        let (document, sequence) = page.capture_document()?;
        Some(SwitcherSource {
            owner: self.me.clone(),
            page: Rc::downgrade(&page),
            visit: document.visit,
            epoch: document.epoch,
            sequence,
        })
    }
}

impl SwitcherSource {
    fn page(&self) -> Option<Rc<LivePage>> {
        let page = self.page.upgrade()?;
        let retained = self.owner.upgrade()?.switcher_page(self.visit)?;
        if !Rc::ptr_eq(&page, &retained) {
            return None;
        }
        let (document, sequence) = page.capture_document()?;
        (document.visit == self.visit && document.epoch == self.epoch && sequence == self.sequence)
            .then_some(page)
    }

    pub(crate) fn retained(&self) -> bool {
        self.page().is_some()
    }

    pub(crate) fn allocation(&self) -> Option<(u64, i32)> {
        let page = self.page()?;
        let widget = page.native.widget();
        let scale = widget.scale_factor();
        let rect = widget.allocation();
        let bytes = u64::try_from(rect.width()).ok()?
            * u64::try_from(rect.height()).ok()?
            * (scale as u64).pow(2)
            * 4;
        Some((bytes, scale))
    }

    pub(crate) fn snapshot(
        &self,
        cancel: &gio::Cancellable,
        done: impl FnOnce(Option<gtk::cairo::Surface>) + 'static,
    ) {
        let Some(page) = self.page() else {
            done(None);
            return;
        };
        let source = self.clone();
        page.native.snapshot(
            Some(cancel),
            Box::new(move |result| {
                done(result.ok().filter(|_| source.retained()));
            }),
        );
    }
}
