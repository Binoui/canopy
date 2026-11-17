use super::{Issue, LivePage, NativeState};
use crate::{
    browser::{LoadTicket, VisitId},
    pages::{Command, PageEvent, Pages},
};
use std::rc::Rc;
use webkit2gtk::WebViewExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Progress(u8);

impl Progress {
    pub fn percent(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadProgress {
    Waiting,
    Measured(Progress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadStatus {
    Idle,
    Busy(LoadProgress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadingReadout {
    pub page: Option<VisitId>,
    pub status: LoadStatus,
}

#[derive(Default)]
pub(super) enum Activity {
    #[default]
    Quiet,
    Attempt {
        ticket: LoadTicket,
        phase: Phase,
    },
}

pub(super) enum Phase {
    Waiting,
    Running,
}

impl Activity {
    pub(super) fn begin(&mut self, ticket: LoadTicket) {
        if !matches!(self, Self::Attempt { ticket: active, .. } if *active == ticket) {
            *self = Self::Attempt {
                ticket,
                phase: Phase::Waiting,
            };
        }
    }

    pub(super) fn started(&mut self, ticket: LoadTicket) {
        if matches!(self, Self::Quiet)
            || matches!(self, Self::Attempt { ticket: active, .. } if *active == ticket)
        {
            *self = Self::Attempt {
                ticket,
                phase: Phase::Running,
            };
        }
    }

    pub(super) fn finish(&mut self, ticket: LoadTicket) {
        if matches!(self, Self::Attempt { ticket: active, .. } if *active == ticket) {
            self.cancel();
        }
    }

    pub(super) fn cancel(&mut self) {
        *self = Self::Quiet;
    }

    fn status(&self, running: Option<LoadTicket>, progress: f64) -> LoadStatus {
        match self {
            Self::Quiet => LoadStatus::Idle,
            Self::Attempt {
                ticket,
                phase: Phase::Running,
            } if running == Some(*ticket) && progress.is_finite() => LoadStatus::Busy(
                LoadProgress::Measured(Progress((progress.clamp(0.0, 1.0) * 100.0).round() as u8)),
            ),
            Self::Attempt { .. } => LoadStatus::Busy(LoadProgress::Waiting),
        }
    }
}

impl NativeState {
    pub(super) fn begin_loading(&self, ticket: LoadTicket) {
        self.loading.borrow_mut().begin(ticket);
        self.schedule_loading();
    }

    pub(super) fn finish_loading(&self, ticket: LoadTicket) {
        self.loading.borrow_mut().finish(ticket);
        self.schedule_loading();
    }

    pub(super) fn cancel_loading(&self) {
        self.loading.borrow_mut().cancel();
        self.schedule_loading();
    }

    fn schedule_loading(&self) {
        if let Some(owner) = self.pages.upgrade() {
            owner.publish_loading();
        }
    }

    fn load_status(&self) -> LoadStatus {
        let running = match *self.issue.borrow() {
            Issue::Loading(ticket) => Some(ticket),
            _ => None,
        };
        self.loading
            .borrow()
            .status(running, self.native.estimated_load_progress())
    }
}

pub(super) fn setup(state: &Rc<NativeState>) -> glib::SignalHandlerId {
    let weak = Rc::downgrade(state);
    state
        .native
        .connect_estimated_load_progress_notify(move |_| {
            if let Some(state) = Pages::enter_native(&weak) {
                state.schedule_loading();
            }
        })
}

impl Pages {
    #[cfg(feature = "gui-smoke")]
    pub fn foreground_loading_view(&self) -> Option<webkit2gtk::WebView> {
        self.foreground_load().map(|page| page.native.clone())
    }

    fn foreground_load(&self) -> Option<Rc<LivePage>> {
        self.branches
            .candidate
            .borrow()
            .clone()
            .or_else(|| self.branches.live.borrow().clone())
    }

    fn queued_navigation(&self) -> bool {
        self.commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::Go(..) | Command::Reload))
    }

    fn loading_readout(&self) -> LoadingReadout {
        let status = self
            .foreground_load()
            .map_or(LoadStatus::Idle, |p| p.state.load_status());
        LoadingReadout {
            page: self
                .branches
                .live
                .borrow()
                .as_ref()
                .and_then(|p| p.current_visit()),
            status: if status == LoadStatus::Idle && self.queued_navigation() {
                LoadStatus::Busy(LoadProgress::Waiting)
            } else {
                status
            },
        }
    }

    pub fn stop(&self) -> bool {
        let page = self.foreground_load();
        let busy = page
            .as_ref()
            .is_some_and(|p| p.state.load_status() != LoadStatus::Idle);
        if !busy && !self.queued_navigation() {
            return false;
        }
        self.commands
            .borrow_mut()
            .retain(|c| !matches!(c, Command::Go(..) | Command::Reload));
        if let Some(page) = &page {
            page.cancel_foreground();
        }
        self.cancel_continuation();
        let candidate = self.branches.candidate.borrow().clone();
        if let Some(candidate) = candidate {
            // Normal switch cancellation restores an adopted preload. User Stop ends it.
            let background = candidate.background_visit().map(|_| candidate.view());
            self.cancel_candidate();
            if let Some(view) = background {
                self.drop_background(view);
            }
        }
        self.publish_loading();
        true
    }

    pub(in crate::pages) fn publish_loading(&self) {
        if self.loading_publish_scheduled.replace(true) {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            let Some(pages) = weak.upgrade() else { return };
            pages.loading_publish_scheduled.set(false);
            let value = pages.loading_readout();
            pages.receive(PageEvent::LoadState(value));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Navigation;
    use url::Url;

    #[test]
    fn replacement_ignores_old_terminal_and_old_progress() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.com").unwrap();
        let a = navigation.native_request(url.clone());
        let b = navigation.native_request(url);
        let mut activity = Activity::default();
        activity.begin(a);
        activity.started(a);
        activity.begin(a);
        assert_eq!(
            activity.status(Some(a), 0.42),
            LoadStatus::Busy(LoadProgress::Measured(Progress(42)))
        );
        activity.begin(b);
        activity.finish(a);
        activity.started(a);
        assert_eq!(
            activity.status(Some(a), 0.9),
            LoadStatus::Busy(LoadProgress::Waiting)
        );
        activity.started(b);
        activity.finish(a);
        assert_eq!(
            activity.status(Some(b), 0.5),
            LoadStatus::Busy(LoadProgress::Measured(Progress(50)))
        );
        activity.finish(b);
        assert_eq!(activity.status(None, 1.0), LoadStatus::Idle);
    }

    #[test]
    fn cancellation_needs_no_native_terminal() {
        let mut navigation = Navigation::default();
        let ticket = navigation.native_request(Url::parse("https://example.com").unwrap());
        let mut activity = Activity::default();
        activity.begin(ticket);
        activity.cancel();
        activity.cancel();
        assert_eq!(activity.status(Some(ticket), 0.4), LoadStatus::Idle);
    }

    #[test]
    fn native_progress_is_bounded_and_non_finite_is_waiting() {
        let mut navigation = Navigation::default();
        let ticket = navigation.native_request(Url::parse("https://example.com").unwrap());
        let mut activity = Activity::default();
        activity.started(ticket);
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                activity.status(Some(ticket), value),
                LoadStatus::Busy(LoadProgress::Waiting)
            );
        }
        assert_eq!(
            activity.status(Some(ticket), -1.0),
            LoadStatus::Busy(LoadProgress::Measured(Progress(0)))
        );
        assert_eq!(
            activity.status(Some(ticket), 2.0),
            LoadStatus::Busy(LoadProgress::Measured(Progress(100)))
        );
    }
}
