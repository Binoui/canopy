mod live;
mod native;

use crate::{
    address::WebOrigin,
    browser::{Browser, Landing, LoadTicket, NavigationTarget, ThemeMode, TreeId, VisitId},
    chrome::{Favicons, CHROME_HEIGHT},
    forest::ForestProjection,
    logins::Login,
    secret::Secret,
};
use anyhow::Result;
use gtk::prelude::*;
use live::LivePage;
pub use native::{CookieApplyError, DownloadEvent};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};
use url::Url;
use webkit2gtk::WebViewExt;

pub enum Go {
    Visit(VisitId),
    Parent,
    Tree(TreeId),
    Address { url: Url, target: NavigationTarget },
    Citation(VisitId),
    Saved(i64),
}

pub enum PageEvent {
    Changed,
    Loading(bool),
    Error(String),
    Remembered(i64),
    Download(DownloadEvent),
    OpenBranchPalette { visit: VisitId },
    Login(LoginEvent),
}

/// What the selected page reports about signing in.
pub enum LoginEvent {
    /// A new document replaced the page. Its fields are not known yet.
    NewDocument,
    /// The document on screen at `page` reported its fields: whether a saved user name or
    /// password can be filled, and whether it shows any password field at all.
    Fields {
        page: Url,
        username: bool,
        password: bool,
        asks_password: bool,
    },
    /// The user sent this login to the page.
    Sent(Login),
}

enum Continuation {
    BranchPalette,
    Path(Url),
    Saved(i64),
}
struct PendingContinuation {
    state: ContinuationState,
    action: Continuation,
}

#[derive(Clone, Copy)]
enum ContinuationState {
    Waiting(LoadTicket),
    Committed { ticket: LoadTicket, visit: VisitId },
    Ready { ticket: LoadTicket, visit: VisitId },
}

enum ContinuationEvent {
    Commit {
        ticket: LoadTicket,
        visit: VisitId,
        same_document: bool,
    },
    Finished,
    Pending(Option<LoadTicket>),
    Failed,
    Discard,
}

impl ContinuationState {
    fn ticket(self) -> LoadTicket {
        match self {
            Self::Waiting(ticket) | Self::Committed { ticket, .. } | Self::Ready { ticket, .. } => {
                ticket
            }
        }
    }

    fn transition(mut self, event: ContinuationEvent) -> Option<Self> {
        match event {
            ContinuationEvent::Commit {
                ticket,
                visit,
                same_document,
            } => {
                if !matches!(self, Self::Ready { .. })
                    && !self.committed(ticket, visit, same_document)
                {
                    return None;
                }
            }
            ContinuationEvent::Finished => {
                self.finished();
            }
            ContinuationEvent::Pending(pending) if self.invalidated_by(pending) => return None,
            ContinuationEvent::Failed if !matches!(self, Self::Ready { .. }) => return None,
            ContinuationEvent::Discard => return None,
            _ => {}
        }
        Some(self)
    }

    fn committed(&mut self, ticket: LoadTicket, visit: VisitId, same_document: bool) -> bool {
        if !matches!(*self, Self::Waiting(waiting) if waiting == ticket) {
            return false;
        }
        *self = if same_document {
            Self::Ready { ticket, visit }
        } else {
            Self::Committed { ticket, visit }
        };
        true
    }
    fn finished(&mut self) -> Option<LoadTicket> {
        if let Self::Committed { ticket, visit } = *self {
            *self = Self::Ready { ticket, visit };
            Some(ticket)
        } else {
            None
        }
    }

    fn invalidated_by(&self, pending: Option<LoadTicket>) -> bool {
        match *self {
            Self::Waiting(ticket) => pending != Some(ticket),
            Self::Committed { ticket, .. } => pending.is_some_and(|next| next != ticket),
            Self::Ready { .. } => false,
        }
    }
}

#[derive(Clone, Copy)]
enum StartPage {
    Reload,
    Show(VisitId),
    Blank,
}

type Observer = Rc<dyn Fn(PageEvent)>;
#[cfg(feature = "gui-smoke")]
type PlacementObserver = Rc<dyn Fn(&webkit2gtk::WebView)>;

enum Command {
    Go(Go, Option<Continuation>),
    Settle(TreeId),
    Reload,
}

struct Branches {
    live: RefCell<Option<Rc<LivePage>>>,
    dormant: RefCell<Vec<live::Checkpoint>>,
    candidate: RefCell<Option<Rc<LivePage>>>,
    candidate_saved: RefCell<Option<(usize, live::Checkpoint)>>,
}

pub struct Pages {
    me: Weak<Pages>,
    browser: RefCell<Browser>,
    context: Rc<native::PageContext>,
    branches: Branches,
    container: gtk::Fixed,
    viewport: Cell<(i32, i32)>,
    views: RefCell<Vec<glib::WeakRef<webkit2gtk::WebView>>>,
    chrome: RefCell<Option<glib::WeakRef<webkit2gtk::WebView>>>,
    commands: RefCell<std::collections::VecDeque<Command>>,
    #[cfg(feature = "gui-smoke")]
    checkpoint: RefCell<Option<live::Checkpoint>>,
    #[cfg(feature = "gui-smoke")]
    before_revival: RefCell<Option<Rc<dyn Fn()>>>,
    #[cfg(feature = "gui-smoke")]
    placement_observer: RefCell<Option<PlacementObserver>>,
    releasing: Cell<bool>,
    continuation: RefCell<Option<PendingContinuation>>,
    observer: RefCell<Option<Observer>>,
    start: Cell<Option<StartPage>>,
}

impl Pages {
    pub fn mount(container: &gtk::Fixed, browser: Browser, profile: &Path) -> Result<Rc<Self>> {
        let context = native::PageContext::new(profile)?;
        let pages = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            browser: RefCell::new(browser),
            context,
            branches: Branches {
                live: RefCell::new(None),
                dormant: RefCell::new(Vec::new()),
                candidate: RefCell::new(None),
                candidate_saved: RefCell::new(None),
            },
            container: container.clone(),
            viewport: Cell::new((container.allocated_width(), container.allocated_height())),
            views: RefCell::new(Vec::new()),
            chrome: RefCell::new(None),
            commands: RefCell::new(std::collections::VecDeque::new()),
            #[cfg(feature = "gui-smoke")]
            checkpoint: RefCell::new(None),
            #[cfg(feature = "gui-smoke")]
            before_revival: RefCell::new(None),
            #[cfg(feature = "gui-smoke")]
            placement_observer: RefCell::new(None),
            releasing: Cell::new(false),
            continuation: RefCell::new(None),
            observer: RefCell::new(None),
            start: Cell::new(None),
        });
        let weak = Rc::downgrade(&pages);
        container.connect_size_allocate(move |_, a| {
            if let Some(pages) = weak.upgrade() {
                pages.resize(a.width() as f64, a.height() as f64);
            }
        });
        let page = LivePage::new(container, Rc::downgrade(&pages), &pages.context)?;

        *pages.branches.live.borrow_mut() = Some(Rc::new(page));
        let weak = Rc::downgrade(&pages);
        pages.context.observe(move |event| {
            if let Some(pages) = weak.upgrade() {
                pages.receive(event);
            }
        });
        let weak = Rc::downgrade(&pages);
        pages
            .context
            .record_downloads(move |visit, path, size, source| {
                let pages = weak
                    .upgrade()
                    .ok_or_else(|| anyhow::anyhow!("The window was closed."))?;
                let result = pages
                    .browser
                    .borrow_mut()
                    .record_download(visit, path, size, source);
                result.map(|_| ())
            });
        Ok(pages)
    }

    #[cfg(feature = "gui-smoke")]
    fn page(&self) -> Rc<LivePage> {
        self.branches
            .live
            .borrow()
            .as_ref()
            .expect("mounted page")
            .clone()
    }

    pub fn read<T>(&self, query: impl FnOnce(&Browser) -> T) -> T {
        query(&self.browser.borrow())
    }

    pub fn favicons(&self) -> Rc<Favicons> {
        self.context.favicons.clone()
    }

    pub fn sandbox_enabled(&self) -> bool {
        use webkit2gtk::WebContextExt;
        self.context.native.is_sandbox_enabled()
    }

    pub fn observe(&self, callback: impl Fn(PageEvent) + 'static) {
        *self.observer.borrow_mut() = Some(Rc::new(callback));
    }

    // Clone the observer and end every state borrow before WebKit, GTK or Ui can reenter.
    fn receive(&self, event: PageEvent) {
        let continuation_selected = self.continuation.borrow().as_ref().is_none_or(|c| {
            self.branches
                .live
                .borrow()
                .as_ref()
                .is_some_and(|page| page.owns(c.state.ticket()))
        });
        if continuation_selected {
            if matches!(event, PageEvent::Error(_)) {
                self.advance_continuation(ContinuationEvent::Failed);
            }
            if matches!(event, PageEvent::Loading(false)) {
                if let Some(ticket) = self.advance_continuation(ContinuationEvent::Finished) {
                    self.schedule_continuation(ticket);
                }
            }
            let pending = self
                .branches
                .live
                .borrow()
                .as_ref()
                .and_then(|page| page.pending_ticket());
            self.advance_continuation(ContinuationEvent::Pending(pending));
        }
        let observer = self.observer.borrow().clone();
        if let Some(observer) = observer {
            observer(event);
        }
    }

    fn advance_continuation(&self, event: ContinuationEvent) -> Option<LoadTicket> {
        let mut pending = self.continuation.borrow_mut();
        let c = pending.as_mut()?;
        if let Some(state) = c.state.transition(event) {
            c.state = state;
            if let ContinuationState::Ready { ticket, .. } = state {
                return Some(ticket);
            }
        } else {
            pending.take();
        }
        None
    }

    fn committed(self: &Rc<Self>, ticket: LoadTicket, visit: VisitId) {
        self.select_candidate(ticket);
        let same_document = self
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|page| !page.native.is_loading());
        if let Some(ticket) = self.advance_continuation(ContinuationEvent::Commit {
            ticket,
            visit,
            same_document,
        }) {
            self.schedule_continuation(ticket);
        }
    }

    fn schedule_continuation(&self, ticket: LoadTicket) {
        // Native commit handling must finish binding before the continuation runs.
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            let Some(pages) = weak.upgrade() else { return };
            if pages.branches.live.borrow().is_none() {
                return;
            }
            let continuation = {
                let mut pending = pages.continuation.borrow_mut();
                if pending.as_ref().is_some_and(|c| matches!(c.state, ContinuationState::Ready { ticket: committed, .. } if committed == ticket)) {
                    pending.take()
                } else { None }
            };
            let Some(PendingContinuation {
                state: ContinuationState::Ready { visit, .. },
                action,
            }) = continuation
            else {
                return;
            };
            match action {
                Continuation::BranchPalette => {
                    pages.receive(PageEvent::OpenBranchPalette { visit })
                }
                Continuation::Saved(id) => {
                    if let Err(error) = pages.go(Go::Saved(id)) {
                        pages.receive(PageEvent::Error(error.to_string()));
                    }
                }
                Continuation::Path(second) => {
                    let result = pages.go(Go::Address {
                        url: second,
                        target: NavigationTarget::ChildOf(visit),
                    });
                    if let Err(error) = result {
                        pages.receive(PageEvent::Error(error.to_string()));
                    }
                }
            }
        });
    }

    pub fn cancel_path_continuation(&self) {
        if self
            .continuation
            .borrow()
            .as_ref()
            .is_some_and(|c| matches!(c.action, Continuation::Path(_)))
        {
            self.cancel_continuation();
        }
    }

    pub fn cancel_continuation(&self) {
        self.advance_continuation(ContinuationEvent::Discard);
    }

    fn cancel_candidate(&self) {
        let page = self.branches.candidate.borrow_mut().take();
        if let Some(page) = page {
            page.cancel();
        }
        if let Some((index, saved)) = self.branches.candidate_saved.borrow_mut().take() {
            let mut dormant = self.branches.dormant.borrow_mut();
            let index = index.min(dormant.len());
            dormant.insert(index, saved);
        }
    }

    fn candidate_page(&self, visit: Option<VisitId>) -> Result<Rc<LivePage>> {
        self.cancel_candidate();
        let selected = self.branches.live.borrow().clone();
        if let Some(selected) = selected {
            selected.cancel_foreground();
        }
        let saved = visit.and_then(|visit| {
            let mut dormant = self.branches.dormant.borrow_mut();
            let index = dormant
                .iter()
                .rposition(|saved| saved.entries.iter().any(|e| e.visit == visit))?;
            // A noncurrent restored POST cannot be replaced in WebKit's list by GET.
            // Keep this checkpoint intact and open that visit in a fresh branch instead.
            let saved = &dormant[index];
            if saved.entries.iter().any(|e| e.visit == visit && e.post)
                && saved
                    .entries
                    .get(saved.current)
                    .is_none_or(|e| e.visit != visit)
            {
                return None;
            }
            Some((index, dormant.remove(index)))
        });
        let page = match (saved.as_ref().map(|(_, saved)| saved), visit) {
            (Some(saved), Some(visit)) => {
                LivePage::revive(
                    &self.container,
                    self.me.clone(),
                    &self.context,
                    saved,
                    visit,
                )?
                .0
            }
            _ => LivePage::empty(&self.container, self.me.clone(), &self.context)?,
        };
        *self.branches.candidate_saved.borrow_mut() = saved;
        page.native.set_child_visible(false);

        let page = Rc::new(page);
        *self.branches.candidate.borrow_mut() = Some(page.clone());
        Ok(page)
    }

    fn cold_restore(&self, visit: VisitId) -> Result<()> {
        self.cancel_candidate();
        let selected = self.branches.live.borrow().clone();
        if let Some(selected) = selected {
            selected.cancel_foreground();
        }
        let page = Rc::new(LivePage::empty(
            &self.container,
            self.me.clone(),
            &self.context,
        )?);
        page.native.set_child_visible(false);

        *self.branches.candidate.borrow_mut() = Some(page.clone());
        page.restore(visit)
    }

    fn route(&self, visit: VisitId) -> Result<()> {
        let candidate = self.branches.candidate.borrow().clone();
        if candidate
            .as_ref()
            .is_some_and(|p| p.pending_visit() == Some(visit))
        {
            return Ok(());
        }
        self.cancel_candidate();
        let selected = self.branches.live.borrow().clone();
        if let Some(page) = selected {
            if page.current_visit() == Some(visit) || page.pending_visit() == Some(visit) {
                return Ok(());
            }
            if self.read(|b| b.visit(visit))?.opened_at.is_some() && page.contains(visit) {
                return if page.requires_get(visit) {
                    self.cold_restore(visit)
                } else {
                    page.restore(visit)
                };
            }
        }
        self.candidate_page(Some(visit))?.restore_revived(visit)
    }

    fn select_candidate(&self, ticket: LoadTicket) {
        let candidate = self.branches.candidate.borrow().clone();
        let Some(candidate) = candidate.filter(|p| p.owns(ticket)) else {
            return;
        };
        self.branches.candidate.borrow_mut().take();
        self.branches.candidate_saved.borrow_mut().take();
        let old = self.branches.live.borrow_mut().replace(candidate.clone());
        let focus = old.as_ref().is_some_and(|page| page.native.has_focus());
        if let Some(old) = old {
            self.releasing.set(true);
            if let Some(window) = old.native.window() {
                window.set_pass_through(true);
            }
            let pending = RefCell::new(Some(old.clone()));
            let cancel = gio::Cancellable::new();
            let stop = cancel.clone();
            let weak = self.me.clone();
            #[cfg(feature = "gui-smoke")]
            let started = std::time::Instant::now();
            let finish: Rc<dyn Fn(&str)> = Rc::new(move |_reason| {
                if let Some(old) = pending.borrow_mut().take() {
                    stop.cancel();
                    #[cfg(feature = "gui-smoke")]
                    eprintln!("PICTURE {_reason} {}ms", started.elapsed().as_millis());
                    if let Some(pages) = weak.upgrade() {
                        pages.release_picture(old);
                    }
                }
            });
            let painted = finish.clone();
            let view = candidate.native.clone();
            let snapshot_cancel = cancel.clone();
            candidate.native.call_async_javascript_function(
                "while (!document.body) await new Promise(r => requestAnimationFrame(r)); await new Promise(r => requestAnimationFrame(r)); await new Promise(r => requestAnimationFrame(r)); return true;",
                None, Some("canopy-picture"), None, Some(&cancel),
                move |result| {
                    if result.is_ok() {
                        view.snapshot(webkit2gtk::SnapshotRegion::Visible, webkit2gtk::SnapshotOptions::NONE, Some(&snapshot_cancel), move |result| {
                            if result.is_ok() {
                                glib::timeout_add_local_once(std::time::Duration::from_millis(34), move || painted("snapshot"));
                            }
                        });
                    }
                },
            );
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                finish("cap")
            });
            candidate.show();
            if let Some(window) = old.native.window() {
                window.raise();
            }
            self.place_view(&candidate.native);
        } else {
            candidate.show();
        }
        if focus {
            candidate.focus();
        }
    }

    fn release_picture(&self, old: Rc<LivePage>) {
        old.native.set_child_visible(false);
        let weak = self.me.clone();
        let pending = old.clone();
        old.capture_checkpoint_scroll(move || {
            let saved = Self::release_page(pending);
            if let Some(pages) = weak.upgrade() {
                if let Ok(saved) = saved {
                    let open = pages
                        .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))
                        .is_ok_and(|forest| {
                            saved.entries.get(saved.current).is_some_and(|e| {
                                forest
                                    .tree_of(e.visit)
                                    .is_some_and(|tree| forest.open().iter().any(|t| t.id == tree))
                            })
                        });
                    if open {
                        pages.branches.dormant.borrow_mut().push(saved);
                    }
                }
                pages.releasing.set(false);
                loop {
                    let command = pages.commands.borrow_mut().pop_front();
                    let Some(command) = command else { break };
                    let result = match command {
                        Command::Go(go, continuation) => {
                            let result = pages.go(go);
                            if let Some(continuation) = continuation {
                                pages.continue_with(continuation);
                            }
                            result
                        }
                        Command::Settle(tree) => pages.settle_tree(tree),
                        Command::Reload => pages.reload(),
                    };
                    if let Err(error) = result {
                        pages.receive(PageEvent::Error(error.to_string()));
                    }
                    if pages.releasing.get() {
                        break;
                    }
                }
            }
        });
    }

    fn receive_view(&self, view: crate::browser::ViewId, empty: bool, event: PageEvent) {
        let candidate = self.branches.candidate.borrow().clone();
        if candidate.as_ref().is_some_and(|p| p.view() == view) {
            if matches!(event, PageEvent::Error(_))
                || matches!(event, PageEvent::Loading(false))
                    && empty
                    && !candidate.as_ref().unwrap().awaiting_download()
            {
                let failed = matches!(event, PageEvent::Error(_));
                self.cancel_candidate();
                self.receive(event);
                if failed {
                    self.receive(PageEvent::Loading(false));
                }
            }
            return;
        }
        let selected = self
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| p.view() == view);
        if selected {
            self.receive(event);
        }
    }

    pub fn go(&self, go: Go) -> Result<()> {
        if self.releasing.get() {
            self.commands.borrow_mut().push_back(Command::Go(go, None));
            return Ok(());
        }
        let page = self
            .branches
            .live
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No selected page."))?;
        if matches!(go, Go::Parent) {
            self.cancel_path_continuation();
        }
        let before = page.pending_ticket();
        let result = match go {
            Go::Visit(visit) => self.route(visit),
            Go::Parent => {
                if let Some(item) = page.native_back() {
                    self.cancel_candidate();
                    page.native.go_to_back_forward_list_item(&item);
                    Ok(())
                } else {
                    match page.back_visit()? {
                        Some(visit) => self.route(visit),
                        None => Ok(()),
                    }
                }
            }
            Go::Tree(tree) => {
                let visit = self.read(|b| b.tree_target(tree))?;
                self.route(visit)
            }
            Go::Address { url, target } => {
                if matches!(target, NavigationTarget::Root) && page.current_visit().is_some() {
                    self.candidate_page(None)?.navigate(url, target)
                } else {
                    self.cancel_candidate();
                    page.navigate(url, target)
                }
            }
            Go::Citation(visit) => {
                let parent = self.read(|b| b.visit(visit))?.parent;
                if parent.is_some() && parent == page.current_visit() {
                    page.open_citation(visit)
                } else {
                    self.candidate_page(None)?.open_citation(visit)
                }
            }
            Go::Saved(id) => {
                let capture = self.read(|b| b.capture(id))?;
                self.route(capture.visit)?;
                let target = capture
                    .citation_url
                    .as_ref()
                    .unwrap_or(&capture.document_url);
                if target != &self.read(|b| b.visit(capture.visit))?.url {
                    let selected = self.branches.live.borrow().clone();
                    if let Some(selected) =
                        selected.filter(|p| p.current_visit() == Some(capture.visit))
                    {
                        selected.open_saved(&capture)
                    } else {
                        self.continue_with(Continuation::Saved(id));
                        Ok(())
                    }
                } else {
                    Ok(())
                }
            }
        };
        if page.pending_ticket().is_some() && page.pending_ticket() != before {
            self.cancel_continuation();
        }
        result
    }

    pub fn branch_from_queued(&self, visit: VisitId) -> Result<()> {
        self.go(Go::Visit(visit))?;
        let pending = self
            .branches
            .candidate
            .borrow()
            .clone()
            .or_else(|| self.branches.live.borrow().clone())
            .and_then(|page| page.pending_ticket());
        if pending.is_none() && self.commands.borrow().is_empty() {
            self.receive(PageEvent::OpenBranchPalette { visit });
        } else {
            self.continue_with(Continuation::BranchPalette);
        }
        Ok(())
    }
    pub fn open_path(&self, first: Url, second: Url) -> Result<()> {
        self.go(Go::Address {
            url: first,
            target: NavigationTarget::Root,
        })?;
        self.continue_with(Continuation::Path(second));
        Ok(())
    }
    fn continue_with(&self, action: Continuation) {
        if let Some(Command::Go(_, continuation)) = self.commands.borrow_mut().back_mut() {
            *continuation = Some(action);
            return;
        }
        let page = self
            .branches
            .candidate
            .borrow()
            .clone()
            .or_else(|| self.branches.live.borrow().clone());
        *self.continuation.borrow_mut() =
            page.as_ref()
                .and_then(|page| page.pending_ticket())
                .map(|ticket| PendingContinuation {
                    state: ContinuationState::Waiting(ticket),
                    action,
                });
    }

    pub fn settle_tree(&self, tree: TreeId) -> Result<()> {
        if self.releasing.get() && self.branches.live.borrow().is_some() {
            self.commands.borrow_mut().push_back(Command::Settle(tree));
            return Ok(());
        }
        anyhow::ensure!(self.branches.live.borrow().is_some(), "No selected page.");
        self.cancel_continuation();
        let forest =
            self.read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
        let target = self
            .branches
            .candidate
            .borrow()
            .as_ref()
            .and_then(|p| p.pending_visit());
        if target.is_some_and(|visit| forest.tree_of(visit) == Some(tree)) {
            self.cancel_candidate();
        }
        self.branches.dormant.borrow_mut().retain(|saved| {
            !saved
                .entries
                .iter()
                .any(|e| forest.tree_of(e.visit) == Some(tree))
        });
        let landing = self.browser.borrow_mut().settle_tree(tree)?;
        self.receive(PageEvent::Changed);
        match landing {
            Landing::Unchanged => Ok(()),
            Landing::Restore(visit) => self.go(Go::Visit(visit)),
            Landing::Start => {
                let page = Rc::new(LivePage::new(
                    &self.container,
                    self.me.clone(),
                    &self.context,
                )?);

                let previous = self.branches.live.borrow_mut().replace(page.clone());
                drop(previous);
                page.show();
                Ok(())
            }
        }
    }

    pub fn prepare_resume(&self, urls: &[Url]) -> Result<()> {
        if !urls.is_empty() {
            self.start.set(Some(StartPage::Reload));
            return Ok(());
        }
        let forest =
            self.read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
        let mut start = StartPage::Reload;
        if let Some(tree) = forest.current().and_then(|id| forest.tree_of(id)) {
            if forest
                .trees()
                .iter()
                .any(|t| t.id == tree && !t.lifecycle.is_open())
            {
                start = match self.browser.borrow_mut().settle_tree(tree)? {
                    Landing::Restore(visit) => StartPage::Show(visit),
                    Landing::Start => StartPage::Blank,
                    Landing::Unchanged => StartPage::Reload,
                };
                self.receive(PageEvent::Changed);
            }
        }
        self.start.set(Some(start));
        Ok(())
    }
    pub fn resume(&self) -> Result<()> {
        if self.start.get().is_some() {
            anyhow::ensure!(self.branches.live.borrow().is_some(), "No selected page.");
        }
        match self.start.take() {
            Some(StartPage::Reload) => self.reload(),
            Some(StartPage::Show(visit)) => self.go(Go::Visit(visit)),
            Some(StartPage::Blank) => {
                self.cancel_continuation();
                let selected = self.branches.live.borrow().clone();
                if let Some(page) = selected {
                    page.blank();
                }
                Ok(())
            }
            None => Ok(()),
        }
    }
    pub fn open_roots(&self, urls: &[Url]) -> Result<()> {
        if !urls.is_empty() && self.start.get().is_none() {
            anyhow::ensure!(self.branches.live.borrow().is_some(), "No selected page.");
        }
        let roots = self.browser.borrow_mut().add_roots(urls)?;
        let Some(last) = roots.last().copied() else {
            return Ok(());
        };
        self.cancel_continuation();
        self.receive(PageEvent::Changed);
        if self.start.get().is_some() {
            self.start.set(Some(StartPage::Show(last)));
            Ok(())
        } else {
            self.go(Go::Visit(last))
        }
    }
    pub fn reload(&self) -> Result<()> {
        if self.releasing.get() && self.branches.live.borrow().is_some() {
            self.commands.borrow_mut().push_back(Command::Reload);
            return Ok(());
        }
        let page = self
            .branches
            .live
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No selected page."))?;
        self.cancel_path_continuation();
        let before = page.pending_ticket();
        let result = page.reload();
        if page.pending_ticket().is_some() && page.pending_ticket() != before {
            self.cancel_continuation();
        }
        result
    }
    pub fn remember_selection(&self) -> Result<()> {
        let page = self
            .branches
            .live
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No selected page."))?;
        page.remember_selection()
    }
    /// Types a saved login into the selected page, if it is still on `origin`.
    pub fn fill_login(
        &self,
        origin: &WebOrigin,
        username: &str,
        password: &Secret,
        done: impl FnOnce(Result<()>) + 'static,
    ) -> Result<()> {
        let page = self
            .branches
            .live
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No selected page."))?;
        page.fill_login(origin, username, password, done);
        Ok(())
    }
    pub fn set_capture_enabled(&self, enabled: bool) -> Result<()> {
        let page = self
            .branches
            .live
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No selected page."))?;
        page.set_capture_enabled(enabled)
    }
    pub fn set_theme(&self, mode: ThemeMode) -> Result<()> {
        self.browser.borrow_mut().set_theme_mode(mode)
    }
    pub fn rename_tree(&self, tree: TreeId, name: &str) -> Result<()> {
        self.browser.borrow_mut().rename_tree(tree, name)
    }
    pub fn focus(&self) {
        let selected = self.branches.live.borrow().clone();
        if let Some(page) = selected {
            page.focus();
        }
    }
    /// Every view uses the current content rectangle; page churn cannot size the chrome.
    fn place_view(&self, view: &webkit2gtk::WebView) {
        let (width, height) = self.viewport.get();
        let rect = gtk::Allocation::new(
            0,
            CHROME_HEIGHT as i32,
            width.max(1),
            (height - CHROME_HEIGHT as i32).max(1),
        );
        if view.size_request() != (rect.width(), rect.height()) {
            view.set_size_request(rect.width(), rect.height());
        }
        self.container.move_(view, rect.x(), rect.y());
        if view.allocation() != rect {
            view.size_allocate(&rect);
        }
        if let Some(window) = self
            .chrome
            .borrow()
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .and_then(|chrome| chrome.window())
        {
            window.raise();
        }
        #[cfg(feature = "gui-smoke")]
        if view.is_mapped() {
            let observer = self.placement_observer.borrow().clone();
            if let Some(observer) = observer {
                observer(view);
            }
        }
    }

    #[cfg(feature = "gui-smoke")]
    pub fn observe_page_placement(&self, observer: impl Fn(&webkit2gtk::WebView) + 'static) {
        *self.placement_observer.borrow_mut() = Some(Rc::new(observer));
    }

    fn register_view(&self, view: &webkit2gtk::WebView) {
        self.views.borrow_mut().push(view.downgrade());
        for map in [false, true] {
            let weak = self.me.clone();
            let place = move |view: &webkit2gtk::WebView| {
                if let Some(pages) = weak.upgrade() {
                    pages.place_view(view);
                }
            };
            if map {
                view.connect_map(place);
            } else {
                view.connect_show(place);
            }
        }
        self.place_view(view);
    }

    pub(crate) fn set_chrome(&self, chrome: &webkit2gtk::WebView) {
        *self.chrome.borrow_mut() = Some(chrome.downgrade());
    }

    pub fn resize(&self, width: f64, height: f64) {
        self.viewport
            .set((width.round() as i32, height.round() as i32));
        let views: Vec<_> = self
            .views
            .borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect();
        self.views.borrow_mut().retain(|w| w.upgrade().is_some());
        for view in views {
            self.place_view(&view);
        }
    }
    pub fn apply_cookie_batches(
        &self,
        batches: Vec<crate::cookies::Batch>,
        done: impl FnOnce(std::result::Result<usize, CookieApplyError>) + 'static,
    ) {
        self.context.apply_cookie_batches(batches, done);
    }
    // The per-branch PR calls the checkpoint lifecycle.
    #[cfg_attr(not(feature = "gui-smoke"), allow(dead_code))]
    fn checkpoint_and_release(
        &self,
        done: impl FnOnce(Result<live::Checkpoint>) + 'static,
    ) -> Result<()> {
        let page = self
            .branches
            .live
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("No live page."))?;
        self.releasing.set(true);
        let weak = self.me.clone();
        let finish = move |result| {
            if let Some(pages) = weak.upgrade() {
                pages.releasing.set(false);
            }
            done(result);
        };
        let pending = page.clone();
        page.capture_checkpoint_scroll(move || finish(Self::release_page(pending)));
        Ok(())
    }

    fn release_page(page: Rc<LivePage>) -> Result<live::Checkpoint> {
        let saved = page.checkpoint();
        drop(page);
        saved
    }

    fn discard_view_continuation(&self, navigation: &crate::browser::Navigation) {
        discard_continuation(&mut self.continuation.borrow_mut(), navigation);
    }

    #[cfg(feature = "gui-smoke")]
    pub fn release_selected_page(&self) {
        let weak = self.me.clone();
        let _ = self.checkpoint_and_release(move |saved| {
            if let Some(pages) = weak.upgrade() {
                *pages.checkpoint.borrow_mut() = saved.ok();
            }
        });
    }
    #[cfg(feature = "gui-smoke")]
    pub fn mount_selected_page(self: &Rc<Self>, container: &gtk::Fixed) -> Result<()> {
        let page = LivePage::new(container, Rc::downgrade(self), &self.context)?;
        let old = self.branches.live.borrow_mut().replace(Rc::new(page));
        if let Some(old) = old {
            let pending = old.clone();
            old.capture_checkpoint_scroll(move || {
                let _ = Self::release_page(pending);
            });
        }
        self.checkpoint.borrow_mut().take();
        Ok(())
    }
    // The per-branch PR calls the checkpoint lifecycle.
    #[cfg_attr(not(feature = "gui-smoke"), allow(dead_code))]
    fn revive_at(
        &self,
        container: &gtk::Fixed,
        saved: Option<&live::Checkpoint>,
        visit: VisitId,
    ) -> Result<live::Revival> {
        anyhow::ensure!(!self.releasing.get(), "Page release still pending.");
        anyhow::ensure!(self.branches.live.borrow().is_none(), "Page still live.");
        let (page, cold) = match saved {
            Some(saved) => {
                LivePage::revive(container, self.me.clone(), &self.context, saved, visit)?
            }
            None => (
                LivePage::new(container, self.me.clone(), &self.context)?,
                live::Revival::Cold,
            ),
        };
        page.show();

        let page = Rc::new(page);
        *self.branches.live.borrow_mut() = Some(page.clone());
        #[cfg(feature = "gui-smoke")]
        {
            let observer = self.before_revival.borrow().clone();
            if let Some(observer) = observer {
                observer();
            }
        }
        page.restore_revived(visit)?;
        page.show();
        Ok(cold)
    }

    #[cfg(feature = "gui-smoke")]
    pub fn revive_selected_page(&self, container: &gtk::Fixed, visit: VisitId) -> Result<bool> {
        let saved = self.checkpoint.borrow_mut().take();
        self.revive_at(container, saved.as_ref(), visit)
            .map(|outcome| matches!(outcome, live::Revival::Cold))
    }

    #[cfg(feature = "gui-smoke")]
    pub fn has_pending_continuation(&self) -> bool {
        self.continuation.borrow().is_some()
    }

    #[cfg(feature = "gui-smoke")]
    pub fn before_revival_traversal(&self, observer: impl Fn() + 'static) {
        *self.before_revival.borrow_mut() = Some(Rc::new(observer));
    }

    #[cfg(feature = "gui-smoke")]
    pub fn forget_history_visit(&self, visit: VisitId) {
        self.page().forget_history_visit(visit);
    }

    #[cfg(feature = "gui-smoke")]
    pub fn revival_refusals(&self) -> usize {
        self.page().revival_refusals()
    }

    #[cfg(feature = "gui-smoke")]
    pub fn selected_history_visits(&self) -> Vec<VisitId> {
        self.page().history_visits()
    }

    #[cfg(feature = "gui-smoke")]
    pub fn checkpoint_size(&self) -> Option<usize> {
        self.checkpoint.borrow().as_ref().map(|c| c.bytes.len())
    }

    #[cfg(feature = "gui-smoke")]
    pub fn corrupt_checkpoint(&self, kind: &str) {
        let mut checkpoint = self.checkpoint.borrow_mut();
        let saved = checkpoint.as_mut().expect("checkpoint");
        match kind {
            "length" => {
                saved.entries.pop();
            }
            "position" => saved.current = usize::MAX,
            "url" => saved.entries[0].stored_url = Url::parse("https://invalid.example/").unwrap(),
            _ => panic!("unknown corruption"),
        }
    }
    #[cfg(feature = "gui-smoke")]
    pub fn selected_page(&self) -> SelectedPage<'_> {
        SelectedPage {
            pages: self,
            native: self.page().native.clone(),
        }
    }
}

#[cfg(feature = "gui-smoke")]
pub struct SelectedPage<'a> {
    pages: &'a Pages,
    pub native: webkit2gtk::WebView,
}
#[cfg(feature = "gui-smoke")]
impl SelectedPage<'_> {
    pub fn navigate(&self, url: Url, target: NavigationTarget) -> Result<()> {
        self.pages.go(Go::Address { url, target })
    }
    pub fn restore(&self, visit: VisitId) -> Result<()> {
        self.pages.go(Go::Visit(visit))
    }
    pub fn back(&self) -> Result<()> {
        self.pages.go(Go::Parent)
    }
    pub fn focus(&self) {
        self.pages.focus();
    }
    pub fn open_citation(&self, visit: VisitId) -> Result<()> {
        self.pages.go(Go::Citation(visit))
    }
    pub fn reload(&self) -> Result<()> {
        self.pages.reload()
    }
    pub fn capture_enabled(&self) -> Result<bool> {
        self.pages.read(|b| b.capture_enabled())
    }
    pub fn remember_selection(&self) -> Result<()> {
        self.pages.remember_selection()
    }
}

fn discard_continuation(
    pending: &mut Option<PendingContinuation>,
    navigation: &crate::browser::Navigation,
) {
    if pending
        .as_ref()
        .is_some_and(|c| c.state.ticket().belongs_to(navigation))
    {
        pending.take();
    }
}

#[cfg(test)]
mod continuation_tests {
    use super::*;
    use crate::browser::Navigation;

    #[test]
    fn checkpoint_discards_only_the_owning_views_continuation() {
        let mut source = Navigation::default();
        let mut target = Navigation::default();
        let url = Url::parse("https://example.org/").unwrap();
        let source_ticket = source.native_request(url.clone());
        let target_ticket = target.native_request(url.clone());
        let directory = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&directory.path().join("forest.sqlite3")).unwrap();
        let visit = browser.add_roots(&[url]).unwrap()[0];
        for ticket in [source_ticket, target_ticket] {
            for state in [
                ContinuationState::Waiting(ticket),
                ContinuationState::Committed { ticket, visit },
                ContinuationState::Ready { ticket, visit },
            ] {
                let mut pending = Some(PendingContinuation {
                    state,
                    action: Continuation::BranchPalette,
                });
                discard_continuation(&mut pending, &source);
                assert_eq!(pending.is_none(), ticket == source_ticket);
            }
        }
    }

    #[test]
    fn waiting_continuation_requires_its_own_attempt() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.org/").unwrap();
        let first = navigation.native_request(url.clone());
        let state = ContinuationState::Waiting(first);
        assert!(!state.invalidated_by(Some(first)));
        let replacement = navigation.native_request(url);
        assert!(state.invalidated_by(Some(replacement)));
        navigation.failed(replacement);
        assert!(state.invalidated_by(navigation.pending_ticket()));
    }

    #[test]
    fn cancellation_and_blank_invalidate_waiting_continuation() {
        let mut navigation = Navigation::default();
        let ticket = navigation.native_request(Url::parse("https://example.org/").unwrap());
        let state = ContinuationState::Waiting(ticket);
        navigation.clear();
        assert!(state.invalidated_by(navigation.pending_ticket()));
    }

    #[test]
    fn committed_and_ready_transitions() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.org/").unwrap();
        let ticket = navigation.native_request(url.clone());
        let replacement = navigation.native_request(url.clone());
        let directory = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&directory.path().join("forest.sqlite3")).unwrap();
        let visit = browser.add_roots(&[url]).unwrap()[0];
        let mut state = ContinuationState::Waiting(ticket);
        assert!(state.finished().is_none());
        assert!(!state.committed(replacement, visit, false));
        assert!(state.committed(ticket, visit, false));
        assert!(!state.invalidated_by(None));
        assert!(!state.invalidated_by(Some(ticket)));
        assert!(state.invalidated_by(Some(replacement)));
        assert!(!state.committed(ticket, visit, false));
        assert_eq!(state.finished(), Some(ticket));
        assert!(matches!(state, ContinuationState::Ready { visit: v, .. } if v == visit));
        assert!(!state.invalidated_by(Some(replacement)));
        assert!(!state.invalidated_by(None));
        assert!(state.finished().is_none());
        assert!(!state.committed(replacement, visit, false));
        let mut same_document = ContinuationState::Waiting(ticket);
        assert!(same_document.committed(ticket, visit, true));
        assert!(matches!(same_document, ContinuationState::Ready { .. }));
    }
    #[test]
    fn every_state_handles_failure_and_explicit_discard() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.org/").unwrap();
        let ticket = navigation.native_request(url.clone());
        let other = navigation.native_request(url.clone());
        let directory = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&directory.path().join("forest.sqlite3")).unwrap();
        let visit = browser.add_roots(&[url]).unwrap()[0];
        for state in [
            ContinuationState::Waiting(ticket),
            ContinuationState::Committed { ticket, visit },
            ContinuationState::Ready { ticket, visit },
        ] {
            for _cause in ["cancel", "blank", "settle"] {
                assert!(state.transition(ContinuationEvent::Discard).is_none());
            }
            let ready = matches!(state, ContinuationState::Ready { .. });
            assert_eq!(state.transition(ContinuationEvent::Failed).is_some(), ready);
            assert_eq!(
                state
                    .transition(ContinuationEvent::Pending(Some(other)))
                    .is_some(),
                ready
            );
            assert!(state
                .transition(ContinuationEvent::Pending(Some(ticket)))
                .is_some());
            assert_eq!(
                state.transition(ContinuationEvent::Pending(None)).is_some(),
                !matches!(state, ContinuationState::Waiting(_))
            );
            assert_eq!(
                state
                    .transition(ContinuationEvent::Commit {
                        ticket: other,
                        visit,
                        same_document: false
                    })
                    .is_some(),
                ready
            );
        }
    }
}
