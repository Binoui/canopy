#[path = "loading.rs"]
pub(super) mod loading;

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use gtk::prelude::*;
use serde_json::Value;
use url::Url;
#[path = "find.rs"]
pub(super) mod find;
#[path = "login_fields.rs"]
mod login_fields;
#[path = "mouse.rs"]
mod mouse;
#[path = "page_scale.rs"]
pub(super) mod page_scale;

use crate::{
    address::{same_resource, web_url},
    browser::{
        CaptureKind, CapturePayload, Document, LoadTicket, Navigation, NavigationTarget,
        SourceContext, Visibility, VisitId,
    },
    pages::engine::{
        EngineView, EntryId, Frames, History, HistoryItem, LoadEvent, LoadFailure, LoadFailureKind,
        MessageHandler, NavigationType, Observer, PolicyDecision, PolicyDecisionType, Request,
        Responder, ScriptDialog, ScriptDialogType, ScriptTime, UserScript, ViewConfig, ViewEvent,
        World,
    },
};

const WORLD: World = World::ReadingMemory;
/// How long after a click or Enter a script route still counts as the person's step. Sites
/// like YouTube fetch the next page's data before they change the address.
const ROUTE_AFTER_PRESS: std::time::Duration = std::time::Duration::from_secs(4);
const ROUTE_SETTLED: Duration = Duration::from_secs(3);
const HELPER: &str = include_str!("../capture.js");
const CONTEXT_MENU_OVERRIDE: &str = "
window.addEventListener('contextmenu', event => {
    if (event.shiftKey) event.stopImmediatePropagation();
}, true);
";
/// Script message the helper posts, debounced, when the page scrolls.
const SCROLLED: &str = "canopyScrolled";
use super::{LoginEvent, PageEvent, Pages};

#[derive(Clone)]
pub(super) struct CheckpointEntry {
    pub(super) visit: VisitId,
    pub(super) document: u64,
    pub(super) post: bool,
    pub(super) uri: Url,
    pub(super) stored_url: Url,
}

// The per-branch PR calls the checkpoint lifecycle.
#[cfg_attr(not(feature = "gui-smoke"), allow(dead_code))]
pub(super) struct Checkpoint {
    pub(super) last_shown: u64,
    pub(super) bytes: glib::Bytes,
    pub(super) entries: Vec<CheckpointEntry>,
    pub(super) current: usize,
}

pub(super) enum Revival {
    History,
    Cold,
}

/// The page-size handler for load events, which joins the observer once the view is placed.
type ScaleLoadHandler = Rc<dyn Fn(LoadEvent)>;

enum InitialPage {
    Home,
    Session,
}

struct HistoryEntry {
    item: EntryId,
    visit: VisitId,
    document: u64,
    post: bool,
    needs_get: bool,
}

enum Issue {
    Idle,
    Issued {
        ticket: LoadTicket,
        decided: bool,
    },
    Loading(LoadTicket),
    Cancelling(LoadTicket),
    Retiring {
        ticket: LoadTicket,
        next: (LoadTicket, Url),
    },
}

impl Issue {
    fn finish(&mut self, navigation: &mut Navigation) -> Option<Url> {
        match std::mem::replace(self, Self::Idle) {
            Self::Retiring {
                next: (ticket, url),
                ..
            } => (navigation.pending_ticket() == Some(ticket)).then_some(url),
            Self::Loading(ticket) => {
                navigation.failed(ticket);
                None
            }
            Self::Issued { ticket, decided } => {
                *self = Self::Issued { ticket, decided };
                None
            }
            Self::Idle | Self::Cancelling(_) => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Live,
    CapturingScroll,
    Released,
}

struct NativeState {
    native: Rc<dyn EngineView>,
    lifecycle: Cell<Lifecycle>,
    /// The page was taken out of use (closed, deleted or replaced) and must not act on
    /// input or policy any more. Its widget is also insensitive.
    retired: Cell<bool>,
    pages: Weak<Pages>,
    navigation: RefCell<Navigation>,
    never_seen: Cell<Option<VisitId>>,
    adopted: Cell<bool>,
    download_origin: Option<VisitId>,
    loading: RefCell<loading::Activity>,
    issue: RefCell<Issue>,
    form_return: RefCell<Option<(u64, PolicyDecision)>>,
    form_return_cancelled: Cell<bool>,
    load_generation: Cell<u64>,
    foreground_generation: Cell<u64>,
    same_document_request: RefCell<Option<Url>>,
    /// When the person last clicked or pressed Enter in this page. A script route that
    /// follows soon after, as on YouTube, is a step of its own in the tree.
    pressed: Cell<Option<std::time::Instant>>,
    /// The URL last handed to WebKit. A fresh view reports no URI when that load starts.
    issued: RefCell<Option<Url>>,
    /// WebKit history entries and the visit each one showed, oldest first. Returning to a
    /// visit that still has an entry goes through WebKit's history, which keeps the page's
    /// scroll and, from its page cache, its live state.
    history: RefCell<Vec<HistoryEntry>>,
    history_document: Cell<u64>,
    back_forward_commit: Cell<bool>,
    next_history_document: Cell<u64>,
    history_cursor: RefCell<Option<EntryId>>,
    /// The history entry this engine asked WebKit for, which the policy check lets through.
    traversal: RefCell<Option<(LoadTicket, EntryId)>>,
    /// Where to scroll once a visit reloaded from the network finishes loading.
    pending_scroll: Cell<Option<(VisitId, i64)>>,
    /// The address of the main frame's last response that has not become a page, and the
    /// page its file belongs to if it becomes a download instead. WebKit may report that
    /// load as failed before it reports the download, and the navigation is forgotten then.
    response_page: RefCell<Option<(LoadTicket, Url, Option<VisitId>)>>,
    download_handover: Cell<Option<LoadTicket>>,
    awaiting_response: Cell<bool>,
    reload_candidate: RefCell<Option<Url>>,
    main_frame_post: Cell<Option<(LoadTicket, bool)>>,
    find: RefCell<find::Driver>,
    #[cfg(feature = "gui-smoke")]
    revival_refusals: Cell<usize>,
}

struct NativeCallback {
    state: Rc<NativeState>,
    _owner: Rc<Pages>,
}

impl std::ops::Deref for NativeCallback {
    type Target = Rc<NativeState>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl Pages {
    fn enter_native(weak: &Weak<NativeState>) -> Option<NativeCallback> {
        let state = weak.upgrade()?;
        if state.lifecycle.get() != Lifecycle::Live {
            return None;
        }
        let owner = state.pages.upgrade()?;
        // A reentrant observer can release the window; its owner must outlive this callback.
        Some(NativeCallback {
            state,
            _owner: owner,
        })
    }
}

pub(super) struct LivePage {
    pub(super) last_shown: Cell<u64>,
    pub(super) native: Rc<dyn EngineView>,
    state: Rc<NativeState>,
    signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
}

impl Drop for LivePage {
    fn drop(&mut self) {
        self.detach();
    }
}

impl LivePage {
    pub(super) fn never_seen(&self) -> Option<VisitId> {
        self.state
            .background_visit()
            .filter(|_| !self.state.adopted.get())
    }
    pub(super) fn background_visit(&self) -> Option<VisitId> {
        self.state.background_visit()
    }
    pub(super) fn adopt(&self) {
        self.state.adopted.set(true);
        self.native.set_muted(false);
    }
    /// Undoes [`Self::adopt`] when the page goes back to the background list unseen: muted again,
    /// counted in a load slot and subject to the background time limit.
    pub(super) fn unadopt(&self) {
        self.state.adopted.set(false);
        self.native.set_muted(true);
    }
    pub(super) fn background(&self, visit: VisitId) -> Result<()> {
        self.state.never_seen.set(Some(visit));
        self.native.set_muted(true);
        self.native.widget().set_child_visible(false);
        self.restore(visit)
    }
    pub(super) fn pending_ticket(&self) -> Option<LoadTicket> {
        self.state.navigation.borrow().pending_ticket()
    }

    pub(super) fn empty(
        container: &gtk::Fixed,
        pages: Weak<Pages>,
        context: &Rc<super::context::PageContext>,
    ) -> Result<Self> {
        Self::build(container, pages, context, InitialPage::Session)
    }
    pub(super) fn view(&self) -> crate::browser::ViewId {
        self.state.navigation.borrow().view_id()
    }
    pub(super) fn owns(&self, ticket: LoadTicket) -> bool {
        ticket.belongs_to(&self.state.navigation.borrow())
    }
    pub(super) fn discard_deleted(&self) {
        self.native.set_muted(true);
        self.mark_retired();
        self.cancel();
        self.detach();
    }
    pub(super) fn document(&self) -> Option<Document> {
        self.state.ticket()
    }

    pub(super) fn capture_document(&self) -> Option<(Document, u64)> {
        Some((self.document()?, self.state.navigation.borrow().sequence()))
    }

    pub(super) fn current_visit(&self) -> Option<VisitId> {
        self.state
            .navigation
            .borrow()
            .current
            .as_ref()
            .map(|d| d.visit)
    }
    pub(super) fn pending_anchor(&self) -> Option<VisitId> {
        self.state.navigation.borrow().pending_anchor()
    }
    pub(super) fn pending_target(&self) -> Option<crate::browser::PendingTarget> {
        self.state.navigation.borrow().pending_target()
    }
    pub(super) fn pending_visit(&self) -> Option<VisitId> {
        self.state.navigation.borrow().pending_restore()
    }
    pub(super) fn contains(&self, visit: VisitId) -> bool {
        let entries = self.native.history().live();
        self.state
            .history
            .borrow()
            .iter()
            .any(|e| e.visit == visit && entries.contains(&e.item))
    }
    pub(super) fn answer_form_return_for(&self, question: u64, proceed: bool) {
        let matches = self
            .state
            .form_return
            .borrow()
            .as_ref()
            .is_some_and(|(id, _)| *id == question);
        if matches {
            self.answer_form_return(proceed);
        }
    }

    pub(super) fn form_return_cursor_restored(&self) -> bool {
        let current = self.native.history().current_id();
        let restored = self.state.lifecycle.get() == Lifecycle::Released
            || self.state.history.borrow().iter().any(|e| {
                Some(e.item) == current && e.document == self.state.history_document.get()
            });
        if restored {
            self.state.form_return_cancelled.set(false);
        }
        restored
    }

    pub(super) fn has_form_return(&self) -> bool {
        self.state.form_return.borrow().is_some()
            || (self.state.form_return_cancelled.get() && !self.form_return_cursor_restored())
    }

    pub(super) fn answer_form_return(&self, proceed: bool) {
        self.state.answer_form_return(proceed);
    }

    pub(super) fn cancel_foreground(&self) {
        self.answer_form_return(false);
        let state = &self.state;
        state.cancel_loading();
        state.end_response();
        state
            .owner()
            .discard_view_continuation(&state.navigation.borrow());
        state
            .foreground_generation
            .set(state.foreground_generation.get().wrapping_add(1));
        state
            .load_generation
            .set(state.load_generation.get().wrapping_add(1));
        if let Some(ticket) = self.pending_ticket() {
            state.navigation.borrow_mut().failed(ticket);
        }
        // A request stopped before policy acceptance has no Finished signal.
        // Accepted requests can still deliver Started after a busy beforeunload;
        // drain those too so they cannot cancel the candidate or commit later.
        let draining = match *state.issue.borrow() {
            Issue::Loading(ticket)
            | Issue::Retiring { ticket, .. }
            | Issue::Cancelling(ticket)
            | Issue::Issued {
                ticket,
                decided: true,
            } if self.native.is_loading() => Some(ticket),
            _ => None,
        };
        *state.issue.borrow_mut() = draining.map(Issue::Cancelling).unwrap_or(Issue::Idle);
        state.issued.borrow_mut().take();
        state.traversal.borrow_mut().take();
        state.same_document_request.borrow_mut().take();
        state.reload_candidate.borrow_mut().take();
        state.pending_scroll.set(None);
        self.native.stop_loading();
    }

    pub(super) fn awaiting_download(&self) -> bool {
        self.state.awaiting_response.get() && self.native.is_loading()
            || self.state.download_handover.get().is_some()
    }

    pub(super) fn requires_get(&self, visit: VisitId) -> bool {
        self.state.history_document.get() != 0
            && self
                .state
                .history
                .borrow()
                .iter()
                .any(|e| e.visit == visit && e.needs_get)
    }

    pub(super) fn cancel(&self) {
        self.state
            .owner()
            .discard_view_continuation(&self.state.navigation.borrow());
        if let Some(ticket) = self.pending_ticket() {
            self.state.navigation.borrow_mut().failed(ticket);
        }
        self.detach();
    }
    /// Whether the page was taken out of use; see `NativeState::retired`.
    pub(super) fn retired(&self) -> bool {
        self.state.retired.get()
    }
    /// Takes the page out of use: no more input or policy. The widget goes insensitive too.
    pub(super) fn mark_retired(&self) {
        self.native.widget().set_sensitive(false);
        self.state.retired.set(true);
    }
    pub(super) fn retire(&self) {
        self.mark_retired();
        self.native.set_muted(true);
        self.cancel_foreground();
        self.state.lifecycle.set(Lifecycle::CapturingScroll);
    }
    pub(super) fn select(&self) -> Result<()> {
        let navigation = self.state.navigation.borrow();
        if let Some(document) = navigation.current.as_ref() {
            self.state
                .owner()
                .browser
                .borrow_mut()
                .select_document(&navigation, document)?;
        }
        Ok(())
    }
    pub(super) fn refresh_login(&self) {
        self.state.look_for_login_fields();
    }
    pub(super) fn show(&self) {
        if self.state.never_seen.take().is_some() {
            self.native.set_muted(false);
        }
        let widget = self.native.widget();
        if let Some(window) = widget.window() {
            window.set_pass_through(false);
        }
        widget.set_sensitive(true);
        self.state.retired.set(false);
        widget.set_child_visible(true);
        widget.show();
        self.state.owner().place_view(widget);
        self.state.restore_scroll();
    }
    pub(super) fn native_back(&self) -> Option<crate::pages::engine::HistoryItem> {
        let list = self.native.history();
        let mut items = list.back;
        // WebKit returns nearest first; frame and pushState entries share a document.
        items.sort_by_key(|item| {
            (1..=list.length)
                .find(|n| self.native.history_nth(-(*n as i32)) == Some(item.id))
                .unwrap_or(u32::MAX)
        });
        let allowed = |item: &&crate::pages::engine::HistoryItem| {
            self.state
                .history
                .borrow()
                .iter()
                .any(|e| e.item == item.id && self.state.native_history_allowed(e.visit))
        };
        items
            .iter()
            .filter(allowed)
            .find(|item| {
                self.state
                    .history
                    .borrow()
                    .iter()
                    .any(|e| e.item == item.id && Some(e.visit) != self.current_visit())
            })
            .or_else(|| {
                if self.back_visit().ok().flatten().is_some() {
                    return None;
                }
                items
                    .iter()
                    .filter(allowed)
                    .find(|item| item.uri != self.native.uri())
            })
            .cloned()
    }
    /// The visit Back opens from here: the page behind `native_back`'s entry, else the
    /// forest parent.
    pub(super) fn back_target(&self) -> Option<VisitId> {
        match self.native_back() {
            Some(item) => self
                .state
                .history
                .borrow()
                .iter()
                .find(|e| e.item == item.id)
                .map(|e| e.visit),
            None => self.back_visit().ok().flatten(),
        }
    }
    pub(super) fn back_visit(&self) -> Result<Option<VisitId>> {
        let Some(visit) = self.current_visit() else {
            return Ok(None);
        };
        let parent = self.state.owner().browser.borrow().visit(visit)?.parent;
        Ok(parent.filter(|v| self.state.native_history_allowed(*v)))
    }

    pub fn new(
        container: &gtk::Fixed,
        pages: Weak<Pages>,
        context: &Rc<super::context::PageContext>,
    ) -> Result<Self> {
        Self::build(container, pages, context, InitialPage::Home)
    }

    fn build(
        container: &gtk::Fixed,
        pages: Weak<Pages>,
        context: &Rc<crate::pages::context::PageContext>,
        initial: InitialPage,
    ) -> Result<Self> {
        let mut config = ViewConfig {
            scripts: vec![
                UserScript {
                    source: HELPER,
                    world: WORLD,
                    frames: Frames::Top,
                    time: ScriptTime::DocumentStart,
                },
                UserScript {
                    source: CONTEXT_MENU_OVERRIDE,
                    world: WORLD,
                    frames: Frames::All,
                    time: ScriptTime::DocumentStart,
                },
            ],
            message_handlers: vec![MessageHandler {
                name: SCROLLED,
                world: WORLD,
            }],
            media: true,
            autoplay: true,
            page_cache: true,
            developer_extras: cfg!(debug_assertions),
        };
        login_fields::configure(&mut config);
        let view = context.engine().create_view(config)?;
        let widget = view.widget();
        let mut signals = Vec::new();
        signals.extend(
            mouse::setup(widget, Rc::downgrade(&view), pages.clone())
                .into_iter()
                .map(|id| (widget.clone().upcast(), id)),
        );
        container.put(widget, 0, 0);
        widget.set_child_visible(matches!(initial, InitialPage::Home));
        widget.show();
        if container.is_realized() {
            widget.realize();
        }
        if matches!(initial, InitialPage::Home) {
            view.load_html(HOME);
        }
        if matches!(initial, InitialPage::Home) {
            widget.show_all();
            widget.grab_focus();
        }
        if let Some(signal) = context.blocking.follow(&*view) {
            signals.push((widget.clone().upcast(), signal));
        }
        let state = Rc::new(NativeState {
            native: view.clone(),
            lifecycle: Cell::new(Lifecycle::Live),
            retired: Cell::new(false),
            pages: pages.clone(),
            never_seen: Cell::new(None),
            adopted: Cell::new(false),
            download_origin: pages.upgrade().and_then(|owner| {
                owner
                    .branches
                    .live
                    .borrow()
                    .as_ref()
                    .and_then(|page| page.current_visit())
            }),
            navigation: RefCell::new(Navigation::default()),
            loading: RefCell::new(loading::Activity::default()),
            issue: RefCell::new(Issue::Idle),
            form_return: RefCell::new(None),
            form_return_cancelled: Cell::new(false),
            load_generation: Cell::new(0),
            foreground_generation: Cell::new(0),
            same_document_request: RefCell::new(None),
            pressed: Cell::new(None),
            issued: RefCell::new(None),
            history: RefCell::new(Vec::new()),
            history_document: Cell::new(0),
            back_forward_commit: Cell::new(false),
            next_history_document: Cell::new(0),
            history_cursor: RefCell::new(None),
            traversal: RefCell::new(None),
            pending_scroll: Cell::new(None),
            response_page: RefCell::new(None),
            download_handover: Cell::new(None),
            awaiting_response: Cell::new(false),
            reload_candidate: RefCell::new(None),
            main_frame_post: Cell::new(None),
            find: RefCell::default(),
            #[cfg(feature = "gui-smoke")]
            revival_refusals: Cell::new(0),
        });
        // The view's events, each handled as its WebKit signal was before the engine layer.
        // They all reach the observer set at the end of this function.
        //
        // WebKit defaults may present modal UI even for an unmapped view.
        let on_script_dialog = {
            let weak = state.pages.clone();
            move |view: &gtk::Widget, dialog: &dyn ScriptDialog| {
                if !is_selected(&weak, view) {
                    if matches!(
                        dialog.dialog_type(),
                        ScriptDialogType::Confirm | ScriptDialogType::BeforeUnloadConfirm
                    ) {
                        dialog.confirm_set_confirmed(false);
                    }
                    true
                } else {
                    false
                }
            }
        };
        let on_permission_request = {
            let weak = state.pages.clone();
            move |view: &gtk::Widget, request: &dyn Responder| {
                if !is_selected(&weak, view) {
                    request.refuse();
                    true
                } else {
                    false
                }
            }
        };
        let on_file_chooser = {
            let weak = state.pages.clone();
            move |view: &gtk::Widget, request: &dyn Responder| {
                if !is_selected(&weak, view) {
                    request.refuse();
                    true
                } else {
                    false
                }
            }
        };
        let on_enter_fullscreen = {
            let weak = state.pages.clone();
            move |view: &gtk::Widget| {
                if !is_selected(&weak, view) {
                    return true;
                }
                // WebKit only fullscreens the window; the page must also leave the chrome's
                // band and the chrome must step aside, or the video stays under the strip.
                if let Some(pages) = weak.upgrade() {
                    pages.fullscreen_entered(view);
                }
                false
            }
        };
        let on_leave_fullscreen = {
            let weak = state.pages.clone();
            move |view: &gtk::Widget| {
                if let Some(pages) = weak.upgrade() {
                    pages.fullscreen_left(view);
                }
                false
            }
        };
        let on_print = {
            let weak = state.pages.clone();
            move |view: &gtk::Widget| !is_selected(&weak, view)
        };
        let on_web_process_terminated = {
            let weak = Rc::downgrade(&state);
            move || {
                if let Some(state) = weak.upgrade() {
                    state.cancel_loading();
                    state.answer_form_return(false);
                    let owner = state.owner();
                    let view = state.navigation.borrow().view_id();
                    if state.is_background() {
                        state.emit(PageEvent::Error(String::new()));
                        return;
                    }
                    let index = owner
                        .branches
                        .hidden
                        .borrow()
                        .iter()
                        .position(|h| h.page.view() == view);
                    if let Some(index) = index {
                        let hidden = owner.branches.hidden.borrow_mut().remove(index);
                        if let Ok(saved) = hidden.page.checkpoint() {
                            owner.remember_checkpoint(saved);
                        }
                        owner.receive(PageEvent::Changed);
                    }
                }
            }
        };
        let on_authenticate = {
            let weak = Rc::downgrade(&state);
            move |request: &dyn Responder| {
                if let Some(state) = Pages::enter_native(&weak) {
                    if state.is_background() {
                        request.refuse();
                        state.emit(PageEvent::Error(String::new()));
                        return true;
                    }
                }
                false
            }
        };
        let on_scrolled = {
            let weak = Rc::downgrade(&state);
            move |body: Option<zeroize::Zeroizing<String>>| {
                if let (Some(state), Some(value)) = (Pages::enter_native(&weak), body) {
                    state.scrolled(&value);
                }
            }
        };
        let on_load_changed = {
            let weak = Rc::downgrade(&state);
            move |event: LoadEvent| {
                let Some(state) = Pages::enter_native(&weak) else {
                    return;
                };
                let url = state.native.uri().and_then(|uri| web_url(&uri).ok());
                match event {
                    LoadEvent::Started => {
                        if matches!(*state.issue.borrow(), Issue::Cancelling(_)) {
                            return;
                        }
                        let bootstrap = url.is_none()
                            && state.navigation.borrow().current.is_none()
                            && state.navigation.borrow().pending_ticket().is_none();
                        state.end_response();
                        state.awaiting_response.set(!bootstrap);
                        state.main_frame_post.set(None);
                        if matches!(*state.issue.borrow(), Issue::Issued { decided: false, .. }) {
                            return;
                        }
                        // Policy also reports iframe reloads; Started belongs to the main frame.
                        let reload = state.reload_candidate.borrow_mut().take();
                        if reload.is_some() && state.navigation.borrow().pending_ticket().is_none()
                        {
                            let result = state
                                .owner()
                                .browser
                                .borrow_mut()
                                .reload(&mut state.navigation.borrow_mut());
                            if let Some(ticket) = state.navigation.borrow().pending_ticket() {
                                *state.issue.borrow_mut() = Issue::Issued {
                                    ticket,
                                    decided: true,
                                };
                            }
                            state.report(result.map(|_| ()));
                        }

                        state.same_document_request.borrow_mut().take();
                        state.traversal.borrow_mut().take();
                        let issued = state.issued.borrow_mut().take();
                        if let Some(url) = issued.or(url) {
                            state.start_attempt(url);
                            state.emit(PageEvent::Loading(true));
                        }
                    }
                    LoadEvent::Redirected => {
                        if let Some(url) = url {
                            if let Some(ticket) = state.loading_ticket() {
                                state.navigation.borrow_mut().redirected(ticket, url);
                            }
                        }
                    }
                    LoadEvent::Committed => {
                        *state.history_cursor.borrow_mut() = state.native.history().current_id();
                        let existing = state
                            .history
                            .borrow()
                            .iter()
                            .find(|entry| Some(entry.item) == *state.history_cursor.borrow())
                            .map(|entry| entry.document);
                        let traversal = state.back_forward_commit.replace(false);
                        let existing = existing.filter(|_| traversal);
                        let document = existing.unwrap_or_else(|| {
                            let next = state.next_history_document.get() + 1;
                            state.next_history_document.set(next);
                            next
                        });
                        state.history_document.set(document);
                        state.response_page.borrow_mut().take();
                        if let Some(url) = url {
                            let result = state.commit_attempt(url.clone());
                            match result {
                                Ok(true) => {
                                    state.bind_document();
                                    state.emit(PageEvent::Changed);
                                }
                                Ok(false) => {}
                                Err(error) => state.emit(PageEvent::Error(error.to_string())),
                            }
                        }
                        // The new document reports its own sign-in fields.
                        state.emit(PageEvent::Login(LoginEvent::NewDocument));
                        state.reconcile_find();
                    }
                    LoadEvent::Finished => {
                        if let Some(ticket) = state.loading_ticket() {
                            state.finish_loading(ticket);
                        }
                        let next = state
                            .issue
                            .borrow_mut()
                            .finish(&mut state.navigation.borrow_mut());
                        if let Some(next) = next {
                            state.load(next);
                            return;
                        }

                        state.complete_same_document();
                        state.update_title();
                        state.restore_scroll();
                        if state.download_handover.get().is_none() {
                            state.end_response();
                            state.emit(PageEvent::Loading(false));
                        }
                        state.look_for_login_fields();
                        let enabled = state
                            .owner()
                            .browser
                            .borrow()
                            .capture_enabled()
                            .unwrap_or(false);
                        if enabled {
                            state.capture(CaptureKind::Page, false);
                            if let Some(ticket) = state.ticket() {
                                for seconds in [1, 3] {
                                    let weak = Rc::downgrade(&state);
                                    let ticket = ticket.clone();
                                    glib::timeout_add_local_once(
                                        std::time::Duration::from_secs(seconds),
                                        move || {
                                            if let Some(state) = Pages::enter_native(&weak) {
                                                if state.valid(&ticket)
                                                    && state
                                                        .owner()
                                                        .browser
                                                        .borrow()
                                                        .capture_enabled()
                                                        .unwrap_or(false)
                                                {
                                                    state.capture(CaptureKind::Page, false);
                                                }
                                            }
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
        };
        let on_load_failed = {
            let weak = Rc::downgrade(&state);
            move |error: &LoadFailure| {
                let Some(state) = Pages::enter_native(&weak) else {
                    return true;
                };
                let ticket = state.loading_ticket();
                if let Some(ticket) = ticket {
                    state.finish_loading(ticket);
                }
                if ticket.is_none() || ticket != state.navigation.borrow().pending_ticket() {
                    return true;
                }
                if state.is_background() {
                    if state.navigation.borrow().current.is_some() {
                        return true;
                    }
                    state.emit(PageEvent::Error(String::new()));
                    return true;
                }
                let cancelled = error.kind == LoadFailureKind::Cancelled;
                if let Some(ticket) = ticket {
                    state.navigation.borrow_mut().failed(ticket);
                }
                // A response turned into a download ends the page load this way; the page
                // the user was reading stays.
                if error.kind != LoadFailureKind::InterruptedByPolicyChange && !cancelled {
                    state.emit(PageEvent::Error(format!(
                        "Could not load this page. {}",
                        error.message
                    )));
                    state.emit(PageEvent::Loading(false));
                }
                true
            }
        };
        let on_load_failed_with_tls_errors = {
            let weak = Rc::downgrade(&state);
            move || {
                if let Some(state) = Pages::enter_native(&weak) {
                    let ticket = state.active_ticket();
                    if let Some(ticket) = ticket {
                        state.finish_loading(ticket);
                    }
                    if ticket.is_none() || ticket != state.navigation.borrow().pending_ticket() {
                        return true;
                    }
                    if let Some(ticket) = ticket {
                        state.navigation.borrow_mut().failed(ticket);
                    }
                    state.emit(PageEvent::Error(
                        "HTTPS certificate rejected. The connection was not opened.".into(),
                    ));
                    state.emit(PageEvent::Loading(false));
                }
                true
            }
        };
        // Input the page receives is a GTK event on its widget, not an engine event.
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                widget.clone().upcast(),
                widget.connect_button_press_event(move |_, event| {
                    if event.button() == 1 && event.event_type() == gtk::gdk::EventType::ButtonPress
                    {
                        if let Some(state) = weak.upgrade() {
                            state.pressed.set(Some(std::time::Instant::now()));
                        }
                    }
                    glib::Propagation::Proceed
                }),
            ));
            let weak = Rc::downgrade(&state);
            signals.push((
                widget.clone().upcast(),
                widget.connect_key_press_event(move |_, event| {
                    use gtk::gdk::keys::constants as key;
                    if [key::Return, key::KP_Enter, key::ISO_Enter].contains(&event.keyval()) {
                        if let Some(state) = weak.upgrade() {
                            state.pressed.set(Some(std::time::Instant::now()));
                        }
                    }
                    glib::Propagation::Proceed
                }),
            ));
        }
        let on_title_changed = {
            let weak = Rc::downgrade(&state);
            move || {
                if let Some(state) = Pages::enter_native(&weak) {
                    state.update_title();
                }
            }
        };
        let on_playing_audio_changed = {
            // The chrome marks pages that play sound, so a start or stop redraws it.
            let weak = Rc::downgrade(&state);
            move || {
                if let Some(state) = Pages::enter_native(&weak) {
                    if !state.native.is_muted() {
                        state.emit(PageEvent::Changed);
                    }
                }
            }
        };
        let on_history_changed = {
            let weak = Rc::downgrade(&state);
            move |added: Option<HistoryItem>, removed: Vec<EntryId>, current: Option<EntryId>| {
                let Some(state) = Pages::enter_native(&weak) else {
                    return;
                };
                *state.history_cursor.borrow_mut() = current;
                for item in removed {
                    state
                        .history
                        .borrow_mut()
                        .retain(|entry| entry.item != item);
                }
                if added.is_some() && state.navigation.borrow().pending_ticket().is_none() {
                    let document = state.navigation.borrow().current.clone();
                    let route = document
                        .as_ref()
                        .zip(added.as_ref().and_then(|item| item.uri.clone()))
                        .and_then(|(document, uri)| state.clicked_route(&document.page_url, &uri));
                    if let Some(url) = route {
                        *state.same_document_request.borrow_mut() = Some(url);
                        state.complete_same_document();
                    } else if let Some(document) = document {
                        state.remember_entry(document.visit);
                        if let Some(url) = state.native.uri().and_then(|uri| web_url(&uri).ok()) {
                            state.navigation.borrow_mut().same_document(url);
                        }
                        state.bind_document();
                    }
                }
                let attempt = state.navigation.borrow().pending_ticket();
                let weak = Rc::downgrade(&state);
                glib::idle_add_local_once(move || {
                    if let Some(state) = Pages::enter_native(&weak).filter(|state| {
                        state.navigation.borrow().pending_ticket() == attempt
                            && *state.history_cursor.borrow() == current
                    }) {
                        state.history_changed(added.is_some());
                    }
                });
            }
        };
        let on_uri_changed = {
            let weak = Rc::downgrade(&state);
            move || {
                let Some(state) = Pages::enter_native(&weak) else {
                    return;
                };
                let attempt = state.navigation.borrow().pending_ticket();
                let weak = Rc::downgrade(&state);
                glib::idle_add_local_once(move || {
                    if let Some(state) = Pages::enter_native(&weak)
                        .filter(|state| state.navigation.borrow().pending_ticket() == attempt)
                    {
                        state.history_changed(false);
                    }
                });
            }
        };
        let on_loading_changed = {
            let weak = Rc::downgrade(&state);
            move || {
                if let Some(state) = Pages::enter_native(&weak) {
                    state.complete_same_document();
                }
            }
        };
        let on_decide = {
            let weak = Rc::downgrade(&state);
            move |decision: &PolicyDecision, kind: PolicyDecisionType| {
                if weak.upgrade().is_some_and(|state| {
                    state.lifecycle.get() != Lifecycle::Live || state.retired.get()
                }) {
                    decision.ignore();
                    return true;
                }
                Pages::enter_native(&weak).is_some_and(|state| state.decide(decision, kind))
            }
        };
        // The main resource's first request. Its later requests, after redirects, are heard
        // only when this one was: the answer `true` asks the engine for them.
        let on_main_resource_started = {
            let weak = Rc::downgrade(&state);
            move |request: &Request| {
                let Some(state) = Pages::enter_native(&weak) else {
                    return false;
                };
                state.note_main_frame_method(request);
                true
            }
        };
        let on_main_request_sent = {
            let weak = Rc::downgrade(&state);
            move |request: &Request| {
                if let Some(state) = Pages::enter_native(&weak) {
                    state.note_main_frame_method(request);
                }
            }
        };
        let weak = Rc::downgrade(&state);
        context.engine().attribute_downloads(
            widget,
            Box::new(move |source| {
                Pages::enter_native(&weak)
                    .map(|state| {
                        if state.is_background() {
                            state.emit(PageEvent::Error(String::new()));
                            return Err(anyhow::anyhow!("Never-seen page download refused"));
                        }
                        let page = state.download_page(source);
                        if let Some(ticket) = state.download_handover.take() {
                            let generation = state.load_generation.get();
                            let weak = Rc::downgrade(&state);
                            glib::idle_add_local_once(move || {
                                if let Some(state) = Pages::enter_native(&weak) {
                                    if state.load_generation.get() == generation
                                        && ticket.is_latest(&state.navigation.borrow())
                                    {
                                        state.emit(PageEvent::Loading(false));
                                    }
                                }
                            });
                        }
                        Ok(page)
                    })
                    .unwrap_or_else(|| Err(anyhow::anyhow!("Page destroyed")))
            }),
        );
        let favicons = Rc::downgrade(&context.favicons());
        let on_favicon_changed =
            move |uri: Option<String>, favicon: Option<gtk::cairo::Surface>| {
                let Some(favicons) = favicons.upgrade() else {
                    return;
                };
                if let (Some(uri), Some(surface)) = (uri, favicon) {
                    favicons.store(uri.to_string(), Some(&surface));
                }
            };
        // The page-size handler joins after the view is placed, as its WebKit signal did.
        let scale_slot: Rc<RefCell<Option<ScaleLoadHandler>>> = Rc::default();
        // One observer hears every event of the view and runs the handlers above, as their
        // WebKit signals did. Handlers that shared a WebKit signal run in the order they were
        // connected, and stop once the page lets the view go, as disconnected handlers did.
        let observer: Observer = {
            let weak = Rc::downgrade(&state);
            let view = widget.downgrade();
            let scale_slot = scale_slot.clone();
            Rc::new(move |event| {
                let Some(view) = view.upgrade() else {
                    return false;
                };
                match event {
                    ViewEvent::Load(event) => {
                        on_load_changed(event);
                        let on_scale = scale_slot.borrow().clone();
                        if let (true, Some(on_scale)) = (attached(&weak), on_scale) {
                            on_scale(event);
                        }
                        false
                    }
                    ViewEvent::LoadFailed(failure) => on_load_failed(&failure),
                    ViewEvent::LoadFailedWithTlsErrors => on_load_failed_with_tls_errors(),
                    ViewEvent::UriChanged => {
                        on_uri_changed();
                        false
                    }
                    ViewEvent::TitleChanged => {
                        on_title_changed();
                        false
                    }
                    ViewEvent::FaviconChanged { uri, favicon } => {
                        on_favicon_changed(uri, favicon);
                        false
                    }
                    ViewEvent::LoadingChanged => {
                        on_loading_changed();
                        false
                    }
                    ViewEvent::ProgressChanged => {
                        loading::progress_changed(&weak);
                        false
                    }
                    ViewEvent::PlayingAudioChanged => {
                        on_playing_audio_changed();
                        false
                    }
                    ViewEvent::HistoryChanged {
                        added,
                        removed,
                        current,
                    } => {
                        on_history_changed(added, removed, current);
                        false
                    }
                    ViewEvent::Decide(decision, kind) => on_decide(&decision, kind),
                    ViewEvent::MainResourceStarted(request) => on_main_resource_started(&request),
                    ViewEvent::MainRequestSent(request) => {
                        on_main_request_sent(&request);
                        false
                    }
                    ViewEvent::ScriptMessage { name, body } => {
                        if name == SCROLLED {
                            on_scrolled(body);
                        } else {
                            login_fields::message(&weak, &name, body);
                        }
                        false
                    }
                    ViewEvent::ScriptDialog(dialog) => on_script_dialog(&view, &*dialog),
                    ViewEvent::PermissionRequest(request) => {
                        on_permission_request(&view, &*request)
                    }
                    ViewEvent::FileChooser(request) => on_file_chooser(&view, &*request),
                    ViewEvent::Print => on_print(&view),
                    ViewEvent::EnterFullscreen => on_enter_fullscreen(&view),
                    ViewEvent::LeaveFullscreen => on_leave_fullscreen(&view),
                    ViewEvent::WebProcessTerminated => {
                        on_web_process_terminated();
                        false
                    }
                    ViewEvent::Authenticate(request) => on_authenticate(&*request),
                    ViewEvent::Close => false,
                    ViewEvent::FoundText(count) => {
                        find::found_text(&weak, count);
                        false
                    }
                    ViewEvent::FailedToFindText => {
                        find::failed_to_find_text(&weak);
                        false
                    }
                }
            })
        };
        // Installed before the view is placed: placing it can already report a load, which
        // the page's own handlers heard when they were WebKit signals.
        view.set_observer(Some(observer));
        state.owner().register_view(widget);
        let (scale_signals, on_scale_load_changed) = page_scale::setup(&state);
        signals.extend(scale_signals);
        *scale_slot.borrow_mut() = Some(Rc::new(on_scale_load_changed));
        Ok(Self {
            last_shown: Cell::new(0),
            native: view.clone(),
            state,
            signals: RefCell::new(signals),
        })
    }

    #[cfg(feature = "gui-smoke")]
    pub(super) fn revival_refusals(&self) -> usize {
        self.state.revival_refusals.get()
    }

    pub(super) fn history_visits(&self) -> Vec<VisitId> {
        self.state
            .history
            .borrow()
            .iter()
            .map(|e| e.visit)
            .collect()
    }

    #[cfg(feature = "gui-smoke")]
    pub(super) fn forget_history_visit(&self, visit: VisitId) {
        self.state.history.borrow_mut().retain(|e| e.visit != visit);
    }

    fn detach(&self) {
        self.state.cancel_loading();
        if self.state.pages.upgrade().is_some() {
            self.answer_form_return(false);
        } else if let Some(decision) = self.state.form_return.borrow_mut().take() {
            decision.1.ignore();
        }
        self.state.end_response();
        if self.state.lifecycle.replace(Lifecycle::Released) == Lifecycle::Released {
            return;
        }
        self.native.set_observer(None);
        let signals = self.signals.take();
        for (object, id) in signals {
            // GTK can destroy the widget before its owner releases the page.
            let connected = unsafe {
                glib::gobject_ffi::g_signal_handler_is_connected(object.as_ptr(), id.as_raw()) != 0
            };
            if connected {
                object.disconnect(id);
            }
        }
        self.state.history.borrow_mut().clear();
        self.state.history_cursor.borrow_mut().take();
        self.state.traversal.borrow_mut().take();
        self.state.navigation.borrow_mut().clear();
        let widget = self.native.widget();
        if let Some(parent) = widget
            .parent()
            .and_then(|p| p.downcast::<gtk::Container>().ok())
        {
            parent.remove(widget);
        }
        unsafe {
            widget.destroy();
        }
    }

    pub(super) fn capture_checkpoint_scroll(&self, done: impl FnOnce() + 'static) {
        let state = &self.state;
        state.cancel_loading();
        if let Some(owner) = state.pages.upgrade() {
            owner.discard_view_continuation(&state.navigation.borrow());
        }
        let ticket = state.navigation.borrow().current.clone();
        state.lifecycle.set(Lifecycle::CapturingScroll);
        let pending = state.navigation.borrow().pending_ticket();
        if let Some(ticket) = pending {
            state.navigation.borrow_mut().failed(ticket);
        }
        *state.issue.borrow_mut() = Issue::Idle;
        state
            .load_generation
            .set(state.load_generation.get().wrapping_add(1));
        state
            .foreground_generation
            .set(state.foreground_generation.get().wrapping_add(1));
        state.same_document_request.borrow_mut().take();
        state.traversal.borrow_mut().take();
        state.issued.borrow_mut().take();
        self.native.stop_loading();
        if ticket.is_none() {
            done();
            return;
        }
        let completion = Rc::new(RefCell::new(Some(done)));
        let timeout_completion = completion.clone();
        let cancel = gio::Cancellable::new();
        let timeout_cancel = cancel.clone();
        let timeout =
            glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                timeout_cancel.cancel();
                if let Some(done) = timeout_completion.borrow_mut().take() {
                    done();
                }
            });
        let state = Rc::downgrade(&self.state);
        let owner = self.state.pages.clone();
        self.native.evaluate(
            "JSON.stringify({url:location.href,y:scrollY,epoch:globalThis.__canopyDocumentEpoch})",
            WORLD,
            Some(&cancel),
            Some(Box::new(move |value| {
                let Some(done) = completion.borrow_mut().take() else {
                    return;
                };
                timeout.remove();
                if let (Some(state), Some(owner), Some(ticket), Ok(value)) =
                    (state.upgrade(), owner.upgrade(), ticket, value)
                {
                    if let Ok(value) = serde_json::from_str::<Value>(&value) {
                        if state.lifecycle.get() != Lifecycle::Released
                            && owner
                                .read(|b| b.contains_visit(ticket.visit))
                                .unwrap_or(false)
                            && checkpoint_document_matches(&value, &ticket)
                        {
                            if let Some(y) = value["y"].as_f64().filter(|y| y.is_finite()) {
                                if y != 0.
                                    || owner
                                        .browser
                                        .borrow()
                                        .scroll(ticket.visit)
                                        .ok()
                                        .flatten()
                                        .is_some()
                                {
                                    let _ = owner.browser.borrow_mut().scroll_for_document(
                                        &state.navigation.borrow(),
                                        &ticket,
                                        y.clamp(0., 1e8) as i64,
                                    );
                                }
                            }
                        }
                    }
                }
                done();
            })),
        );
    }

    // The per-branch PR calls the checkpoint lifecycle.
    #[cfg_attr(not(feature = "gui-smoke"), allow(dead_code))]
    pub(super) fn checkpoint(&self) -> Result<Checkpoint> {
        let state = &self.state;
        let owner = state.pages.upgrade().context("Page owner was closed.")?;
        let list = self.native.history();
        let (items, current) = ordered_entries(&*self.native, &list)?;
        let entries = {
            let history = state.history.borrow();
            anyhow::ensure!(
                history.len() == items.len(),
                "History binding count differs."
            );
            items
                .iter()
                .map(|item| {
                    let mut matches = history.iter().filter(|e| e.item == *item);
                    let entry = matches.next().context("Unbound history entry.")?;
                    anyhow::ensure!(matches.next().is_none(), "Duplicate history binding.");
                    Ok(CheckpointEntry {
                        visit: entry.visit,
                        document: entry.document,
                        post: entry.post,
                        uri: web_url(
                            list.find(*item)
                                .and_then(|item| item.uri.as_deref())
                                .context("History entry has no URL.")?,
                        )?,
                        stored_url: owner.browser.borrow().visit(entry.visit)?.url,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        };
        let bytes = self.native.session_state().context("No session state.")?;
        Ok(Checkpoint {
            last_shown: self.last_shown.get(),
            bytes,
            entries,
            current,
        })
    }

    // The per-branch PR calls the checkpoint lifecycle.
    #[cfg_attr(not(feature = "gui-smoke"), allow(dead_code))]
    pub(super) fn revive(
        container: &gtk::Fixed,
        pages: Weak<Pages>,
        context: &Rc<super::context::PageContext>,
        saved: &Checkpoint,
        visit: VisitId,
    ) -> Result<(Self, Revival)> {
        let page = Self::build(container, pages.clone(), context, InitialPage::Session)?;
        page.native.restore_session_state(&saved.bytes);
        let binding = (|| -> Result<Vec<HistoryEntry>> {
            let shows_target = |e: &&CheckpointEntry| e.visit == visit && e.uri == e.stored_url;
            let _target = saved
                .entries
                .get(saved.current)
                .filter(shows_target)
                .or_else(|| saved.entries.iter().rev().find(shows_target))
                .context("Visit has no traversable entry in checkpoint.")?;

            let list = page.native.history();
            let (items, current) = ordered_entries(&*page.native, &list)?;
            let urls = items
                .iter()
                .map(|i| {
                    list.find(*i)
                        .and_then(|i| i.uri.as_deref())
                        .and_then(|u| web_url(u).ok())
                })
                .collect::<Option<Vec<_>>>()
                .context("Invalid history URL.")?;
            let stored = saved
                .entries
                .iter()
                .map(|e| {
                    page.state
                        .owner()
                        .browser
                        .borrow()
                        .visit(e.visit)
                        .map(|v| v.url)
                })
                .collect::<Result<Vec<_>>>()?;
            anyhow::ensure!(
                valid_binding(saved, &urls, current, &stored),
                "Invalid checkpoint binding."
            );
            Ok(items
                .into_iter()
                .zip(&saved.entries)
                .map(|(item, e)| HistoryEntry {
                    item,
                    visit: e.visit,
                    document: e.document,
                    post: e.post,
                    needs_get: e.post,
                })
                .collect())
        })();
        match binding {
            Ok(history) => {
                *page.state.history.borrow_mut() = history;
                *page.state.history_cursor.borrow_mut() = page.native.history().current_id();
                page.state
                    .next_history_document
                    .set(saved.entries.iter().map(|e| e.document).max().unwrap_or(0));
                Ok((page, Revival::History))
            }
            Err(_) => {
                drop(page);
                Ok((
                    Self::build(container, pages, context, InitialPage::Session)?,
                    Revival::Cold,
                ))
            }
        }
    }

    pub fn focus(&self) {
        self.native.widget().grab_focus();
        self.state.owner().place_view(self.native.widget());
    }

    pub fn navigate(&self, url: Url, target: NavigationTarget) -> Result<()> {
        let url = self.state.owner().browser.borrow_mut().navigate(
            &mut self.state.navigation.borrow_mut(),
            url,
            target,
        )?;
        self.state.load(url);
        Ok(())
    }

    /// Shows the home page in place of the current one and drops any load in flight, for a
    /// window with no current page.
    pub fn blank(&self) {
        let state = &self.state;
        state.cancel_loading();
        state
            .load_generation
            .set(state.load_generation.get().wrapping_add(1));
        state
            .foreground_generation
            .set(state.foreground_generation.get().wrapping_add(1));
        state.same_document_request.borrow_mut().take();
        state.traversal.borrow_mut().take();
        state.issued.borrow_mut().take();
        state.pending_scroll.set(None);
        state.reload_candidate.borrow_mut().take();
        state.back_forward_commit.set(false);
        *state.issue.borrow_mut() = Issue::Idle;
        state.navigation.borrow_mut().clear();
        self.native.load_html(HOME);
    }

    pub fn restore(&self, id: VisitId) -> Result<()> {
        let url = self
            .state
            .owner()
            .browser
            .borrow_mut()
            .restore(&mut self.state.navigation.borrow_mut(), id)?;
        self.state.load(url);
        Ok(())
    }

    pub(super) fn restore_revived(&self, id: VisitId) -> Result<()> {
        anyhow::ensure!(
            self.state.lifecycle.get() == Lifecycle::Live,
            "Page no longer live."
        );
        let url = self.state.owner().browser.borrow().visit(id)?.url;
        let target = self.state.entry(id, &url);
        let was_current = target.is_some() && target == self.native.history().current_id();
        self.restore(id)?;
        if !was_current {
            return Ok(());
        }
        let saved = self
            .state
            .owner()
            .browser
            .borrow()
            .scroll(id)
            .ok()
            .flatten();
        self.state.pending_scroll.set(saved.map(|y| (id, y)));
        Ok(())
    }

    pub fn reload(&self) -> Result<()> {
        let request = self
            .state
            .owner()
            .browser
            .borrow_mut()
            .reload(&mut self.state.navigation.borrow_mut())?;
        if let Some(url) = request {
            self.state.load(url);
        }
        Ok(())
    }

    pub fn open_citation(&self, child: VisitId) -> Result<()> {
        let request = self
            .state
            .owner()
            .browser
            .borrow_mut()
            .open_citation(&mut self.state.navigation.borrow_mut(), child)?;
        if let Some(url) = request {
            self.state.load(url);
        }
        Ok(())
    }

    pub(super) fn open_saved(&self, capture: &crate::browser::Capture) -> Result<()> {
        let url = self
            .state
            .owner()
            .browser
            .borrow()
            .open_saved_link(&mut self.state.navigation.borrow_mut(), capture)?;
        self.state.load(url);
        Ok(())
    }

    pub fn remember_selection(&self) -> Result<()> {
        if self.state.ticket().is_none() {
            bail!("Open a page and select the text to remember.")
        }
        self.state.capture(CaptureKind::Selection, true);
        Ok(())
    }

    pub fn set_capture_enabled(&self, enabled: bool) -> Result<()> {
        self.state
            .owner()
            .browser
            .borrow_mut()
            .set_capture_enabled(enabled)?;
        if enabled {
            self.state.capture(CaptureKind::Page, false);
        }
        Ok(())
    }
}

impl NativeState {
    fn active_ticket(&self) -> Option<LoadTicket> {
        match *self.issue.borrow() {
            Issue::Idle => None,
            Issue::Issued { ticket, .. }
            | Issue::Loading(ticket)
            | Issue::Cancelling(ticket)
            | Issue::Retiring { ticket, .. } => Some(ticket),
        }
    }

    fn background_visit(&self) -> Option<VisitId> {
        self.never_seen.get()
    }

    fn is_background(&self) -> bool {
        self.never_seen.get().is_some() && !self.adopted.get()
    }

    fn start_attempt(&self, url: Url) {
        let mut navigation = self.navigation.borrow_mut();
        if matches!(*self.issue.borrow(), Issue::Issued { decided: false, .. }) {
            return;
        }
        let ticket = match self.active_ticket() {
            Some(ticket) => ticket,
            None => {
                if navigation.pending_ticket().is_some() {
                    return;
                }
                let ticket = if let Some(visit) = self.never_seen.get() {
                    navigation.request_queued(visit, url.clone())
                } else {
                    navigation.native_request(url.clone())
                };
                *self.issue.borrow_mut() = Issue::Loading(ticket);
                ticket
            }
        };
        if matches!(*self.issue.borrow(), Issue::Issued { .. }) {
            *self.issue.borrow_mut() = Issue::Loading(ticket);
        }
        navigation.started(ticket);
        drop(navigation);
        self.loading.borrow_mut().started(ticket);
        self.owner().publish_loading();
    }

    fn accept_issue(&self) {
        if let Issue::Issued { decided, .. } = &mut *self.issue.borrow_mut() {
            *decided = true;
        }
    }

    /// Response metadata belongs only to the current load ticket.
    fn end_response(&self) {
        self.main_frame_post.set(None);
        self.download_handover.set(None);
        self.awaiting_response.set(false);
        self.response_page.borrow_mut().take();
    }

    fn loading_ticket(&self) -> Option<LoadTicket> {
        match *self.issue.borrow() {
            Issue::Loading(ticket) | Issue::Cancelling(ticket) | Issue::Retiring { ticket, .. } => {
                Some(ticket)
            }
            _ => None,
        }
    }

    fn note_main_frame_method(&self, request: &Request) {
        self.main_frame_post
            .set(self.loading_ticket().map(|ticket| {
                let post = request.http_method().as_deref() == Some("POST");
                if post {
                    // A redirect to GET changes the history method but still follows a POST.
                    self.navigation.borrow_mut().mark_form_post(ticket);
                }
                (ticket, post)
            }));
    }

    fn commit_attempt(&self, url: Url) -> Result<bool> {
        let Some(ticket) = self.loading_ticket() else {
            return Ok(false);
        };
        let owner = self.owner();
        let foreground = owner
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| p.owns(ticket))
            || owner
                .branches
                .candidate
                .borrow()
                .as_ref()
                .is_some_and(|p| p.owns(ticket));
        let committed = owner.browser.borrow_mut().commit_load(
            &mut self.navigation.borrow_mut(),
            ticket,
            url,
            if foreground {
                Visibility::Selected
            } else {
                Visibility::Hidden
            },
        )?;
        if committed {
            self.report(self.apply_page_scale());
            self.owner().publish_loading();
            let visit = self
                .navigation
                .borrow()
                .current
                .as_ref()
                .expect("committed document")
                .visit;
            self.owner().committed(ticket, visit);
            self.owner().publish_page_scale();
        }
        Ok(committed)
    }

    fn owner(&self) -> Rc<Pages> {
        self.pages
            .upgrade()
            .expect("page owner is alive during callbacks")
    }

    fn emit(&self, event: PageEvent) {
        if self.lifecycle.get() == Lifecycle::Live {
            let (view, empty) = {
                let navigation = self.navigation.borrow();
                (
                    navigation.view_id(),
                    navigation.current.is_none() && navigation.pending_ticket().is_none(),
                )
            };
            self.owner().receive_view(view, empty, event);
        }
    }

    fn report(&self, result: Result<()>) {
        if let Err(error) = result {
            self.emit(PageEvent::Error(error.to_string()));
        }
    }

    fn bind_document(&self) {
        let document = self.navigation.borrow().current.clone();
        if let Some(document) = document {
            self.remember_entry(document.visit);
            self.native.evaluate(
                &format!("globalThis.__canopyDocumentEpoch = {}", document.epoch),
                WORLD,
                None,
                None,
            );
        }
        self.reconcile_find();
    }

    fn ticket(&self) -> Option<Document> {
        let navigation = self.navigation.borrow();
        let document = navigation.current.as_ref()?;
        if !navigation.valid(document)
            || self.native.uri().as_deref() != Some(document.page_url.as_str())
            || self.current_entry_document() != Some(self.history_document.get())
        {
            return None;
        }
        Some(document.clone())
    }

    fn valid(&self, ticket: &Document) -> bool {
        self.ticket().as_ref() == Some(ticket)
    }

    fn foreground_requested(&self) {
        let owner = self.owner();
        let selected = owner
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| p.native.widget() == self.native.widget());
        if selected {
            owner.cancel_candidate();
        }
    }

    fn load(self: &Rc<Self>, url: Url) {
        self.foreground_requested();
        self.end_response();
        self.awaiting_response.set(true);
        // Every caller registers its intent first; an address with no attempt has no visit to
        // commit to.
        let Some(requested) = self.navigation.borrow().pending_ticket() else {
            return;
        };
        self.begin_loading(requested);
        {
            let mut issue = self.issue.borrow_mut();
            if let Issue::Retiring { next, .. } = &mut *issue {
                *next = (requested, url);
                return;
            }
            if let Issue::Cancelling(ticket) = *issue {
                *issue = Issue::Retiring {
                    ticket,
                    next: (requested, url),
                };
                return;
            }
        }
        let generation = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(generation);
        self.foreground_generation
            .set(self.foreground_generation.get().wrapping_add(1));
        self.same_document_request.borrow_mut().take();
        self.traversal.borrow_mut().take();
        self.pending_scroll.set(None);
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = Pages::enter_native(&weak) {
                if state.load_generation.get() == generation
                    && state.navigation.borrow().pending_ticket() == Some(requested)
                {
                    {
                        let mut issue = state.issue.borrow_mut();
                        if let Issue::Retiring { next, .. } = &mut *issue {
                            *next = (requested, url);
                            return;
                        }
                    }
                    let retiring = match *state.issue.borrow() {
                        Issue::Loading(ticket) if state.native.is_loading() => Some(ticket),
                        _ => None,
                    };
                    if let Some(ticket) = retiring {
                        *state.issue.borrow_mut() = Issue::Retiring {
                            ticket,
                            next: (requested, url),
                        };
                        state.native.stop_loading();
                        return;
                    }
                    *state.issue.borrow_mut() = Issue::Issued {
                        ticket: requested,
                        decided: false,
                    };
                    let current = state.native.uri().and_then(|uri| web_url(&uri).ok());
                    let same_document = state.navigation.borrow().current.is_some()
                        && !state.native.is_loading()
                        && current
                            .is_some_and(|current| current != url && same_resource(&current, &url));
                    if same_document {
                        *state.same_document_request.borrow_mut() = Some(url.clone());
                    } else {
                        *state.issued.borrow_mut() = Some(url.clone());
                    }
                    let returning = state
                        .owner()
                        .browser
                        .borrow()
                        .requested_restore(&state.navigation.borrow());
                    state
                        .owner()
                        .context
                        .blocking
                        .loading(&*state.native, Some(&url));
                    if let Some(entry) = returning.and_then(|visit| state.entry(visit, &url)) {
                        *state.traversal.borrow_mut() = Some((requested, entry));
                        state.native.go_to(entry);
                    } else {
                        if let Some(visit) = returning.filter(|_| !same_document) {
                            let saved = state.owner().browser.borrow().scroll(visit).ok().flatten();
                            state
                                .pending_scroll
                                .set(saved.filter(|&y| y > 0).map(|y| (visit, y)));
                        }
                        state.native.load_uri(url.as_str());
                    }
                    state.complete_same_document();
                }
            }
        });
    }

    fn complete_same_document(self: &Rc<Self>) {
        if self.native.is_loading() {
            return;
        }
        let Some(target) = self.same_document_request.borrow().clone() else {
            return;
        };
        if self.native.uri().as_deref() != Some(target.as_str()) {
            return;
        }
        self.same_document_request.borrow_mut().take();
        self.traversal.borrow_mut().take();
        self.accept_issue();
        self.start_attempt(target.clone());
        let ticket = self.loading_ticket();
        let title_at_commit = self.native.title();
        let committed = self.commit_attempt(target.clone());
        if let Some(ticket) = ticket {
            self.finish_loading(ticket);
        }
        self.awaiting_response.set(false);
        match committed {
            Ok(true) => {
                *self.issue.borrow_mut() = Issue::Idle;
                self.bind_document();
                self.update_title();
                self.emit(PageEvent::Changed);
                self.capture_route_later(title_at_commit);
            }
            Ok(false) => {
                self.issue
                    .borrow_mut()
                    .finish(&mut self.navigation.borrow_mut());
            }
            Err(error) => {
                self.issue
                    .borrow_mut()
                    .finish(&mut self.navigation.borrow_mut());
                self.emit(PageEvent::Error(error.to_string()));
            }
        }
    }

    fn evaluate(
        self: &Rc<Self>,
        script: &str,
        callback: impl FnOnce(Rc<Self>, Option<Value>) + 'static,
    ) {
        let weak = Rc::downgrade(self);
        self.native.evaluate(
            script,
            WORLD,
            None,
            Some(Box::new(move |result| {
                let Some(state) = Pages::enter_native(&weak) else {
                    return;
                };
                let value = result
                    .ok()
                    .and_then(|value| serde_json::from_str(value.as_str()).ok());
                callback(state.state.clone(), value);
            })),
        );
    }

    fn context(
        self: &Rc<Self>,
        target: &Url,
        callback: impl FnOnce(Rc<Self>, Option<SourceContext>) + 'static,
    ) {
        let target = serde_json::to_string(target.as_str()).expect("a URL serializes as JSON");
        let script =
            format!("JSON.stringify(globalThis.__canopyMemory?.takeContext({target}) ?? null)");
        self.evaluate(&script, move |state, value| {
            let context = value.and_then(|value| parse_context(&value).ok());
            callback(state, context);
        });
    }

    fn answer_form_return(self: &Rc<Self>, proceed: bool) {
        let held = self.form_return.borrow_mut().take();
        let Some((_, decision)) = held else { return };
        self.emit(PageEvent::FormReturn(None));
        if proceed {
            if !self.decide_navigation(&decision, PolicyDecisionType::NavigationAction) {
                let target = decision
                    .navigation_action()
                    .and_then(|action| action.request())
                    .and_then(|request| request.uri())
                    .and_then(|uri| web_url(&uri).ok());
                self.owner()
                    .context
                    .blocking
                    .loading(&*self.native, target.as_ref());
                decision.use_();
            }
        } else {
            self.owner()
                .discard_view_continuation(&self.navigation.borrow());
            let ticket = self.navigation.borrow().pending_ticket();
            if let Some(ticket) = ticket {
                self.navigation.borrow_mut().failed(ticket);
            }
            self.cancel_loading();
            *self.issue.borrow_mut() = Issue::Idle;
            self.issued.borrow_mut().take();
            self.traversal.borrow_mut().take();
            self.pending_scroll.set(None);
            self.end_response();
            self.form_return_cancelled.set(true);
            decision.ignore();
        }
    }

    fn decide(self: &Rc<Self>, decision: &PolicyDecision, kind: PolicyDecisionType) -> bool {
        if kind == PolicyDecisionType::NavigationAction {
            if let Some(action) = decision.navigation_action() {
                // Preserve the form-return guard independently of switch intent.
                if action.request().and_then(|r| r.uri()) == self.native.uri() {
                    self.answer_form_return(false);
                }
                // NavigationAction has no main-frame flag. A missing target name is
                // the closest signal available (unnamed subframes are ambiguous).
                let main_action = action.frame_name().is_none_or(|name| name == "_top");
                if main_action {
                    let modifiers = action.modifiers();
                    let background = action.navigation_type() == NavigationType::LinkClicked
                        && (action.mouse_button() == 2
                            || modifiers.intersects(
                                gdk::ModifierType::CONTROL_MASK
                                    | gdk::ModifierType::META_MASK
                                    | gdk::ModifierType::SHIFT_MASK,
                            ));
                    if action.is_user_gesture()
                        && !background
                        && action.request().and_then(|r| r.uri()).is_some_and(|uri| {
                            self.navigation
                                .borrow()
                                .current
                                .as_ref()
                                .is_some_and(|document| {
                                    web_url(&uri).is_ok_and(|url| {
                                        !same_resource(&document.page_url, &url)
                                            || url.fragment().is_none()
                                            || matches!(
                                                action.navigation_type(),
                                                NavigationType::FormSubmitted
                                                    | NavigationType::FormResubmitted
                                                    | NavigationType::Reload
                                            )
                                    })
                                })
                        })
                    {
                        self.foreground_requested();
                    }
                }
                let returning = action.navigation_type() == NavigationType::BackForward
                    && action
                        .request()
                        .is_some_and(|r| r.http_method().as_deref() == Some("POST"));
                if returning || action.navigation_type() == NavigationType::FormResubmitted {
                    let main = action.frame_name().is_none()
                        && (action.navigation_type() != NavigationType::BackForward
                            || self.current_entry_document() != Some(self.history_document.get()));
                    let restored = self
                        .history
                        .borrow()
                        .iter()
                        .any(|e| e.needs_get && Some(e.item) == *self.history_cursor.borrow());
                    if main && !restored {
                        let owner = self.owner();
                        owner.context.blocking.hold_navigation(&*self.native);
                        let candidate = owner.branches.candidate.borrow().clone();
                        if let Some(page) = candidate.filter(|p| {
                            p.native.widget() == self.native.widget()
                                && owner.branches.candidate_live.get().is_some()
                        }) {
                            if let Err(error) = page.select() {
                                owner.cancel_candidate();
                                decision.ignore();
                                owner.receive(PageEvent::Error(error.to_string()));
                                return true;
                            }
                            owner.branches.candidate.borrow_mut().take();
                            owner.branches.candidate_live.take();
                            owner.promote_live(page);
                        }
                        let selected = self
                            .owner()
                            .branches
                            .live
                            .borrow()
                            .as_ref()
                            .is_some_and(|p| p.native.widget() == self.native.widget());
                        if selected && !self.is_background() {
                            self.foreground_requested();
                            static QUESTIONS: std::sync::atomic::AtomicU64 =
                                std::sync::atomic::AtomicU64::new(1);
                            let question =
                                QUESTIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            self.form_return.replace(Some((question, decision.clone())));
                            self.emit(PageEvent::FormReturn(Some(question)));
                        } else {
                            decision.ignore();
                        }
                        return true;
                    }
                }
            }
        }
        self.decide_navigation(decision, kind)
    }

    fn decide_navigation(
        self: &Rc<Self>,
        decision: &PolicyDecision,
        kind: PolicyDecisionType,
    ) -> bool {
        if kind == PolicyDecisionType::Response {
            let Some(response) = decision.response() else {
                return false;
            };
            let main = response.is_main_frame_main_resource();
            if main {
                self.awaiting_response.set(false);
                let url = response
                    .request()
                    .and_then(|request| request.uri())
                    .and_then(|uri| web_url(&uri).ok());
                let noted = url.map(|url| {
                    let page = self
                        .owner()
                        .browser
                        .borrow()
                        .download_page(&self.navigation.borrow(), Some(&url));
                    (self.loading_ticket(), url, page.ok().flatten())
                });
                self.response_page.replace(
                    noted.and_then(|(ticket, url, page)| ticket.map(|ticket| (ticket, url, page))),
                );
            }
            if main
                && self.is_background()
                && response
                    .response()
                    .is_some_and(|r| matches!(r.status_code(), 204 | 205))
            {
                decision.ignore();
                self.emit(PageEvent::Error(String::new()));
                return true;
            }
            let attachment = main
                && response
                    .response()
                    .and_then(|r| r.content_disposition())
                    .is_some_and(|disposition| disposition.eq_ignore_ascii_case("attachment"));
            if response.is_mime_type_supported() && !attachment {
                return false;
            }
            if self.is_background() {
                decision.ignore();
                self.emit(PageEvent::Error(String::new()));
                return true;
            }
            if main {
                if let Some(ticket) = self.loading_ticket() {
                    self.finish_loading(ticket);
                }
                self.download_handover.set(self.loading_ticket());
                decision.download();
            } else {
                decision.ignore();
            }
            return true;
        }
        let Some(action) = decision.navigation_action() else {
            return false;
        };
        let Some(uri) = action.request().and_then(|request| request.uri()) else {
            return false;
        };
        if uri == "about:blank" {
            return false;
        }
        let Ok(url) = web_url(&uri) else {
            // Frames load about:, blob: and data: documents all the time; only a link the
            // user clicked deserves a refusal and a message.
            if action.navigation_type() != NavigationType::LinkClicked || !action.is_user_gesture()
            {
                return false;
            }
            decision.ignore();
            self.emit(PageEvent::Error(
                "This kind of link is not supported.".into(),
            ));
            return true;
        };
        if action.navigation_type() == NavigationType::BackForward
            && self.history.borrow().iter().any(|entry| {
                entry.needs_get
                    && Some(entry.item) == *self.history_cursor.borrow()
                    && entry.document != self.history_document.get()
                    && self
                        .native
                        .history()
                        .find(entry.item)
                        .and_then(|item| item.uri.as_deref())
                        == Some(uri.as_str())
            })
        {
            #[cfg(feature = "gui-smoke")]
            self.revival_refusals.set(self.revival_refusals.get() + 1);
            decision.ignore();
            if self.history_document.get() != 0 {
                if let Some(visit) = self.current_entry_visit() {
                    self.report(self.owner().cold_restore(visit));
                }
                return true;
            }
            // At initial revival the POST is current in the native list. Its GET
            // appends after it; page Back skips the duplicate visit.
            if let Some(document) = self.current_entry_document() {
                self.history_document.set(document);
            }
            self.back_forward_commit.set(false);
            let visit = self.current_entry_visit();
            if let Some(visit) = visit {
                let result = self
                    .owner()
                    .browser
                    .borrow_mut()
                    .restore(&mut self.navigation.borrow_mut(), visit);
                match result {
                    Ok(url) => self.load(url),
                    Err(error) => self.emit(PageEvent::Error(error.to_string())),
                }
            } else {
                self.owner()
                    .context
                    .blocking
                    .loading(&*self.native, Some(&url));
                self.native.load_uri(url.as_str());
            }
            return true;
        }
        if action.navigation_type() != NavigationType::Reload {
            self.reload_candidate.borrow_mut().take();
        }
        let modifiers = action.modifiers();
        let background = kind == PolicyDecisionType::NewWindowAction
            || action.mouse_button() == 2
            || modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::META_MASK
                    | gdk::ModifierType::SHIFT_MASK,
            );
        if background && action.navigation_type() == NavigationType::LinkClicked {
            decision.ignore();
            if !action.is_user_gesture() {
                return true;
            }
            let Some(ticket) = self.ticket() else {
                return true;
            };
            let source = SourceContext {
                page_url: ticket.page_url.clone(),
                citation_url: None,
                link_text: String::new(),
                excerpt: String::new(),
            };
            let result =
                self.owner()
                    .browser
                    .borrow_mut()
                    .queue_link(ticket.visit, url.clone(), source);
            match result {
                Ok(child) => {
                    self.owner().queue_background(child);
                    self.emit(PageEvent::Queued(child));
                    let queued_url = url.clone();
                    self.context(&url, move |state, context| {
                        if !state.valid(&ticket) {
                            return;
                        }
                        if let Some(source) =
                            context.filter(|source| source.page_url == ticket.page_url)
                        {
                            let result = state.owner().browser.borrow_mut().source_for_document(
                                &state.navigation.borrow(),
                                &ticket,
                                child,
                                source.clone(),
                            );
                            match result {
                                Ok(true) => {
                                    state.emit(PageEvent::Changed);
                                    state.emit(PageEvent::Waiting {
                                        parent: ticket.visit,
                                        label: if source.link_text.trim().is_empty() {
                                            queued_url.to_string()
                                        } else {
                                            source.link_text
                                        },
                                    });
                                }
                                Ok(false) => {}
                                Err(error) => state.emit(PageEvent::Error(error.to_string())),
                            }
                        } else {
                            state.emit(PageEvent::Waiting {
                                parent: ticket.visit,
                                label: queued_url.to_string(),
                            });
                        }
                    });
                }
                Err(error) => self.emit(PageEvent::Error(error.to_string())),
            }
            return true;
        }
        if action.navigation_type() != NavigationType::BackForward {
            self.back_forward_commit.set(false);
        }
        if action.navigation_type() == NavigationType::Reload
            && self
                .navigation
                .borrow()
                .current
                .as_ref()
                .is_some_and(|document| document.page_url == url)
            && self.navigation.borrow().pending_ticket().is_none()
        {
            *self.reload_candidate.borrow_mut() = Some(url.clone());
        } else if action.navigation_type() == NavigationType::BackForward {
            if matches!(*self.issue.borrow(), Issue::Retiring { .. }) {
                decision.ignore();
                return true;
            }
            self.back_forward_commit.set(true);
            if self
                .traversal
                .borrow()
                .as_ref()
                .is_some_and(|(ticket, item)| {
                    Some(*ticket) == self.active_ticket()
                        && *self.history_cursor.borrow() == Some(*item)
                })
            {
                self.accept_issue();
                return false;
            }
            let returning = self
                .current_entry_visit()
                .filter(|visit| self.native_history_allowed(*visit));
            if returning.is_some()
                && self.current_entry_document() == Some(self.history_document.get())
            {
                return false;
            }
            if let Some(visit) = returning {
                let result = self
                    .owner()
                    .browser
                    .borrow_mut()
                    .restore(&mut self.navigation.borrow_mut(), visit);
                if let Some(ticket) = self.navigation.borrow().pending_ticket() {
                    *self.issue.borrow_mut() = Issue::Issued {
                        ticket,
                        decided: true,
                    };
                    self.begin_loading(ticket);
                }
                self.report(result.map(|_| ()));
            } else {
                decision.ignore();
                let result = self
                    .owner()
                    .browser
                    .borrow_mut()
                    .back(&mut self.navigation.borrow_mut());
                match result {
                    Ok(Some(url)) => self.load(url),
                    Ok(None) => {}
                    Err(error) => self.emit(PageEvent::Error(error.to_string())),
                }
                return true;
            }
        } else if action.navigation_type() == NavigationType::LinkClicked
            && action.is_user_gesture()
        {
            let Some(ticket) = self.ticket() else {
                return false;
            };
            if same_resource(&ticket.page_url, &url) {
                return false;
            }
            let generation = self.foreground_generation.get().wrapping_add(1);
            self.foreground_generation.set(generation);
            let decision = decision.clone();
            self.context(&url.clone(), move |state, context| {
                if !state.valid(&ticket) || state.foreground_generation.get() != generation {
                    decision.ignore();
                    return;
                }
                if let Some(source) = context.filter(|source| source.page_url == ticket.page_url) {
                    decision.ignore();
                    let request = state.owner().browser.borrow_mut().follow_link(
                        &mut state.navigation.borrow_mut(),
                        ticket.visit,
                        url,
                        source,
                    );
                    match request {
                        Ok(url) => state.load(url),
                        Err(error) => state.emit(PageEvent::Error(error.to_string())),
                    }
                } else {
                    decision.use_();
                }
            });
            return true;
        }
        if self.issued.borrow().as_ref() == Some(&url) {
            self.accept_issue();
        }
        false
    }

    fn native_history_allowed(&self, visit: VisitId) -> bool {
        let owner = self.owner();
        let browser = owner.browser.borrow();
        let projection = (|| -> Result<_> {
            crate::forest::ForestProjection::from_snapshot(
                browser.snapshot()?,
                browser.tree_states()?,
            )
        })();
        let Ok(forest) = projection else { return false };
        let Some(tree) = self
            .navigation
            .borrow()
            .current
            .as_ref()
            .and_then(|d| forest.tree_of(d.visit))
        else {
            return false;
        };
        forest.tree_of(visit) == Some(tree) && forest.tabs().iter().any(|open| open.id == tree)
    }

    fn current_entry_visit(&self) -> Option<VisitId> {
        let current = (*self.history_cursor.borrow())?;
        self.history
            .borrow()
            .iter()
            .find(|entry| entry.item == current)
            .map(|entry| entry.visit)
    }

    fn current_entry_document(&self) -> Option<u64> {
        let current = (*self.history_cursor.borrow())?;
        self.history
            .borrow()
            .iter()
            .find(|entry| entry.item == current)
            .map(|entry| entry.document)
    }

    fn history_changed(self: &Rc<Self>, added: bool) {
        if self.native.is_loading() && self.navigation.borrow().pending_ticket().is_some() {
            return;
        }
        // A native cross-document traversal can move the list before Started.
        if !added && self.current_entry_document() != Some(self.history_document.get()) {
            return;
        }
        self.complete_same_document();
        if self.navigation.borrow().pending_ticket().is_some() {
            return;
        }
        let Some(url) = self.native.uri().and_then(|uri| web_url(&uri).ok()) else {
            return;
        };
        if let Some(visit) = self.current_entry_visit() {
            if !self.native_history_allowed(visit) {
                return;
            }
            let different = self
                .navigation
                .borrow()
                .current
                .as_ref()
                .is_some_and(|d| d.visit != visit);
            if different {
                let Ok(stored) = self.owner().browser.borrow().visit(visit) else {
                    return;
                };
                self.navigation.borrow_mut().publish(visit, stored.url);
            }
        }
        if self.is_background() {
            let result = self
                .owner()
                .browser
                .borrow_mut()
                .update_unopened_url(&mut self.navigation.borrow_mut(), url.clone());
            self.report(result);
        }
        self.navigation.borrow_mut().same_document(url);
        self.owner().publish_loading();
        if let Some(document) = self.ticket() {
            self.bind_document();
            if is_selected(&self.pages, self.native.widget()) {
                let result = self
                    .owner()
                    .browser
                    .borrow_mut()
                    .select_document(&self.navigation.borrow(), &document);
                self.report(result);
                self.owner().publish_page_scale();
            }
            self.emit(PageEvent::Changed);
        }
    }

    /// Records which visit WebKit's current history entry shows.
    /// The address a page script moved to just after the person clicked or pressed Enter in
    /// the page, when it is another resource than `page_url` (not only another `#part`).
    /// A press counts for the first history entry after it only, so a `#part` link spends it.
    /// WebKit adds the entry before the view reports its new URI.
    fn clicked_route(&self, page_url: &Url, entry_uri: &str) -> Option<Url> {
        let pressed = self.pressed.take()?;
        if pressed.elapsed() > ROUTE_AFTER_PRESS || self.is_background() {
            return None;
        }
        let url = web_url(entry_uri).ok()?;
        (url.origin() == page_url.origin() && !same_resource(&url, page_url)).then_some(url)
    }

    fn remember_entry(&self, visit: VisitId) {
        let Some(current) = *self.history_cursor.borrow() else {
            return;
        };
        let live = self.native.history().live();
        let mut history = self.history.borrow_mut();
        let previous = history
            .iter()
            .find(|entry| entry.item == current)
            .or_else(|| {
                history
                    .iter()
                    .rev()
                    .find(|entry| entry.document == self.history_document.get())
            });
        let method = self
            .main_frame_post
            .take()
            .filter(|(ticket, _)| Some(*ticket) == self.loading_ticket())
            .map(|(_, post)| post);
        let post = method.unwrap_or_else(|| previous.is_some_and(|entry| entry.post));
        let needs_get = method.is_none() && previous.is_some_and(|entry| entry.needs_get);
        history.retain(|entry| entry.item != current && live.contains(&entry.item));
        history.push(HistoryEntry {
            item: current,
            visit,
            document: self.history_document.get(),
            post,
            needs_get,
        });
    }

    /// A history entry other than the current one that still shows `visit` at `url`.
    fn entry(&self, visit: VisitId, url: &Url) -> Option<EntryId> {
        let list = self.native.history();
        let current = list.current_id();
        let live = list.live();
        let history = self.history.borrow();
        let preferred = history
            .iter()
            .filter(|entry| self.history_document.get() == 0 && Some(entry.item) == current);
        preferred
            .chain(history.iter().rev())
            .find(|entry| {
                entry.visit == visit
                    && (!entry.needs_get || self.history_document.get() == 0)
                    && entry.document != self.history_document.get()
                    && (self.history_document.get() == 0 || Some(entry.item) != current)
                    && live.contains(&entry.item)
                    && list.find(entry.item).and_then(|item| item.uri.as_deref())
                        == Some(url.as_str())
            })
            .map(|entry| entry.item)
    }

    fn scrolled(&self, message: &str) {
        let Ok(value) = serde_json::from_str::<Value>(message) else {
            return;
        };
        let (Some(page), Some(y)) = (value["url"].as_str(), value["y"].as_f64()) else {
            return;
        };
        let Some(ticket) = self.ticket() else { return };
        if value["epoch"].as_u64() != Some(ticket.epoch)
            || ticket.page_url.as_str() != page
            || !y.is_finite()
        {
            return;
        }
        // Best effort: a failed write only loses where this page was left.
        let _ = self.owner().browser.borrow_mut().scroll_for_document(
            &self.navigation.borrow(),
            &ticket,
            y.clamp(0., 1e8) as i64,
        );
    }

    fn restore_scroll(self: &Rc<Self>) {
        if !self.native.widget().is_mapped() {
            return;
        }
        // A cancelled earlier load can finish first; keep the target for the visit it names.
        let Some((visit, y)) = self.pending_scroll.get() else {
            return;
        };
        if self.ticket().is_some_and(|ticket| ticket.visit == visit) {
            self.pending_scroll.set(None);
            let (width, height) = self.owner().viewport.get();
            let viewport = format!(
                "[{},{}]",
                width.max(1),
                (height - super::CHROME_HEIGHT as i32).max(1)
            );
            let script = format!(
                "JSON.stringify(globalThis.__canopyMemory?.restoreScroll({y}, {viewport}) ?? null)"
            );
            self.evaluate(&script, |_, _| {});
        }
    }

    /// The page a download from `source` that starts now belongs to.
    fn download_page(&self, source: Option<&Url>) -> Option<VisitId> {
        let noted = self.response_page.borrow_mut().take_if(|(ticket, url, _)| {
            Some(&*url) == source
                && (self.loading_ticket() == Some(*ticket)
                    || self.download_handover.get() == Some(*ticket))
        });
        let page = match noted {
            Some((_, _, page)) => page,
            None => {
                let page = self
                    .owner()
                    .browser
                    .borrow()
                    .download_page(&self.navigation.borrow(), source);
                page.ok().flatten()
            }
        };
        page.or_else(|| {
            self.navigation
                .borrow()
                .current
                .is_none()
                .then_some(self.download_origin)
                .flatten()
        })
    }

    fn update_title(&self) {
        if let (Some(document), Some(title)) = (self.ticket(), self.native.title()) {
            let result = self.owner().browser.borrow_mut().title_for_document(
                &self.navigation.borrow(),
                &document,
                &title,
            );
            self.report(result);
            self.emit(PageEvent::Changed);
        }
    }

    /// A script route changes the address before the site swaps in the new page. YouTube
    /// still shows the previous page's text or title a second later, so the text is read
    /// once the route has settled, and never while the old page's title is up.
    fn capture_route_later(self: &Rc<Self>, old_title: Option<String>) {
        let Some(ticket) = self.ticket() else { return };
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(ROUTE_SETTLED, move || {
            let Some(state) = Pages::enter_native(&weak) else {
                return;
            };
            if state.valid(&ticket)
                && state.native.title() != old_title
                && state
                    .owner()
                    .browser
                    .borrow()
                    .capture_enabled()
                    .unwrap_or(false)
            {
                state.capture(CaptureKind::Page, false);
            }
        });
    }

    fn capture(self: &Rc<Self>, kind: CaptureKind, explicit: bool) {
        let Some(ticket) = self.ticket() else { return };
        let selection = matches!(kind, CaptureKind::Selection);
        let script =
            format!("JSON.stringify(globalThis.__canopyMemory?.capture({selection}) ?? null)");
        self.evaluate(&script, move |state, value| {
            if !state.valid(&ticket) {
                return;
            }
            if !explicit
                && !state
                    .owner()
                    .browser
                    .borrow()
                    .capture_enabled()
                    .unwrap_or(false)
            {
                return;
            }
            let Some(value) = value else { return };
            if value
                .get("blocked")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                if explicit {
                    state.emit(PageEvent::Error(
                        "Form fields and pages with a password are never remembered.".into(),
                    ));
                }
                return;
            }
            let Ok(payload) = parse_capture(&value, kind) else {
                return;
            };
            if payload.document_url != ticket.page_url {
                return;
            }
            if payload.text.trim().is_empty() {
                if explicit {
                    state.emit(PageEvent::Error(
                        "Select the text of the post or article to remember.".into(),
                    ));
                }
                return;
            }
            let result = state.owner().browser.borrow_mut().capture_for_document(
                &state.navigation.borrow(),
                &ticket,
                payload,
            );
            match result {
                Ok(Some(id)) => {
                    state.emit(PageEvent::Changed);
                    if explicit {
                        state.emit(PageEvent::Remembered(id));
                    }
                }
                Ok(None) => {}
                Err(error) => state.emit(PageEvent::Error(error.to_string())),
            }
        });
    }
}

/// Whether the page still hears its view: a page that let its view go hears nothing more, not
/// even the rest of the event it was handling.
fn attached(weak: &Weak<NativeState>) -> bool {
    weak.upgrade()
        .is_some_and(|state| state.lifecycle.get() != Lifecycle::Released)
}

fn is_selected(weak: &Weak<Pages>, view: &gtk::Widget) -> bool {
    weak.upgrade().is_some_and(|pages| {
        pages
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| p.native.widget() == view && !p.retired())
    })
}

fn ordered_entries(view: &dyn EngineView, list: &History) -> Result<(Vec<EntryId>, usize)> {
    let back = list.back.len();
    let forward = list.forward.len();
    let entries = (-(back as i32)..=forward as i32)
        .map(|i| view.history_nth(i).context("Missing history position."))
        .collect::<Result<Vec<_>>>()?;
    anyhow::ensure!(
        entries.len() == list.length as usize,
        "Native history length differs."
    );
    anyhow::ensure!(
        entries.get(back).copied() == list.current_id(),
        "Native history cursor differs."
    );
    Ok((entries, back))
}

fn valid_binding(saved: &Checkpoint, urls: &[Url], current: usize, stored: &[Url]) -> bool {
    !saved.entries.is_empty()
        && saved.current == current
        && current < urls.len()
        && saved.entries.len() == urls.len()
        && stored.len() == urls.len()
        && saved
            .entries
            .iter()
            .zip(urls)
            .zip(stored)
            .all(|((e, uri), stored)| e.uri == *uri && e.stored_url == *stored)
}

fn checkpoint_document_matches(value: &Value, document: &Document) -> bool {
    value["url"].as_str() == Some(document.page_url.as_str())
        && value["epoch"].as_u64() == Some(document.epoch)
}

fn string(value: &Value, name: &str, limit: usize) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(limit)
        .collect()
}

fn citation(value: &Value, name: &str, page: &Url) -> Option<Url> {
    value
        .get(name)
        .and_then(Value::as_str)
        .and_then(|value| web_url(value).ok())
        .filter(|url| url.origin() == page.origin())
}

fn parse_context(value: &Value) -> Result<SourceContext> {
    let page_url = web_url(&string(value, "page_url", 8192))?;
    Ok(SourceContext {
        citation_url: citation(value, "citation_url", &page_url),
        page_url,
        link_text: string(value, "link_text", 300),
        excerpt: string(value, "excerpt", 1000),
    })
}

fn parse_capture(value: &Value, kind: CaptureKind) -> Result<CapturePayload> {
    let document_url = web_url(&string(value, "document_url", 8192))?;
    Ok(CapturePayload {
        citation_url: citation(value, "citation_url", &document_url),
        document_url,
        title: string(value, "title", 1000),
        text: string(value, "text", 200000),
        kind,
    })
}

const HOME: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><title>Canopy</title>
<style>body{margin:0;background:#f6f5f0;color:#29362e;font:18px system-ui;display:grid;place-content:center;height:100vh}main{max-width:520px;padding:40px}.mark{font-size:56px;color:#53795b}h1{font-weight:550;font-size:36px;letter-spacing:-1px}p{line-height:1.7;color:#647168}</style>
<main><div class="mark">⌘</div><h1>A trail starts here.</h1><p>Open a page, follow a link, then come back to explore another direction. Your branches stay within reach.</p></main></html>"#;

#[cfg(test)]
mod attempt_tests {
    use super::*;
    use crate::browser::Browser;

    #[test]
    fn finished_failed_commit_cannot_rewrite_restored_visit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("forest.sqlite3");
        let mut browser = Browser::open(&path).unwrap();
        let mut navigation = Navigation::default();
        let url = |path| Url::parse(&format!("https://example.org/{path}")).unwrap();
        let first = browser
            .navigate(&mut navigation, url("a"), NavigationTarget::Root)
            .unwrap();
        let ticket = navigation.pending_ticket().unwrap();
        navigation.started(ticket);
        browser
            .commit_load(&mut navigation, ticket, first, Visibility::Selected)
            .unwrap();
        let a = navigation.current.as_ref().unwrap().visit;
        browser.restore(&mut navigation, a).unwrap();
        let ticket = navigation.pending_ticket().unwrap();
        navigation.started(ticket);
        let mut issue = Issue::Loading(ticket);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TRIGGER reject BEFORE UPDATE ON tree_state BEGIN SELECT RAISE(ABORT, 'disk'); END;").unwrap();
        assert!(browser
            .commit_load(&mut navigation, ticket, url("a"), Visibility::Selected)
            .is_err());
        connection.execute_batch("DROP TRIGGER reject").unwrap();
        assert!(issue.finish(&mut navigation).is_none());
        assert!(
            navigation.pending_ticket().is_none(),
            "Finished must abandon the failed commit"
        );
        let ticket = navigation.native_request(url("elsewhere"));
        navigation.started(ticket);
        browser
            .commit_load(
                &mut navigation,
                ticket,
                url("elsewhere"),
                Visibility::Selected,
            )
            .unwrap();
        assert_eq!(browser.visit(a).unwrap().url, url("a"));
        assert_ne!(navigation.current.as_ref().unwrap().visit, a);
    }
}

#[cfg(test)]
mod checkpoint_tests {
    #[test]
    fn same_url_checkpoint_scroll_requires_the_document_epoch() {
        let mut navigation = Navigation::default();
        let directory = tempfile::tempdir().unwrap();
        let mut browser =
            crate::browser::Browser::open(&directory.path().join("forest.sqlite3")).unwrap();
        let visit = browser
            .add_roots(&[Url::parse("https://example.org/").unwrap()])
            .unwrap()[0];
        let url = Url::parse("https://example.org/").unwrap();
        navigation.publish(visit, url.clone());
        let old = navigation.current.clone().unwrap();
        navigation.publish(visit, url.clone());
        let new = navigation.current.clone().unwrap();
        let value = serde_json::json!({"url": url.as_str(), "y": 0, "epoch": new.epoch});
        assert!(!checkpoint_document_matches(&value, &old));
        assert!(checkpoint_document_matches(&value, &new));
    }
    use super::*;

    #[test]
    fn binding_requires_every_position_cursor_and_stored_visit_url() {
        let directory = tempfile::tempdir().unwrap();
        let mut browser =
            crate::browser::Browser::open(&directory.path().join("forest.sqlite3")).unwrap();
        let a = Url::parse("https://example.org/a").unwrap();
        let route = Url::parse("https://example.org/route").unwrap();
        let b = Url::parse("https://example.org/b").unwrap();
        let ids = browser.add_roots(&[a.clone(), b.clone()]).unwrap();
        let mut saved = Checkpoint {
            last_shown: 0,
            bytes: glib::Bytes::from_static(&[]),
            entries: vec![
                CheckpointEntry {
                    visit: ids[0],
                    document: 1,
                    post: false,
                    uri: a.clone(),
                    stored_url: a.clone(),
                },
                CheckpointEntry {
                    visit: ids[0],
                    document: 1,
                    post: false,
                    uri: route.clone(),
                    stored_url: a.clone(),
                },
                CheckpointEntry {
                    visit: ids[1],
                    document: 2,
                    post: false,
                    uri: b.clone(),
                    stored_url: b.clone(),
                },
            ],
            current: 1,
        };
        let urls = [a.clone(), route, b.clone()];
        let stored = [a.clone(), a.clone(), b.clone()];
        assert!(valid_binding(&saved, &urls, 1, &stored));
        assert!(!valid_binding(&saved, &urls[..2], 1, &stored));
        assert!(!valid_binding(&saved, &urls, 0, &stored));
        assert!(!valid_binding(
            &saved,
            &[b.clone(), urls[1].clone(), a.clone()],
            1,
            &stored
        ));
        assert!(!valid_binding(&saved, &urls, 1, &[a.clone(), b, a]));
        saved.current = 3;
        assert!(!valid_binding(&saved, &urls, 3, &stored));
    }
}
