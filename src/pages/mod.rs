mod live;
mod native;
pub(crate) mod stack;

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
pub use live::find::{
    FindAction, FindClose, FindId, FindMatches, FindReadout, FindTarget, FindUpdate,
};
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
    Forward,
    Sibling { next: bool },
    Tree(TreeId),
    Address { url: Url, target: NavigationTarget },
    Citation(VisitId),
    Saved(i64),
}

pub enum PageEvent {
    FormReturn(Option<u64>),
    Selected,
    Settled(String),
    Changed,
    Loading(bool),
    Error(String),
    Remembered(i64),
    Download(DownloadEvent),
    OpenBranchPalette { visit: VisitId },
    Login(LoginEvent),
    Find(FindUpdate),
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
    Go(Box<Go>, Option<Continuation>, bool),
    Settle(SettleTarget),
    Dismiss(VisitId),
    Move(VisitId),
    Reload,
}

#[derive(Clone, Copy)]
enum SettleTarget {
    Tree(TreeId),
    Branch(VisitId),
}

impl SettleTarget {
    fn head(self) -> VisitId {
        match self {
            Self::Tree(tree) => tree.root(),
            Self::Branch(head) => head,
        }
    }
    fn contains(self, forest: &ForestProjection, visit: VisitId) -> bool {
        match self {
            Self::Tree(tree) => forest.tree_of(visit) == Some(tree),
            Self::Branch(head) => forest.path(visit).contains(&head),
        }
    }
}

struct SettleUndo {
    head: VisitId,
    reopen: VisitId,
}

struct Hidden {
    page: Rc<LivePage>,
    last_shown: u64,
}

struct Branches {
    hidden: RefCell<Vec<Hidden>>,
    background: RefCell<Vec<Hidden>>,
    queued: RefCell<Vec<(u64, VisitId)>>,
    clock: Cell<u64>,
    live: RefCell<Option<Rc<LivePage>>>,
    dormant: RefCell<Vec<live::Checkpoint>>,
    candidate: RefCell<Option<Rc<LivePage>>>,
    candidate_live: Cell<Option<u64>>,
    candidate_saved: RefCell<Option<(usize, live::Checkpoint)>>,
}

#[cfg(feature = "gui-smoke")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleasePhase {
    Holding,
    Hidden,
    Checkpointing,
    Released,
}
#[cfg(feature = "gui-smoke")]
#[derive(Clone, Copy, Debug)]
pub struct ReleaseEvent {
    pub view: crate::browser::ViewId,
    pub widget: usize,
    pub phase: ReleasePhase,
}
#[cfg(feature = "gui-smoke")]
type ReleaseObserver = Rc<dyn Fn(ReleaseEvent)>;

struct Hold {
    last_shown: u64,
    outgoing: Rc<LivePage>,
    cancel: gio::Cancellable,
    cap: Option<glib::SourceId>,
}

const BACKGROUND_PAGE_RESERVE: u64 = 256 * 1024 * 1024;

thread_local! {
    static BACKGROUND_WINDOWS: RefCell<Vec<Weak<Pages>>> = const { RefCell::new(Vec::new()) };
    static SCHEDULING_BACKGROUND: Cell<bool> = const { Cell::new(false) };
    static SAMPLE_STARTS: Cell<u64> = const { Cell::new(0) };
    static BACKGROUND_STARTS: Cell<u64> = const { Cell::new(0) };
    static BACKGROUND_TIMEOUT: Cell<std::time::Duration> = const { Cell::new(std::time::Duration::from_secs(20)) };
    static QUEUE_CLOCK: Cell<u64> = const { Cell::new(0) };
    static LATEST_MEMORY_SAMPLE: Cell<Option<u64>> = const { Cell::new(None) };
}

static MEMORY_DROP: std::sync::Mutex<()> = std::sync::Mutex::new(());
static MEMORY_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct MemoryDrop {
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Drop for MemoryDrop {
    fn drop(&mut self) {
        MEMORY_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct Pages {
    me: Weak<Pages>,
    browser: RefCell<Browser>,
    context: Rc<native::PageContext>,
    branches: Branches,
    container: gtk::Fixed,
    viewport: Cell<(i32, i32)>,
    views: RefCell<Vec<glib::WeakRef<webkit2gtk::WebView>>>,
    hold: RefCell<Option<Hold>>,
    chrome: RefCell<Option<glib::WeakRef<webkit2gtk::WebView>>>,
    commands: RefCell<std::collections::VecDeque<Command>>,
    retiring: RefCell<Vec<Rc<LivePage>>>,
    #[cfg(feature = "gui-smoke")]
    checkpoint: RefCell<Option<live::Checkpoint>>,
    #[cfg(feature = "gui-smoke")]
    before_revival: RefCell<Option<Rc<dyn Fn()>>>,
    #[cfg(feature = "gui-smoke")]
    placement_observer: RefCell<Option<PlacementObserver>>,
    #[cfg(feature = "gui-smoke")]
    release_observer: RefCell<Option<ReleaseObserver>>,
    budget: crate::budget::Budget,
    sampling: Cell<bool>,
    sample_started: Cell<Option<std::time::Instant>>,
    sample_scheduled: Cell<bool>,
    memory_epoch: Cell<u64>,
    cooling: Cell<bool>,
    releases: Cell<usize>,
    memory_drop: RefCell<Option<MemoryDrop>>,
    continuation: RefCell<Option<PendingContinuation>>,
    observer: RefCell<Option<Observer>>,
    start: Cell<Option<StartPage>>,
    settle_undo: RefCell<Option<SettleUndo>>,
}

impl Pages {
    pub fn mount(container: &gtk::Fixed, browser: Browser, profile: &Path) -> Result<Rc<Self>> {
        let context = native::PageContext::new(profile)?;
        let budget = crate::budget::Budget::of_machine()?;
        let pages = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            browser: RefCell::new(browser),
            context,
            branches: Branches {
                hidden: RefCell::new(Vec::new()),
                background: RefCell::new(Vec::new()),
                queued: RefCell::new(Vec::new()),
                clock: Cell::new(0),
                live: RefCell::new(None),
                dormant: RefCell::new(Vec::new()),
                candidate: RefCell::new(None),
                candidate_live: Cell::new(None),
                candidate_saved: RefCell::new(None),
            },
            container: container.clone(),
            viewport: Cell::new((container.allocated_width(), container.allocated_height())),
            views: RefCell::new(Vec::new()),
            hold: RefCell::new(None),
            chrome: RefCell::new(None),
            commands: RefCell::new(std::collections::VecDeque::new()),
            retiring: RefCell::new(Vec::new()),
            #[cfg(feature = "gui-smoke")]
            checkpoint: RefCell::new(None),
            #[cfg(feature = "gui-smoke")]
            before_revival: RefCell::new(None),
            #[cfg(feature = "gui-smoke")]
            placement_observer: RefCell::new(None),
            #[cfg(feature = "gui-smoke")]
            release_observer: RefCell::new(None),
            budget,
            sampling: Cell::new(false),
            sample_started: Cell::new(None),
            sample_scheduled: Cell::new(false),
            memory_epoch: Cell::new(0),
            cooling: Cell::new(false),
            releases: Cell::new(0),
            memory_drop: RefCell::new(None),
            continuation: RefCell::new(None),
            observer: RefCell::new(None),
            start: Cell::new(None),
            settle_undo: RefCell::new(None),
        });
        let weak = Rc::downgrade(&pages);
        container.connect_size_allocate(move |container, a| {
            if let Some(pages) = weak.upgrade() {
                if let Some(window) = container
                    .toplevel()
                    .and_then(|w| w.downcast::<gtk::Window>().ok())
                {
                    pages.resize(
                        window.allocated_width() as f64,
                        window.allocated_height() as f64,
                    );
                } else {
                    pages.resize(a.width() as f64, a.height() as f64);
                }
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
        let weak = Rc::downgrade(&pages);
        glib::timeout_add_local(std::time::Duration::from_secs(5), move || {
            let Some(pages) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            pages.sample_memory();
            glib::ControlFlow::Continue
        });
        BACKGROUND_WINDOWS.with(|windows| windows.borrow_mut().push(Rc::downgrade(&pages)));
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
        let settled = matches!(event, PageEvent::Loading(false));
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
        if settled {
            self.sample_memory();
        }
    }

    fn selected_changed(&self) {
        let observer = self.observer.borrow().clone();
        if let Some(observer) = observer {
            observer(PageEvent::Selected);
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
        if !self
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| p.owns(ticket))
        {
            return;
        }
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
        self.receive(PageEvent::Changed);
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
        self.end_hold();
        self.advance_continuation(ContinuationEvent::Discard);
    }

    fn cancel_candidate(&self) {
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            let Some(pages) = weak.upgrade() else { return };
            if pages.retired_selected() && pages.branches.candidate.borrow().is_none() {
                let result = LivePage::new(&pages.container, pages.me.clone(), &pages.context);
                match result {
                    Ok(page) => {
                        if let Err(error) = pages.browser.borrow_mut().show_start() {
                            pages.receive(PageEvent::Error(error.to_string()));
                        }
                        pages.promote_live(Rc::new(page));
                    }
                    Err(error) => pages.receive(PageEvent::Error(error.to_string())),
                }
            }
        });
        self.end_hold();
        let page = self.branches.candidate.borrow_mut().take();
        if let Some(page) = page {
            if page.background_visit().is_some() {
                let stamp = page.last_shown.get();
                self.branches.background.borrow_mut().push(Hidden {
                    page,
                    last_shown: stamp,
                });
            } else if let Some(last_shown) = self.branches.candidate_live.take() {
                page.cancel_foreground();
                page.native.set_child_visible(false);
                let mut hidden = self.branches.hidden.borrow_mut();
                let index = hidden
                    .iter()
                    .position(|h| h.last_shown > last_shown)
                    .unwrap_or(hidden.len());
                hidden.insert(index, Hidden { page, last_shown });
                drop(hidden);
                self.receive(PageEvent::Changed);
            } else {
                page.cancel();
            }
        }
        if let Some((_, saved)) = self.branches.candidate_saved.borrow_mut().take() {
            self.remember_checkpoint(saved);
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
        let retiring = self
            .retiring
            .borrow()
            .iter()
            .any(|page| page.contains(visit));
        let retired = self
            .branches
            .live
            .borrow()
            .clone()
            .filter(|page| !page.native.is_sensitive());
        let target_retired = if let Some(page) = retired.filter(|page| page.contains(visit)) {
            let forest =
                self.read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
            page.current_visit() == Some(visit)
                || forest.is_settled(visit)
                || forest
                    .trees()
                    .iter()
                    .any(|t| Some(t.id) == forest.tree_of(visit) && !t.lifecycle.is_open())
        } else {
            false
        };
        if retiring || target_retired {
            self.cancel_candidate();
            self.commands.borrow_mut().push_back(Command::Go(
                Box::new(Go::Visit(visit)),
                None,
                false,
            ));
            return Ok(());
        }
        let candidate = self.branches.candidate.borrow().clone();
        if candidate
            .as_ref()
            .is_some_and(|p| p.pending_visit() == Some(visit))
        {
            return Ok(());
        }
        self.cancel_candidate();
        self.branches
            .queued
            .borrow_mut()
            .retain(|(_, v)| *v != visit);
        let background = self
            .branches
            .background
            .borrow()
            .iter()
            .position(|h| h.page.background_visit() == Some(visit));
        if let Some(index) = background {
            let hidden = self.branches.background.borrow_mut().remove(index);
            hidden.page.adopt();
            if hidden.page.current_visit().is_some() {
                hidden.page.select()?;
                self.promote_live(hidden.page);
                self.receive(PageEvent::Changed);
            } else {
                *self.branches.candidate.borrow_mut() = Some(hidden.page);
            }
            self.start_background();
            return Ok(());
        }
        let selected = self
            .branches
            .live
            .borrow()
            .clone()
            .filter(|page| page.native.is_sensitive());
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
        let index = self
            .branches
            .hidden
            .borrow()
            .iter()
            .rposition(|h| h.page.contains(visit));
        let dormant_stamp = self
            .branches
            .dormant
            .borrow()
            .iter()
            .filter(|saved| saved.entries.iter().any(|e| e.visit == visit))
            .map(|saved| saved.last_shown)
            .max();
        let index = index.filter(|index| {
            dormant_stamp
                .is_none_or(|stamp| self.branches.hidden.borrow()[*index].last_shown >= stamp)
        });
        if let Some(index) = index {
            let hidden = self.branches.hidden.borrow_mut().remove(index);
            if hidden.page.current_visit() == Some(visit) {
                hidden.page.select()?;
                self.promote_live(hidden.page);
                self.receive(PageEvent::Changed);
                let loading = self
                    .branches
                    .live
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.native.is_loading());
                self.receive(PageEvent::Loading(loading));
                return Ok(());
            }
            self.branches.candidate_live.set(Some(hidden.last_shown));
            *self.branches.candidate.borrow_mut() = Some(hidden.page.clone());
            self.receive(PageEvent::Changed);
            return hidden.page.restore(visit);
        }
        self.candidate_page(Some(visit))?.restore_revived(visit)
    }

    fn select_candidate(&self, ticket: LoadTicket) {
        let candidate = self.branches.candidate.borrow().clone();
        let Some(candidate) = candidate.filter(|p| p.owns(ticket)) else {
            return;
        };
        self.branches.candidate.borrow_mut().take();
        self.branches.candidate_live.take();
        self.branches.candidate_saved.borrow_mut().take();
        self.promote(candidate);
    }

    fn promote_live(&self, incoming: Rc<LivePage>) {
        self.answer_form_return(false);
        self.end_hold();
        let last_shown = self.branches.clock.get();
        self.branches.clock.set(last_shown + 1);
        incoming.last_shown.set(last_shown + 1);
        let old = self.branches.live.borrow_mut().replace(incoming.clone());
        let focus = old.as_ref().is_some_and(|p| p.native.is_focus());
        incoming.show();
        if let Some(old) = old {
            old.close_find();
            old.native.set_child_visible(false);
            if !old.native.is_sensitive() {
                self.release_outgoing(old);
            } else {
                // A later checkpoint of this page must sort where the hidden page did.
                old.last_shown.set(last_shown);
                self.branches.hidden.borrow_mut().push(Hidden {
                    page: old,
                    last_shown,
                });
            }
        }
        if focus {
            incoming.focus();
        }
        self.receive(PageEvent::Login(LoginEvent::NewDocument));
        incoming.refresh_login();
        self.selected_changed();
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(pages) = weak.upgrade() {
                pages.sample_memory();
            }
        });
    }

    fn promote(&self, candidate: Rc<LivePage>) {
        self.answer_form_return(false);
        if self.retired_selected() {
            self.promote_live(candidate);
            return;
        }
        self.end_hold();
        let last_shown = self.branches.clock.get();
        self.branches.clock.set(last_shown + 1);
        candidate.last_shown.set(last_shown + 1);
        let old = self.branches.live.borrow_mut().replace(candidate.clone());
        let focus = old.as_ref().is_some_and(|page| page.native.is_focus());
        if let Some(old) = old {
            old.close_find();
            self.hold_page(old, &candidate, last_shown);
        } else {
            candidate.show();
        }
        if focus {
            candidate.focus();
        }
        self.selected_changed();
        // Native commit notification advances its continuation before emitting login events.
        candidate.refresh_login();
    }

    /// Only the real outgoing view covers the incoming view. All work belongs
    /// to this hold and is cancelled before the outgoing page is checkpointed.
    fn hold_page(&self, old: Rc<LivePage>, incoming: &LivePage, last_shown: u64) {
        incoming.show();
        let incoming = &incoming.native;
        let cancel = gio::Cancellable::new();
        *self.hold.borrow_mut() = Some(Hold {
            last_shown,
            outgoing: old.clone(),
            cancel: cancel.clone(),
            cap: None,
        });
        if let Some(window) = old.native.window() {
            window.set_pass_through(true);
        }
        self.place_view(incoming); // also raises the chrome
        #[cfg(feature = "gui-smoke")]
        self.release_event(&old, ReleasePhase::Holding);
        let weak = self.me.clone();
        let cap = glib::timeout_add_local_once(std::time::Duration::from_millis(1000), move || {
            if let Some(pages) = weak.upgrade() {
                if let Some(hold) = pages.hold.borrow_mut().as_mut() {
                    hold.cap.take();
                }
                pages.end_hold();
            }
        });
        self.hold.borrow_mut().as_mut().unwrap().cap = Some(cap);
        let weak = self.me.clone();
        let target = incoming.downgrade();
        incoming.call_async_javascript_function(
            "while (!document.body) await new Promise(r => requestAnimationFrame(r)); await new Promise(r => requestAnimationFrame(r)); await new Promise(r => requestAnimationFrame(r)); return true;",
            None, Some("canopy-hold"), None, Some(&cancel.clone()),
            move |result| {
                if cancel.is_cancelled() || result.is_err() { return; }
                let Some(view) = target.upgrade() else { return; };
                let next_cancel = cancel.clone();
                view.snapshot(webkit2gtk::SnapshotRegion::Visible, webkit2gtk::SnapshotOptions::NONE, Some(&cancel),
                    move |result| {
                        if next_cancel.is_cancelled() || result.is_err() { return; }
                        drop(result); // rendering signal only; never paint the image
                        let Some(view) = target.upgrade() else { return; };
                        let done_cancel = next_cancel.clone();
                        view.call_async_javascript_function(
                            "await new Promise(r => requestAnimationFrame(r)); return true;",
                            None, Some("canopy-hold"), None, Some(&next_cancel),
                            move |result| {
                                if !done_cancel.is_cancelled() && result.is_ok() {
                                    if let Some(pages) = weak.upgrade() { pages.end_hold(); }
                                }
                            });
                    });
            });
    }

    fn end_hold(&self) {
        let hold = self.hold.borrow_mut().take();
        if let Some(hold) = hold {
            hold.cancel.cancel();
            if let Some(cap) = hold.cap {
                cap.remove();
            }
            hold.outgoing.native.set_child_visible(false);
            #[cfg(feature = "gui-smoke")]
            self.release_event(&hold.outgoing, ReleasePhase::Hidden);
            if !hold.outgoing.native.is_sensitive() {
                self.release_outgoing(hold.outgoing);
                return;
            }
            if hold.outgoing.current_visit().is_some() {
                hold.outgoing.last_shown.set(hold.last_shown);
                self.branches.hidden.borrow_mut().push(Hidden {
                    page: hold.outgoing,
                    last_shown: hold.last_shown,
                });
            }
            self.receive(PageEvent::Changed);
            self.drain_commands();
            self.sample_memory();
        }
    }

    fn busy(&self) -> bool {
        self.releases.get() != 0 || self.hold.borrow().is_some()
    }

    fn drain_commands(&self) {
        let count = self.commands.borrow().len();
        for _ in 0..count {
            if self.busy()
                || !self.retiring.borrow().is_empty()
                || self.retired_selected()
                    && matches!(
                        self.commands.borrow().front(),
                        Some(Command::Go(_, _, true))
                    )
            {
                break;
            }
            let command = self.commands.borrow_mut().pop_front();
            let Some(command) = command else { break };
            let result = match command {
                Command::Go(go, continuation, reparent) => {
                    let mut go = *go;
                    if let Go::Address { target, .. } = &mut go {
                        if reparent && matches!(target, NavigationTarget::ChildOf(_)) {
                            *target = self
                                .read(|b| b.snapshot())
                                .ok()
                                .and_then(|s| s.current)
                                .map_or(NavigationTarget::Root, NavigationTarget::ChildOf);
                        }
                    }
                    let result = self.go(go);
                    if let Some(continuation) = continuation {
                        self.continue_with(continuation);
                    }
                    result
                }
                Command::Settle(target) => self.settle(target),
                Command::Dismiss(head) => self.dismiss(head),
                Command::Move(head) => self.move_to_tree(head),
                Command::Reload => self.reload(),
            };
            if let Err(error) = result {
                self.receive(PageEvent::Error(error.to_string()));
            }
        }
    }

    #[cfg(feature = "gui-smoke")]
    pub fn background_view(&self, visit: VisitId) -> Option<webkit2gtk::WebView> {
        self.branches
            .background
            .borrow()
            .iter()
            .find(|h| h.page.background_visit() == Some(visit))
            .map(|h| h.page.native.clone())
    }

    /// Current documents of retained, previously shown pages; no database reads.
    pub fn hidden_visits(&self) -> std::collections::HashSet<VisitId> {
        self.branches
            .hidden
            .borrow()
            .iter()
            .filter_map(|h| h.page.current_visit())
            .collect()
    }

    pub fn loaded_visits(&self) -> std::collections::HashSet<VisitId> {
        self.branches
            .background
            .borrow()
            .iter()
            .filter_map(|h| h.page.current_visit())
            .collect()
    }

    fn queue_background(&self, visit: VisitId) {
        let open = self
            .read(|b| {
                let forest = ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)?;
                Ok::<_, anyhow::Error>(
                    forest.is_waiting(visit)
                        && forest.tree_of(visit).is_some_and(|id| {
                            forest
                                .trees()
                                .iter()
                                .any(|t| t.id == id && t.lifecycle.is_open())
                        }),
                )
            })
            .unwrap_or(false);
        if open {
            let stamp = QUEUE_CLOCK.with(|clock| {
                let stamp = clock.get() + 1;
                clock.set(stamp);
                stamp
            });
            self.branches.queued.borrow_mut().push((stamp, visit));
            self.start_background();
            self.sample_memory();
        }
    }

    fn start_background(&self) {
        if SCHEDULING_BACKGROUND.with(|scheduling| scheduling.replace(true)) {
            return;
        }
        let windows: Vec<_> = BACKGROUND_WINDOWS.with(|registry| {
            let mut registry = registry.borrow_mut();
            registry.retain(|window| window.strong_count() != 0);
            registry.iter().filter_map(Weak::upgrade).collect()
        });
        loop {
            let running: usize = windows
                .iter()
                .map(|window| {
                    window
                        .branches
                        .background
                        .borrow()
                        .iter()
                        .filter(|h| {
                            h.page.never_seen().is_some() && h.page.current_visit().is_none()
                        })
                        .count()
                })
                .sum();
            if running >= 3 {
                break;
            }
            let window = windows
                .iter()
                .filter(|window| {
                    !window.busy()
                        && !window.cooling.get()
                        && LATEST_MEMORY_SAMPLE
                            .with(|sample| sample.get())
                            .is_some_and(|used| {
                                let reserve = BACKGROUND_STARTS
                                    .with(|n| {
                                        n.get()
                                            .saturating_sub(SAMPLE_STARTS.with(Cell::get))
                                            .saturating_add(1)
                                    })
                                    .saturating_mul(BACKGROUND_PAGE_RESERVE);
                                used.saturating_add(reserve) <= window.budget.bytes()
                            })
                })
                .filter_map(|window| {
                    window
                        .branches
                        .queued
                        .borrow()
                        .last()
                        .map(|(stamp, _)| (*stamp, window))
                })
                .max_by_key(|(stamp, _)| *stamp)
                .map(|(_, window)| window);
            let Some(window) = window else { break };
            let (stamp, visit) = window.branches.queued.borrow_mut().pop().unwrap();
            let Ok(page) = LivePage::empty(&window.container, window.me.clone(), &window.context)
            else {
                continue;
            };
            let page = Rc::new(page);
            page.last_shown.set(stamp);
            window.branches.background.borrow_mut().push(Hidden {
                page: page.clone(),
                last_shown: stamp,
            });
            BACKGROUND_STARTS.with(|n| n.set(n.get() + 1));
            let weak = window.me.clone();
            let view = page.view();
            glib::timeout_add_local_once(BACKGROUND_TIMEOUT.with(Cell::get), move || {
                if let Some(window) = weak.upgrade() {
                    let expired = window.branches.background.borrow().iter().any(|h| {
                        h.page.view() == view
                            && h.page.never_seen().is_some()
                            && h.page.current_visit().is_none()
                    });
                    if expired {
                        window.drop_background(view);
                        window.start_background();
                    }
                }
            });
            if page.background(visit).is_err() {
                window.drop_background(page.view());
            }
        }
        SCHEDULING_BACKGROUND.with(|scheduling| scheduling.set(false));
    }

    fn drop_background(&self, view: crate::browser::ViewId) {
        let index = self
            .branches
            .background
            .borrow()
            .iter()
            .position(|h| h.page.view() == view);
        if let Some(index) = index {
            let hidden = self.branches.background.borrow_mut().remove(index);
            hidden.page.cancel();
            self.receive(PageEvent::Changed);
        }
    }

    fn sample_memory(&self) {
        if (self.branches.hidden.borrow().is_empty()
            && self.branches.background.borrow().is_empty()
            && self.branches.queued.borrow().is_empty())
            || self.busy()
            || self.cooling.get()
        {
            return;
        }
        if let Some(delay) =
            crate::budget::sample_delay(self.sample_started.get(), std::time::Instant::now())
        {
            if !self.sample_scheduled.replace(true) {
                let weak = self.me.clone();
                glib::timeout_add_local_once(delay, move || {
                    if let Some(pages) = weak.upgrade() {
                        pages.sample_scheduled.set(false);
                        pages.sample_memory();
                    }
                });
            }
            return;
        }
        if self.sampling.get() {
            return;
        }
        self.sample_started.set(Some(std::time::Instant::now()));
        self.sampling.set(true);
        let starts = BACKGROUND_STARTS.with(Cell::get);
        let epoch = self.memory_epoch.get();
        let global_epoch = MEMORY_EPOCH.load(std::sync::atomic::Ordering::Relaxed);
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(crate::budget::measure());
        });
        let weak = self.me.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(20), move || {
            let Some(pages) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let sample = match receive.try_recv() {
                Ok(sample) => sample,
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    pages.sampling.set(false);
                    return glib::ControlFlow::Break;
                }
            };
            pages.sampling.set(false);
            if pages.memory_epoch.get() == epoch
                && MEMORY_EPOCH.load(std::sync::atomic::Ordering::Relaxed) == global_epoch
            {
                if let Ok(used) = sample {
                    pages.apply_sample_since(used, starts);
                }
            }
            glib::ControlFlow::Break
        });
    }

    #[cfg(feature = "gui-smoke")]
    fn apply_sample(&self, used: u64) {
        self.apply_sample_since(used, BACKGROUND_STARTS.with(Cell::get));
    }

    fn apply_sample_since(&self, used: u64, starts: u64) {
        LATEST_MEMORY_SAMPLE.with(|sample| sample.set(Some(used)));
        SAMPLE_STARTS.with(|n| n.set(starts));
        if self.busy() || self.cooling.get() {
            return;
        }
        let mut candidates: Vec<_> = self
            .branches
            .hidden
            .borrow()
            .iter()
            .filter(|h| !h.page.awaiting_download())
            .filter(|_| {
                self.branches
                    .candidate
                    .borrow()
                    .as_ref()
                    .is_none_or(|p| p.never_seen().is_none())
            })
            .map(|h| crate::budget::Droppable {
                page: h.page.view(),
                last_shown: h.last_shown,
                never_seen: false,
                audible: h.page.native.is_playing_audio(),
            })
            .collect();
        candidates.extend(self.branches.background.borrow().iter().map(|h| {
            crate::budget::Droppable {
                page: h.page.view(),
                last_shown: h.last_shown,
                never_seen: true,
                audible: false,
            }
        }));
        if used <= self.budget.bytes() {
            self.start_background();
        }
        if let Some(view) = crate::budget::next_to_drop(used, self.budget, &candidates) {
            if self
                .branches
                .background
                .borrow()
                .iter()
                .any(|h| h.page.view() == view)
            {
                let Ok(guard) = MEMORY_DROP.try_lock() else {
                    return;
                };
                *self.memory_drop.borrow_mut() = Some(MemoryDrop { _guard: guard });
                self.drop_background(view);
                self.cooling.set(true);
                self.memory_epoch.set(self.memory_epoch.get() + 1);
                let weak = self.me.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                    if let Some(pages) = weak.upgrade() {
                        pages.memory_drop.borrow_mut().take();
                        pages.cooling.set(false);
                        pages.sample_memory();
                    }
                });
                return;
            }
            let index = self
                .branches
                .hidden
                .borrow()
                .iter()
                .position(|h| h.page.view() == view);
            if let Some(index) = index {
                let Ok(guard) = MEMORY_DROP.try_lock() else {
                    return;
                };
                *self.memory_drop.borrow_mut() = Some(MemoryDrop { _guard: guard });
                let hidden = self.branches.hidden.borrow_mut().remove(index);
                self.release_outgoing(hidden.page);
                self.receive(PageEvent::Changed);
            }
        }
    }

    fn remember_checkpoint(&self, saved: live::Checkpoint) {
        let mut dormant = self.branches.dormant.borrow_mut();
        let index = dormant
            .iter()
            .position(|other| other.last_shown > saved.last_shown)
            .unwrap_or(dormant.len());
        dormant.insert(index, saved);
    }

    fn release_outgoing(&self, old: Rc<LivePage>) {
        self.releases.set(self.releases.get() + 1);
        MEMORY_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.cooling.set(true);
        self.memory_epoch.set(self.memory_epoch.get() + 1);
        #[cfg(feature = "gui-smoke")]
        self.release_event(&old, ReleasePhase::Checkpointing);
        #[cfg(feature = "gui-smoke")]
        let event = ReleaseEvent {
            view: old.view(),
            widget: old.native.as_ptr() as usize,
            phase: ReleasePhase::Released,
        };
        old.native.set_child_visible(false);
        let weak = self.me.clone();
        let pending = old.clone();
        old.capture_checkpoint_scroll(move || {
            let saved = Self::release_page(pending);
            if let Some(pages) = weak.upgrade() {
                if let Ok(saved) = saved {
                    pages.remember_checkpoint(saved);
                }
                #[cfg(feature = "gui-smoke")]
                pages.emit_release(event);
                pages.releases.set(pages.releases.get() - 1);
                pages.drain_commands();
                let weak = pages.me.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                    if let Some(pages) = weak.upgrade() {
                        pages.memory_drop.borrow_mut().take();
                        pages.cooling.set(false);
                        pages.sample_memory();
                    }
                });
            }
        });
    }

    fn receive_view(&self, view: crate::browser::ViewId, empty: bool, event: PageEvent) {
        if self
            .branches
            .background
            .borrow()
            .iter()
            .any(|h| h.page.view() == view)
        {
            if matches!(event, PageEvent::Error(_))
                || empty && matches!(event, PageEvent::Loading(false))
            {
                self.drop_background(view);
            } else if matches!(event, PageEvent::Changed) {
                self.receive(PageEvent::Changed);
            }
            if matches!(
                event,
                PageEvent::Changed | PageEvent::Loading(false) | PageEvent::Error(_)
            ) {
                self.start_background();
                self.sample_memory();
            }
            return;
        }
        let candidate = self.branches.candidate.borrow().clone();
        if candidate.as_ref().is_some_and(|p| p.view() == view) {
            if matches!(event, PageEvent::Error(_))
                || matches!(event, PageEvent::Loading(false))
                    && empty
                    && !candidate.as_ref().unwrap().awaiting_download()
            {
                if candidate.as_ref().is_some_and(|p| p.never_seen().is_some()) {
                    if let Some(page) = self.branches.candidate.borrow_mut().take() {
                        page.cancel();
                    }
                    self.receive(PageEvent::Changed);
                    self.sample_memory();
                    return;
                }
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
            let sensitive = self
                .branches
                .live
                .borrow()
                .as_ref()
                .is_some_and(|p| p.native.is_sensitive());
            if sensitive {
                self.receive(event);
            }
        } else if matches!(event, PageEvent::Loading(false)) {
            self.sample_memory();
        } else if matches!(event, PageEvent::Changed) {
            let observer = self.observer.borrow().clone();
            if let Some(observer) = observer {
                observer(PageEvent::Changed);
            }
        }
    }

    fn tree_target(&self, tree: TreeId) -> Result<VisitId> {
        let forest =
            self.read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
        let mut targets: Vec<_> = self
            .branches
            .hidden
            .borrow()
            .iter()
            .filter_map(|h| h.page.current_visit().map(|visit| (h.last_shown, visit)))
            .collect();
        targets.extend(
            self.branches
                .live
                .borrow()
                .iter()
                .filter_map(|p| p.current_visit().map(|visit| (p.last_shown.get(), visit))),
        );
        targets.extend(
            self.retiring
                .borrow()
                .iter()
                .filter_map(|p| p.current_visit().map(|visit| (p.last_shown.get(), visit))),
        );
        targets.extend(self.branches.dormant.borrow().iter().filter_map(|saved| {
            saved
                .entries
                .get(saved.current)
                .map(|e| (saved.last_shown, e.visit))
        }));
        if let Some((_, visit)) = targets
            .into_iter()
            .filter(|(_, visit)| forest.tree_of(*visit) == Some(tree))
            .max_by_key(|(stamp, _)| *stamp)
        {
            return Ok(visit);
        }
        self.read(|b| b.tree_target(tree))
    }

    fn retired_selected(&self) -> bool {
        self.branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| !p.native.is_sensitive())
    }

    pub fn answer_form_return_for(&self, question: u64, proceed: bool) {
        let page = self.branches.live.borrow().clone();
        if let Some(page) = page {
            page.answer_form_return_for(question, proceed);
        }
    }

    pub fn after_form_return_cancel(&self, action: impl FnOnce() + 'static) {
        let page = self.branches.live.borrow().clone();
        let mut action = Some(action);
        glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
            if page
                .as_ref()
                .is_none_or(|p| p.form_return_cursor_restored())
            {
                action.take().unwrap()();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub fn has_form_return(&self) -> bool {
        self.branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| p.has_form_return())
    }

    pub fn answer_form_return(&self, proceed: bool) {
        let page = self.branches.live.borrow().clone();
        if let Some(page) = page {
            page.answer_form_return(proceed);
        }
    }

    pub fn go(&self, go: Go) -> Result<()> {
        if self.has_form_return() {
            self.answer_form_return(false);
            let weak = self.me.clone();
            self.after_form_return_cancel(move || {
                if let Some(pages) = weak.upgrade() {
                    if let Err(error) = pages.go(go) {
                        pages.receive(PageEvent::Error(error.to_string()));
                    }
                }
            });
            return Ok(());
        }
        if self.retired_selected()
            && matches!(
                go,
                Go::Address {
                    target: NavigationTarget::ChildOf(_),
                    ..
                } | Go::Citation(_)
                    | Go::Saved(_)
                    | Go::Parent
                    | Go::Forward
                    | Go::Sibling { .. }
            )
        {
            self.commands
                .borrow_mut()
                .push_back(Command::Go(Box::new(go), None, true));
            return Ok(());
        }
        let in_place = self.commands.borrow().is_empty()
            && match &go {
                Go::Address {
                    target: NavigationTarget::ChildOf(parent),
                    ..
                } => self
                    .branches
                    .live
                    .borrow()
                    .as_ref()
                    .is_some_and(|page| page.current_visit() == Some(*parent)),
                _ => false,
            };
        if self.busy() && !in_place {
            self.commands
                .borrow_mut()
                .push_back(Command::Go(Box::new(go), None, false));
            self.end_hold();
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
            Go::Forward | Go::Sibling { .. } => {
                let forest = self
                    .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
                let target = if let Some(current) = forest.current() {
                    match go {
                        Go::Forward => self.read(|b| b.forward(current))?,
                        Go::Sibling { next } => {
                            let siblings: Vec<_> = forest
                                .siblings(current)
                                .into_iter()
                                .filter(|id| {
                                    !forest.is_settled(*id)
                                        && forest
                                            .visit(*id)
                                            .is_some_and(|v| v.dismissed_at.is_none())
                                })
                                .collect();
                            siblings
                                .iter()
                                .position(|id| *id == current)
                                .and_then(|i| {
                                    if next {
                                        siblings.get(i + 1)
                                    } else {
                                        i.checked_sub(1).and_then(|i| siblings.get(i))
                                    }
                                })
                                .copied()
                        }
                        _ => unreachable!(),
                    }
                } else {
                    None
                };
                if let Some(visit) = target {
                    self.route(visit)
                } else {
                    Ok(())
                }
            }
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
                let visit = self.tree_target(tree)?;
                self.route(visit)
            }
            Go::Address { url, target } => {
                let crossing_tree = if let NavigationTarget::ChildOf(parent) = target {
                    let forest = self.read(|b| {
                        ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)
                    })?;
                    page.current_visit().and_then(|visit| forest.tree_of(visit))
                        != forest.tree_of(parent)
                } else {
                    false
                };
                if crossing_tree
                    || matches!(target, NavigationTarget::Root) && page.current_visit().is_some()
                {
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
            // The chrome intent is followed by a page-focus message. Open the palette after
            // that input batch, just as the commit continuation does for a cold queued page.
            let weak = self.me.clone();
            glib::idle_add_local_once(move || {
                if let Some(pages) = weak.upgrade() {
                    if pages
                        .read(|browser| browser.snapshot())
                        .is_ok_and(|s| s.current == Some(visit))
                    {
                        pages.receive(PageEvent::OpenBranchPalette { visit });
                    }
                }
            });
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
        if let Some(Command::Go(_, continuation, _)) = self.commands.borrow_mut().back_mut() {
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
        self.settle(SettleTarget::Tree(tree))
    }

    pub fn settle_branch(&self, head: VisitId) -> Result<()> {
        self.settle(SettleTarget::Branch(head))
    }

    pub fn undo_settle(&self) -> Result<()> {
        let undo = self.settle_undo.borrow_mut().take();
        if let Some(undo) = undo {
            self.browser.borrow_mut().undo_settle(undo.head)?;
            self.receive(PageEvent::Changed);
            self.go(Go::Visit(undo.reopen))?;
        }
        Ok(())
    }

    pub fn dismiss(&self, head: VisitId) -> Result<()> {
        if self.busy() {
            self.commands.borrow_mut().push_back(Command::Dismiss(head));
            self.end_hold();
            return Ok(());
        }
        self.browser.borrow_mut().dismiss(head)?;
        let forest =
            self.read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
        self.branches
            .queued
            .borrow_mut()
            .retain(|(_, v)| forest.is_waiting(*v));
        let removed: Vec<_> = self
            .branches
            .background
            .borrow()
            .iter()
            .filter(|h| {
                h.page
                    .background_visit()
                    .is_some_and(|v| !forest.is_waiting(v))
            })
            .map(|h| h.page.view())
            .collect();
        for view in removed {
            self.drop_background(view);
        }
        self.receive(PageEvent::Changed);
        Ok(())
    }

    pub fn move_to_tree(&self, head: VisitId) -> Result<()> {
        if self.busy() {
            self.commands.borrow_mut().push_back(Command::Move(head));
            self.end_hold();
            return Ok(());
        }
        self.browser.borrow_mut().move_to_tree(head)?;
        self.receive(PageEvent::Changed);
        Ok(())
    }

    fn settle(&self, target: SettleTarget) -> Result<()> {
        self.answer_form_return(false);
        anyhow::ensure!(self.branches.live.borrow().is_some(), "No selected page.");
        let forest =
            self.read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
        if let SettleTarget::Branch(head) = target {
            anyhow::ensure!(
                forest.visit(head).and_then(|v| v.parent).is_some(),
                "A root is settled as a tree."
            );
            if forest.is_settled(head) {
                return Ok(());
            }
        }
        let candidate = self.branches.candidate.borrow().clone();
        if candidate
            .as_ref()
            .and_then(|p| p.background_visit())
            .is_some_and(|v| target.contains(&forest, v))
        {
            if let Some(page) = self.branches.candidate.borrow_mut().take() {
                page.cancel();
            }
        }
        self.branches
            .queued
            .borrow_mut()
            .retain(|(_, v)| !target.contains(&forest, *v));
        let removed: Vec<_> = self
            .branches
            .background
            .borrow()
            .iter()
            .filter(|h| {
                h.page
                    .background_visit()
                    .is_some_and(|v| target.contains(&forest, v))
            })
            .map(|h| h.page.view())
            .collect();
        for view in removed {
            self.drop_background(view);
        }
        let selected = self.branches.live.borrow().clone();
        if let Some(page) = selected.filter(|page| {
            page.current_visit()
                .is_some_and(|visit| target.contains(&forest, visit))
        }) {
            page.retire();
            self.receive(PageEvent::Login(LoginEvent::NewDocument));
        }
        if self.busy() {
            self.commands
                .borrow_mut()
                .push_back(Command::Settle(target));
            self.end_hold();
            return Ok(());
        }
        self.cancel_continuation();
        let candidate_target = self
            .branches
            .candidate
            .borrow()
            .as_ref()
            .and_then(|p| p.pending_anchor());
        if candidate_target.is_some_and(|visit| target.contains(&forest, visit)) {
            self.cancel_candidate();
        }
        let belongs = |page: &LivePage| {
            page.current_visit()
                .is_some_and(|visit| target.contains(&forest, visit))
        };
        let mut removed = Vec::new();
        self.branches.hidden.borrow_mut().retain(|hidden| {
            if belongs(&hidden.page) {
                removed.push(hidden.page.clone());
                false
            } else {
                true
            }
        });
        if !removed.is_empty() {
            self.memory_epoch.set(self.memory_epoch.get() + 1);
            self.cooling.set(true);
            let weak = self.me.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                if let Some(pages) = weak.upgrade() {
                    pages.cooling.set(false);
                    pages.sample_memory();
                }
            });
        }
        self.retiring.borrow_mut().extend(removed.iter().cloned());
        for page in removed {
            let weak = self.me.clone();
            let view = page.view();
            page.native.set_sensitive(false);
            let pending = page.clone();
            page.capture_checkpoint_scroll(move || {
                let Some(pages) = weak.upgrade() else { return };
                pages
                    .retiring
                    .borrow_mut()
                    .retain(|page| page.view() != view);
                if let Ok(saved) = Self::release_page(pending) {
                    pages.remember_checkpoint(saved);
                }
                pages.drain_commands();
            });
        }
        let head = target.head();
        let name = match target {
            SettleTarget::Tree(tree) => forest
                .trees()
                .iter()
                .find(|t| t.id == tree)
                .map(|t| t.title.clone())
                .unwrap_or_default(),
            SettleTarget::Branch(_) => forest
                .visit(head)
                .map(|v| v.label().to_owned())
                .unwrap_or_default(),
        };
        let waiting = self
            .read(|b| b.snapshot())?
            .visits
            .iter()
            .filter(|v| {
                v.opened_at.is_none() && v.dismissed_at.is_none() && target.contains(&forest, v.id)
            })
            .count();
        let landing = match target {
            SettleTarget::Tree(tree) => self.browser.borrow_mut().settle_tree(tree)?,
            SettleTarget::Branch(head) => self.browser.borrow_mut().settle_branch(head)?,
        };
        *self.settle_undo.borrow_mut() = Some(SettleUndo {
            head,
            reopen: forest
                .current()
                .filter(|id| target.contains(&forest, *id))
                .unwrap_or_else(|| match target {
                    SettleTarget::Tree(tree) => forest
                        .trees()
                        .iter()
                        .find(|t| t.id == tree)
                        .map_or(head, |t| t.last_visit),
                    SettleTarget::Branch(_) => head,
                }),
        });
        self.receive(PageEvent::Settled(format!(
            "Settled ‘{name}’ · {waiting} waiting kept"
        )));
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

                self.promote(page);
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
            } else if let Some(current) = forest.current().filter(|id| forest.is_settled(*id)) {
                if let Some(ancestor) = forest
                    .path(current)
                    .into_iter()
                    .rev()
                    .find(|id| !forest.is_settled(*id))
                {
                    start = StartPage::Show(ancestor);
                }
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
        self.answer_form_return(false);
        if self.retired_selected() {
            return Ok(());
        }
        if self.busy() && self.branches.live.borrow().is_some() {
            self.commands.borrow_mut().push_back(Command::Reload);
            self.end_hold();
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
    pub fn toggle_current_bookmark(&self) -> Result<crate::browser::BookmarkToggle> {
        self.browser.borrow_mut().toggle_current_bookmark()
    }

    pub fn remove_bookmark(&self, id: crate::browser::BookmarkId) -> Result<()> {
        self.browser.borrow_mut().remove_bookmark(id)
    }

    pub fn bookmark(&self, url: &Url, title: &str) -> Result<()> {
        self.browser.borrow_mut().bookmark(url, title)
    }
    pub fn decline(&self, key: &crate::suggest::SuggestionKey) -> Result<()> {
        self.browser.borrow_mut().decline(key)
    }
    pub fn set_pinned(&self, tree: TreeId, pinned: bool) -> Result<()> {
        self.browser.borrow_mut().set_pinned(tree, pinned)
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
        if self.container.child_property::<i32>(view, "x") != rect.x()
            || self.container.child_property::<i32>(view, "y") != rect.y()
        {
            self.container.move_(view, rect.x(), rect.y());
        }
        if view.allocation() != rect {
            view.size_allocate(&rect);
        }
        let chrome = self.chrome.borrow().as_ref().and_then(|w| w.upgrade());
        let held = self
            .hold
            .borrow()
            .as_ref()
            .map(|h| h.outgoing.native.clone());
        stack::Stack::order(
            &self.container,
            chrome.as_ref().map(|w| w.upcast_ref()),
            held.as_ref().map(|w| w.upcast_ref()),
        );
        #[cfg(feature = "gui-smoke")]
        if view.is_mapped() {
            let observer = self.placement_observer.borrow().clone();
            if let Some(observer) = observer {
                observer(view);
            }
        }
    }

    #[cfg(feature = "gui-smoke")]
    pub fn live_visits(&self) -> Vec<VisitId> {
        let mut visits: Vec<_> = self
            .branches
            .hidden
            .borrow()
            .iter()
            .filter_map(|h| h.page.current_visit())
            .collect();
        visits.extend(
            self.branches
                .live
                .borrow()
                .iter()
                .filter_map(|p| p.current_visit()),
        );
        visits.extend(
            self.hold
                .borrow()
                .iter()
                .filter_map(|h| h.outgoing.current_visit()),
        );
        visits
    }

    #[cfg(feature = "gui-smoke")]
    pub fn inject_recent_memory_sample(&self, used: u64) {
        self.memory_epoch.set(self.memory_epoch.get() + 1);
        self.sample_started.set(Some(std::time::Instant::now()));
        self.apply_sample(used);
    }

    #[cfg(feature = "gui-smoke")]
    pub fn set_background_timeout(&self, timeout: std::time::Duration) {
        BACKGROUND_TIMEOUT.with(|limit| limit.set(timeout));
    }

    #[cfg(feature = "gui-smoke")]
    pub fn inject_memory_sample(&self, sample: Option<u64>) {
        if let Some(used) = sample {
            self.apply_sample(used);
        }
    }

    #[cfg(feature = "gui-smoke")]
    pub fn observe_release(&self, observer: impl Fn(ReleaseEvent) + 'static) {
        *self.release_observer.borrow_mut() = Some(Rc::new(observer));
    }

    #[cfg(feature = "gui-smoke")]
    fn release_event(&self, page: &LivePage, phase: ReleasePhase) {
        self.emit_release(ReleaseEvent {
            view: page.view(),
            widget: page.native.as_ptr() as usize,
            phase,
        });
    }

    #[cfg(feature = "gui-smoke")]
    fn emit_release(&self, event: ReleaseEvent) {
        let observer = self.release_observer.borrow().clone();
        if let Some(observer) = observer {
            observer(event);
        }
    }

    #[cfg(feature = "gui-smoke")]
    pub fn observe_page_placement(&self, observer: impl Fn(&webkit2gtk::WebView) + 'static) {
        *self.placement_observer.borrow_mut() = Some(Rc::new(observer));
    }

    fn register_view(&self, view: &webkit2gtk::WebView) {
        self.views.borrow_mut().push(view.downgrade());
        let weak = self.me.clone();
        view.connect_unmap(move |view| {
            if let Some(pages) = weak.upgrade() {
                let selected = pages
                    .branches
                    .live
                    .borrow()
                    .as_ref()
                    .is_some_and(|page| page.native == *view);
                if selected {
                    pages.end_hold();
                }
            }
        });
        let weak = self.me.clone();
        view.connect_event(move |view, event| {
            let input = matches!(
                event.event_type(),
                gdk::EventType::KeyPress
                    | gdk::EventType::KeyRelease
                    | gdk::EventType::ButtonPress
                    | gdk::EventType::ButtonRelease
                    | gdk::EventType::DoubleButtonPress
                    | gdk::EventType::TripleButtonPress
                    | gdk::EventType::MotionNotify
                    | gdk::EventType::Scroll
                    | gdk::EventType::TouchBegin
                    | gdk::EventType::TouchUpdate
                    | gdk::EventType::TouchEnd
            );
            let selected = weak.upgrade().is_some_and(|pages| {
                pages
                    .branches
                    .live
                    .borrow()
                    .as_ref()
                    .is_some_and(|page| page.native == *view)
            });
            if input && !selected {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });

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
        self.end_hold();
        let page = self
            .branches
            .live
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("No live page."))?;
        self.releases.set(self.releases.get() + 1);
        let weak = self.me.clone();
        let finish = move |result| {
            if let Some(pages) = weak.upgrade() {
                pages.releases.set(pages.releases.get() - 1);
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
        anyhow::ensure!(!self.busy(), "Page release still pending.");
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
