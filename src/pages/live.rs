#[path = "loading.rs"]
pub(super) mod loading;

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
use webkit2gtk::AuthenticationRequestExt;

use anyhow::{bail, Context, Result};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use url::Url;
use webkit2gtk::{
    BackForwardListExt, BackForwardListItem, BackForwardListItemExt, LoadEvent,
    NavigationPolicyDecision, NavigationPolicyDecisionExt, NavigationType, NetworkError,
    PolicyDecision, PolicyDecisionExt, PolicyDecisionType, PolicyError, ResponsePolicyDecision,
    ResponsePolicyDecisionExt, URIRequestExt, URIResponseExt, UserContentInjectedFrames,
    UserContentManagerExt, UserScript, UserScriptInjectionTime, WebViewExt,
};
#[path = "find.rs"]
pub(super) mod find;
#[path = "login_fields.rs"]
mod login_fields;
#[path = "mouse.rs"]
mod mouse;
#[path = "page_scale.rs"]
pub(super) mod page_scale;

use crate::{
    address::web_url,
    browser::{
        CaptureKind, CapturePayload, Document, LoadTicket, Navigation, NavigationTarget,
        SourceContext, Visibility, VisitId,
    },
};

const WORLD: &str = "canopy-reading-memory";
/// How long after a click or Enter a script route still counts as the person's step. Sites
/// like YouTube fetch the next page's data before they change the address.
const ROUTE_AFTER_PRESS: std::time::Duration = std::time::Duration::from_secs(4);
const HELPER: &str = include_str!("../capture.js");
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

enum InitialPage {
    Home,
    Session,
}

struct HistoryEntry {
    item: BackForwardListItem,
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
    native: webkit2gtk::WebView,
    lifecycle: Cell<Lifecycle>,
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
    history_cursor: RefCell<Option<BackForwardListItem>>,
    /// The history entry this engine asked WebKit for, which the policy check lets through.
    traversal: RefCell<Option<(LoadTicket, BackForwardListItem)>>,
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
    pub(super) native: webkit2gtk::WebView,
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
        self.native.set_is_muted(false);
    }
    /// Undoes [`Self::adopt`] when the page goes back to the background list unseen: muted again,
    /// counted in a load slot and subject to the background time limit.
    pub(super) fn unadopt(&self) {
        self.state.adopted.set(false);
        self.native.set_is_muted(true);
    }
    pub(super) fn background(&self, visit: VisitId) -> Result<()> {
        self.state.never_seen.set(Some(visit));
        self.native.set_is_muted(true);
        self.native.set_child_visible(false);
        self.restore(visit)
    }
    pub(super) fn pending_ticket(&self) -> Option<LoadTicket> {
        self.state.navigation.borrow().pending_ticket()
    }

    pub(super) fn empty(
        container: &gtk::Fixed,
        pages: Weak<Pages>,
        context: &Rc<super::native::PageContext>,
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
        self.native.set_is_muted(true);
        self.native.set_sensitive(false);
        self.cancel();
        self.detach();
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
        let Some(list) = self.native.back_forward_list() else {
            return false;
        };
        let entries = live_entries(&list);
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
        let current = self
            .native
            .back_forward_list()
            .and_then(|l| l.current_item());
        let restored = self.state.lifecycle.get() == Lifecycle::Released
            || self.state.history.borrow().iter().any(|e| {
                Some(&e.item) == current.as_ref() && e.document == self.state.history_document.get()
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
    pub(super) fn retire(&self) {
        self.native.set_sensitive(false);
        self.native.set_is_muted(true);
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
            self.native.set_is_muted(false);
        }
        if let Some(window) = self.native.window() {
            window.set_pass_through(false);
        }
        self.native.set_sensitive(true);
        self.native.set_child_visible(true);
        self.native.show();
        self.state.owner().place_view(&self.native);
        self.state.restore_scroll();
    }
    pub(super) fn native_back(&self) -> Option<BackForwardListItem> {
        let list = self.native.back_forward_list()?;
        let mut items = list.back_list();
        // WebKit returns nearest first; frame and pushState entries share a document.
        items.sort_by_key(|item| {
            (1..=list.length())
                .find(|n| list.nth_item(-(*n as i32)).as_ref() == Some(item))
                .unwrap_or(u32::MAX)
        });
        let allowed = |item: &&BackForwardListItem| {
            self.state
                .history
                .borrow()
                .iter()
                .any(|e| e.item == **item && self.state.native_history_allowed(e.visit))
        };
        items
            .iter()
            .filter(allowed)
            .find(|item| {
                self.state
                    .history
                    .borrow()
                    .iter()
                    .any(|e| e.item == **item && Some(e.visit) != self.current_visit())
            })
            .or_else(|| {
                if self.back_visit().ok().flatten().is_some() {
                    return None;
                }
                items
                    .iter()
                    .filter(allowed)
                    .find(|item| item.uri() != self.native.uri())
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
                .find(|e| e.item == item)
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
        context: &Rc<super::native::PageContext>,
    ) -> Result<Self> {
        Self::build(container, pages, context, InitialPage::Home)
    }

    fn build(
        container: &gtk::Fixed,
        pages: Weak<Pages>,
        context: &Rc<crate::pages::native::PageContext>,
        initial: InitialPage,
    ) -> Result<Self> {
        use webkit2gtk::{InputMethodContextExt, SettingsExt};
        let native = webkit2gtk::WebView::builder()
            .web_context(&context.native)
            .user_content_manager(&webkit2gtk::UserContentManager::new())
            .is_controlled_by_automation(false)
            .website_policies(
                &webkit2gtk::WebsitePolicies::builder()
                    .autoplay(webkit2gtk::AutoplayPolicy::Allow)
                    .build(),
            )
            .build();
        if let Some(input) = native.input_method_context() {
            input.set_enable_preedit(false);
        }
        let settings = WebViewExt::settings(&native).context("Page settings are unavailable.")?;
        settings.set_enable_webgl(true);
        settings.set_enable_webaudio(true);
        settings.set_enable_back_forward_navigation_gestures(false);
        settings.set_enable_page_cache(true);
        settings.set_user_agent(None);
        settings.set_enable_developer_extras(cfg!(debug_assertions));
        let mut signals = Vec::new();
        signals.push((native.clone().upcast(), native.connect_close(|_| {})));
        signals.extend(
            mouse::setup(&native, pages.clone())
                .into_iter()
                .map(|id| (native.clone().upcast(), id)),
        );
        container.put(&native, 0, 0);
        native.set_child_visible(matches!(initial, InitialPage::Home));
        native.show();
        if container.is_realized() {
            native.realize();
        }
        if matches!(initial, InitialPage::Home) {
            native.load_html(HOME, None);
        }
        if matches!(initial, InitialPage::Home) {
            native.show_all();
            native.grab_focus();
        }
        let manager = native
            .user_content_manager()
            .context("The JavaScript context is unavailable.")?;
        manager.add_script(&UserScript::for_world(
            HELPER,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::Start,
            WORLD,
            &[],
            &[],
        ));
        let state = Rc::new(NativeState {
            native: native.clone(),
            lifecycle: Cell::new(Lifecycle::Live),
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
        // WebKit defaults may present modal UI even for an unmapped view.
        {
            let weak = state.pages.clone();
            signals.push((
                native.clone().upcast(),
                native.connect_script_dialog(move |view, dialog| {
                    if !is_selected(&weak, view) {
                        if matches!(
                            dialog.dialog_type(),
                            webkit2gtk::ScriptDialogType::Confirm
                                | webkit2gtk::ScriptDialogType::BeforeUnloadConfirm
                        ) {
                            dialog.confirm_set_confirmed(false);
                        }
                        true
                    } else {
                        false
                    }
                }),
            ));
            let weak = state.pages.clone();
            signals.push((
                native.clone().upcast(),
                native.connect_permission_request(move |view, request| {
                    if !is_selected(&weak, view) {
                        use webkit2gtk::PermissionRequestExt;
                        request.deny();
                        true
                    } else {
                        false
                    }
                }),
            ));
            let weak = state.pages.clone();
            signals.push((
                native.clone().upcast(),
                native.connect_run_file_chooser(move |view, request| {
                    if !is_selected(&weak, view) {
                        use webkit2gtk::FileChooserRequestExt;
                        request.cancel();
                        true
                    } else {
                        false
                    }
                }),
            ));
            let weak = state.pages.clone();
            signals.push((
                native.clone().upcast(),
                native.connect_enter_fullscreen(move |view| !is_selected(&weak, view)),
            ));
            let weak = state.pages.clone();
            signals.push((
                native.clone().upcast(),
                native.connect_print(move |view, _| !is_selected(&weak, view)),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_web_process_terminated(move |_, _| {
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
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_authenticate(move |_, request| {
                    if let Some(state) = Pages::enter_native(&weak) {
                        if state.is_background() {
                            request.cancel();
                            state.emit(PageEvent::Error(String::new()));
                            return true;
                        }
                    }
                    false
                }),
            ));
        }
        manager.register_script_message_handler_in_world(SCROLLED, WORLD);
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                manager.clone().upcast(),
                manager.connect_script_message_received(Some(SCROLLED), move |_, message| {
                    if let (Some(state), Some(value)) =
                        (Pages::enter_native(&weak), message.js_value())
                    {
                        state.scrolled(&value.to_str());
                    }
                }),
            ));
        }
        signals.push((native.clone().upcast(), loading::setup(&state)));
        signals.extend(
            login_fields::setup(&manager, &state)
                .into_iter()
                .map(|id| (manager.clone().upcast(), id)),
        );
        signals.extend(find::setup(&state));
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_load_changed(move |native, event| {
                    let Some(state) = Pages::enter_native(&weak) else {
                        return;
                    };
                    let url = native.uri().and_then(|uri| web_url(&uri).ok());
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
                            if matches!(*state.issue.borrow(), Issue::Issued { decided: false, .. })
                            {
                                return;
                            }
                            // Policy also reports iframe reloads; Started belongs to the main frame.
                            let reload = state.reload_candidate.borrow_mut().take();
                            if reload.is_some()
                                && state.navigation.borrow().pending_ticket().is_none()
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
                            *state.history_cursor.borrow_mut() = native
                                .back_forward_list()
                                .and_then(|list| list.current_item());
                            let existing = state
                                .history
                                .borrow()
                                .iter()
                                .find(|entry| {
                                    Some(&entry.item) == state.history_cursor.borrow().as_ref()
                                })
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
                        _ => {}
                    }
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_load_failed(move |_, _, _uri, error| {
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
                    let cancelled = error.matches(NetworkError::Cancelled);
                    if let Some(ticket) = ticket {
                        state.navigation.borrow_mut().failed(ticket);
                    }
                    // A response turned into a download ends the page load this way; the page
                    // the user was reading stays.
                    if !error.matches(PolicyError::FrameLoadInterruptedByPolicyChange) && !cancelled
                    {
                        state.emit(PageEvent::Error(format!(
                            "Could not load this page. {}",
                            error.message()
                        )));
                        state.emit(PageEvent::Loading(false));
                    }
                    true
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_load_failed_with_tls_errors(move |_, _uri, _, _| {
                    if let Some(state) = Pages::enter_native(&weak) {
                        let ticket = state.active_ticket();
                        if let Some(ticket) = ticket {
                            state.finish_loading(ticket);
                        }
                        if ticket.is_none() || ticket != state.navigation.borrow().pending_ticket()
                        {
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
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_button_press_event(move |_, event| {
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
                native.clone().upcast(),
                native.connect_key_press_event(move |_, event| {
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
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_title_notify(move |_| {
                    if let Some(state) = Pages::enter_native(&weak) {
                        state.update_title();
                    }
                }),
            ));
        }
        if let Some(list) = native.back_forward_list() {
            let weak = Rc::downgrade(&state);
            signals.push((
                list.clone().upcast(),
                list.connect_local("changed", false, move |values| {
                    let state = Pages::enter_native(&weak)?;
                    let list: webkit2gtk::BackForwardList = values[0].get().unwrap();
                    let added: Option<BackForwardListItem> = values[1].get().unwrap();
                    let current = list.current_item();
                    *state.history_cursor.borrow_mut() = current.clone();
                    let removed: glib::Pointer = values[2].get().unwrap_or(std::ptr::null_mut());
                    // WebKit keeps removed objects readable while our bindings retain them.
                    unsafe {
                        use glib::object::ObjectType;
                        let mut node = removed as *mut glib::ffi::GList;
                        while !node.is_null() {
                            let item =
                                (*node).data as *mut webkit2gtk::ffi::WebKitBackForwardListItem;
                            state
                                .history
                                .borrow_mut()
                                .retain(|entry| entry.item.as_ptr() != item);
                            node = (*node).next;
                        }
                    }
                    if added.is_some() && state.navigation.borrow().pending_ticket().is_none() {
                        let document = state.navigation.borrow().current.clone();
                        let route = document
                            .as_ref()
                            .zip(added.as_ref().and_then(|item| item.uri()))
                            .and_then(|(document, uri)| {
                                state.clicked_route(&document.page_url, &uri)
                            });
                        if let Some(url) = route {
                            *state.same_document_request.borrow_mut() = Some(url);
                            state.complete_same_document();
                        } else if let Some(document) = document {
                            state.remember_entry(document.visit);
                            if let Some(url) = state.native.uri().and_then(|uri| web_url(&uri).ok())
                            {
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
                    None
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_uri_notify(move |_| {
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
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_is_loading_notify(move |_| {
                    if let Some(state) = Pages::enter_native(&weak) {
                        state.complete_same_document();
                    }
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_decide_policy(move |_, decision, kind| {
                    if weak.upgrade().is_some_and(|state| {
                        state.lifecycle.get() != Lifecycle::Live || !state.native.is_sensitive()
                    }) {
                        decision.ignore();
                        return true;
                    }
                    Pages::enter_native(&weak).is_some_and(|state| state.decide(decision, kind))
                }),
            ));
        }
        {
            let weak = Rc::downgrade(&state);
            signals.push((
                native.clone().upcast(),
                native.connect_resource_load_started(move |native, resource, request| {
                    if native.main_resource().as_ref() != Some(resource) {
                        return;
                    }
                    let Some(state) = Pages::enter_native(&weak) else {
                        return;
                    };
                    state.main_frame_post.set(
                        state.loading_ticket().map(|ticket| {
                            (ticket, request.http_method().as_deref() == Some("POST"))
                        }),
                    );
                    let weak = Rc::downgrade(&state);
                    // The typed binding treats the absent redirect response as non-null.
                    resource.connect_local("sent-request", false, move |values| {
                        // A panic here would cross the signal boundary and abort the browser.
                        let (Some(Ok(resource)), Some(Ok(request))) = (
                            values.first().map(|v| v.get::<webkit2gtk::WebResource>()),
                            values.get(1).map(|v| v.get::<webkit2gtk::URIRequest>()),
                        ) else {
                            return None;
                        };
                        if let Some(state) = Pages::enter_native(&weak) {
                            if state.native.main_resource().as_ref() == Some(&resource) {
                                state
                                    .main_frame_post
                                    .set(state.loading_ticket().map(|ticket| {
                                        (ticket, request.http_method().as_deref() == Some("POST"))
                                    }));
                            }
                        }
                        None
                    });
                }),
            ));
        }
        let weak = Rc::downgrade(&state);
        let favicon = context.register_view(&native, move |source| {
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
        });
        signals.push((native.clone().upcast(), favicon));
        state.owner().register_view(&native);
        signals.extend(page_scale::setup(&state));
        Ok(Self {
            last_shown: Cell::new(0),
            native,
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
        if let Some(parent) = self
            .native
            .parent()
            .and_then(|p| p.downcast::<gtk::Container>().ok())
        {
            parent.remove(&self.native);
        }
        unsafe {
            self.native.destroy();
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
        self.native.evaluate_javascript(
            "JSON.stringify({url:location.href,y:scrollY,epoch:globalThis.__canopyDocumentEpoch})",
            Some(WORLD),
            None,
            Some(&cancel),
            move |value| {
                let Some(done) = completion.borrow_mut().take() else {
                    return;
                };
                timeout.remove();
                if let (Some(state), Some(owner), Some(ticket), Ok(value)) =
                    (state.upgrade(), owner.upgrade(), ticket, value)
                {
                    if let Ok(value) = serde_json::from_str::<Value>(&value.to_str()) {
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
            },
        );
    }

    // The per-branch PR calls the checkpoint lifecycle.
    #[cfg_attr(not(feature = "gui-smoke"), allow(dead_code))]
    pub(super) fn checkpoint(&self) -> Result<Checkpoint> {
        let state = &self.state;
        let owner = state.pages.upgrade().context("Page owner was closed.")?;
        let list = self
            .native
            .back_forward_list()
            .context("No native history.")?;
        let (items, current) = ordered_entries(&list)?;
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
                        uri: web_url(&item.uri().context("History entry has no URL.")?)?,
                        stored_url: owner.browser.borrow().visit(entry.visit)?.url,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        };
        let bytes = self
            .native
            .session_state()
            .and_then(|s| s.serialize())
            .context("No session state.")?;
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
        context: &Rc<super::native::PageContext>,
        saved: &Checkpoint,
        visit: VisitId,
    ) -> Result<(Self, Revival)> {
        let page = Self::build(container, pages.clone(), context, InitialPage::Session)?;
        let session = webkit2gtk::WebViewSessionState::new(&saved.bytes);
        page.native.restore_session_state(&session);
        let binding = (|| -> Result<Vec<HistoryEntry>> {
            let shows_target = |e: &&CheckpointEntry| e.visit == visit && e.uri == e.stored_url;
            let _target = saved
                .entries
                .get(saved.current)
                .filter(shows_target)
                .or_else(|| saved.entries.iter().rev().find(shows_target))
                .context("Visit has no traversable entry in checkpoint.")?;

            let (items, current) =
                ordered_entries(&page.native.back_forward_list().context("No history.")?)?;
            let urls = items
                .iter()
                .map(|i| i.uri().and_then(|u| web_url(&u).ok()))
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
                *page.state.history_cursor.borrow_mut() = page
                    .native
                    .back_forward_list()
                    .and_then(|l| l.current_item());
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
        self.native.grab_focus();
        self.state.owner().place_view(&self.native);
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
        self.native.load_html(HOME, None);
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
        let was_current = target.is_some()
            && target
                == self
                    .native
                    .back_forward_list()
                    .and_then(|l| l.current_item());
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
        let target = capture
            .citation_url
            .clone()
            .unwrap_or_else(|| capture.document_url.clone());
        let source = SourceContext {
            page_url: capture.document_url.clone(),
            citation_url: capture.citation_url.clone(),
            link_text: capture.title.clone(),
            excerpt: capture.text.chars().take(1000).collect(),
        };
        let url = self.state.owner().browser.borrow_mut().follow_link(
            &mut self.state.navigation.borrow_mut(),
            capture.visit,
            target,
            source,
        )?;
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
            self.native.evaluate_javascript(
                &format!("globalThis.__canopyDocumentEpoch = {}", document.epoch),
                Some(WORLD),
                None,
                None::<&gio::Cancellable>,
                |_| {},
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
            .is_some_and(|p| p.native == self.native);
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
                    if let Some(entry) = returning.and_then(|visit| state.entry(visit, &url)) {
                        *state.traversal.borrow_mut() = Some((requested, entry.clone()));
                        state.native.go_to_back_forward_list_item(&entry);
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
                if self
                    .owner()
                    .browser
                    .borrow()
                    .capture_enabled()
                    .unwrap_or(false)
                {
                    self.capture(CaptureKind::Page, false);
                }
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
        self.native.evaluate_javascript(
            script,
            Some(WORLD),
            None,
            None::<&gio::Cancellable>,
            move |result| {
                let Some(state) = Pages::enter_native(&weak) else {
                    return;
                };
                let value = result
                    .ok()
                    .and_then(|value| serde_json::from_str(value.to_str().as_str()).ok());
                callback(state.state.clone(), value);
            },
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
            if let Some(mut action) = decision
                .downcast_ref::<NavigationPolicyDecision>()
                .and_then(|n| n.navigation_action())
            {
                // Preserve the form-return guard independently of switch intent.
                if action.request().and_then(|r| r.uri()) == self.native.uri() {
                    self.answer_form_return(false);
                }
                // NavigationAction has no main-frame flag. A missing target name is
                // the closest signal available (unnamed subframes are ambiguous).
                let main_action = action.frame_name().is_none_or(|name| name == "_top");
                if main_action {
                    let modifiers = gdk::ModifierType::from_bits_truncate(action.modifiers());
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
                    let restored = self.history.borrow().iter().any(|e| {
                        e.needs_get && Some(&e.item) == self.history_cursor.borrow().as_ref()
                    });
                    if main && !restored {
                        let owner = self.owner();
                        let candidate = owner.branches.candidate.borrow().clone();
                        if let Some(page) = candidate.filter(|p| {
                            p.native == self.native && owner.branches.candidate_live.get().is_some()
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
                            .is_some_and(|p| p.native == self.native);
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
            let Some(response) = decision.downcast_ref::<ResponsePolicyDecision>() else {
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
                    .and_then(|r| r.http_headers())
                    .and_then(|headers| headers.content_disposition())
                    .is_some_and(|(disposition, _)| disposition.eq_ignore_ascii_case("attachment"));
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
        let Some(navigation) = decision.downcast_ref::<NavigationPolicyDecision>() else {
            return false;
        };
        let Some(action) = navigation.navigation_action() else {
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
                    && Some(&entry.item) == self.history_cursor.borrow().as_ref()
                    && entry.document != self.history_document.get()
                    && entry.item.uri().as_deref() == Some(uri.as_str())
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
                self.native.load_uri(url.as_str());
            }
            return true;
        }
        if action.navigation_type() != NavigationType::Reload {
            self.reload_candidate.borrow_mut().take();
        }
        let modifiers = gdk::ModifierType::from_bits_truncate(action.modifiers());
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
                    self.emit(PageEvent::Changed);
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
                        && self.history_cursor.borrow().as_ref() == Some(item)
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
        let current = self.history_cursor.borrow().clone()?;
        self.history
            .borrow()
            .iter()
            .find(|entry| entry.item == current)
            .map(|entry| entry.visit)
    }

    fn current_entry_document(&self) -> Option<u64> {
        let current = self.history_cursor.borrow().clone()?;
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
            if is_selected(&self.pages, &self.native) {
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
        let Some(list) = self.native.back_forward_list() else {
            return;
        };
        let Some(current) = self.history_cursor.borrow().clone() else {
            return;
        };
        let live = live_entries(&list);
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
    fn entry(&self, visit: VisitId, url: &Url) -> Option<BackForwardListItem> {
        let list = self.native.back_forward_list()?;
        let current = list.current_item();
        let live = live_entries(&list);
        let history = self.history.borrow();
        let preferred = history.iter().filter(|entry| {
            self.history_document.get() == 0 && Some(&entry.item) == current.as_ref()
        });
        preferred
            .chain(history.iter().rev())
            .find(|entry| {
                entry.visit == visit
                    && (!entry.needs_get || self.history_document.get() == 0)
                    && entry.document != self.history_document.get()
                    && (self.history_document.get() == 0 || Some(&entry.item) != current.as_ref())
                    && live.contains(&entry.item)
                    && entry.item.uri().as_deref() == Some(url.as_str())
            })
            .map(|entry| entry.item.clone())
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
        if !self.native.is_mapped() {
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

fn is_selected(weak: &Weak<Pages>, view: &webkit2gtk::WebView) -> bool {
    weak.upgrade().is_some_and(|pages| {
        pages
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|p| p.native == *view && view.is_sensitive())
    })
}

fn ordered_entries(
    list: &webkit2gtk::BackForwardList,
) -> Result<(Vec<BackForwardListItem>, usize)> {
    let back = list.back_list().len();
    let forward = list.forward_list().len();
    let entries = (-(back as i32)..=forward as i32)
        .map(|i| list.nth_item(i).context("Missing history position."))
        .collect::<Result<Vec<_>>>()?;
    anyhow::ensure!(
        entries.len() == list.length() as usize,
        "Native history length differs."
    );
    anyhow::ensure!(
        entries.get(back) == list.current_item().as_ref(),
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

fn live_entries(list: &webkit2gtk::BackForwardList) -> Vec<BackForwardListItem> {
    let mut entries = list.back_list();
    entries.extend(list.forward_list());
    entries.extend(list.current_item());
    entries
}

fn same_resource(left: &Url, right: &Url) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.set_fragment(None);
    right.set_fragment(None);
    left == right
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
<style>body{margin:0;background:#f6f5f0;color:#29362e;font:18px system-ui;display:grid;place-content:center;height:100vh}main{max-width:520px;padding:40px}.mark{font-size:56px;color:#53795b}h1{font-weight:550;font-size:36px;letter-spacing:-1px}p{line-height:1.7;color:#647168}kbd{font:14px system-ui;border:1px solid #ccd3c8;border-radius:6px;padding:4px 9px;background:white}.small{font-size:14px;margin-top:36px}</style>
<main><div class="mark">⌘</div><h1>A trail starts here.</h1><p>Open a page, follow a link, then come back to explore another direction. Your branches stay within reach.</p><p class="small"><kbd>Ctrl</kbd> + <kbd>K</kbd> to search or go to an address.</p></main></html>"#;

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
