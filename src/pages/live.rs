use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use anyhow::{bail, Context, Result};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use url::Url;
use webkit2gtk::{
    BackForwardListExt, BackForwardListItem, BackForwardListItemExt, LoadEvent,
    NavigationPolicyDecision, NavigationPolicyDecisionExt, NavigationType, NetworkError,
    PolicyDecision, PolicyDecisionExt, PolicyDecisionType, PolicyError, ResponsePolicyDecision,
    ResponsePolicyDecisionExt, URIRequestExt, UserContentInjectedFrames, UserContentManagerExt,
    UserScript, UserScriptInjectionTime, WebViewExt,
};
#[path = "mouse.rs"]
mod mouse;

use crate::{
    address::web_url,
    browser::{
        Capture, CaptureKind, CapturePayload, Document, LoadTicket, Navigation, NavigationTarget,
        SourceContext, Visibility, VisitId,
    },
};

const WORLD: &str = "canopy-reading-memory";
const HELPER: &str = include_str!("../capture.js");
/// Script message the helper posts, debounced, when the page scrolls.
const SCROLLED: &str = "canopyScrolled";
use super::{PageEvent, Pages};

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
            Self::Idle => None,
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
    issue: RefCell<Issue>,
    load_generation: Cell<u64>,
    foreground_generation: Cell<u64>,
    same_document_request: RefCell<Option<Url>>,
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
    restoration_viewport: Cell<Option<(i32, i32)>>,
    /// The address of the main frame's last response that has not become a page, and the
    /// page its file belongs to if it becomes a download instead. WebKit may report that
    /// load as failed before it reports the download, and the navigation is forgotten then.
    response_page: RefCell<Option<(Url, Option<VisitId>)>>,
    reload_candidate: RefCell<Option<Url>>,
    main_frame_post: Cell<Option<bool>>,
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
    pub(super) fn pending_ticket(&self) -> Option<LoadTicket> {
        self.state.navigation.borrow().pending_ticket()
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
            mouse::setup(&native)
                .into_iter()
                .map(|id| (native.clone().upcast(), id)),
        );
        native.set_size_request(200, 200);
        container.put(&native, 0, 0);
        if matches!(initial, InitialPage::Home) {
            native.load_html(HOME, None);
        }
        native.show_all();
        native.grab_focus();
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
            pages,
            navigation: RefCell::new(Navigation::default()),
            issue: RefCell::new(Issue::Idle),
            load_generation: Cell::new(0),
            foreground_generation: Cell::new(0),
            same_document_request: RefCell::new(None),
            issued: RefCell::new(None),
            history: RefCell::new(Vec::new()),
            history_document: Cell::new(0),
            back_forward_commit: Cell::new(false),
            next_history_document: Cell::new(0),
            history_cursor: RefCell::new(None),
            traversal: RefCell::new(None),
            pending_scroll: Cell::new(None),
            restoration_viewport: Cell::new(None),
            response_page: RefCell::new(None),
            reload_candidate: RefCell::new(None),
            main_frame_post: Cell::new(None),
            #[cfg(feature = "gui-smoke")]
            revival_refusals: Cell::new(0),
        });
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
                        }
                        LoadEvent::Finished => {
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
                            state.emit(PageEvent::Loading(false));
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
                    if ticket.is_none() || ticket != state.navigation.borrow().pending_ticket() {
                        return true;
                    }
                    let cancelled = error.matches(NetworkError::Cancelled);
                    if let Some(ticket) = ticket {
                        state.navigation.borrow_mut().failed(ticket);
                    }
                    // A response turned into a download ends the page load this way; the page
                    // the user was reading stays.
                    if error.matches(PolicyError::FrameLoadInterruptedByPolicyChange) {
                        state.emit(PageEvent::Loading(false));
                    } else if !cancelled {
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
                        if let Some(document) = document {
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
                    if weak
                        .upgrade()
                        .is_some_and(|state| state.lifecycle.get() != Lifecycle::Live)
                    {
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
                    state
                        .main_frame_post
                        .set(Some(request.http_method().as_deref() == Some("POST")));
                    let weak = Rc::downgrade(&state);
                    // The typed binding treats the absent redirect response as non-null.
                    resource.connect_local("sent-request", false, move |values| {
                        let resource = values[0].get::<webkit2gtk::WebResource>().unwrap();
                        let request = values[1].get::<webkit2gtk::URIRequest>().unwrap();
                        if let Some(state) = Pages::enter_native(&weak) {
                            if state.native.main_resource().as_ref() == Some(&resource) {
                                state
                                    .main_frame_post
                                    .set(Some(request.http_method().as_deref() == Some("POST")));
                            }
                        }
                        None
                    });
                }),
            ));
        }
        let weak = Rc::downgrade(&state);
        let favicon = context.register_view(&native, move |source| {
            Pages::enter_native(&weak).and_then(|state| state.download_page(source))
        });
        signals.push((native.clone().upcast(), favicon));
        Ok(Self {
            native,
            state,
            signals: RefCell::new(signals),
        })
    }

    #[cfg(feature = "gui-smoke")]
    pub(super) fn revival_refusals(&self) -> usize {
        self.state.revival_refusals.get()
    }

    #[cfg(feature = "gui-smoke")]
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
                        if checkpoint_document_matches(&value, &ticket) {
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
            let target = saved
                .entries
                .get(saved.current)
                .filter(shows_target)
                .or_else(|| saved.entries.iter().rev().find(shows_target))
                .context("Visit has no traversable entry in checkpoint.")?;
            anyhow::ensure!(!target.post, "POST target requires GET.");
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

    pub(super) fn follow_container_bounds(&self, container: &gtk::Fixed) {
        let weak = Rc::downgrade(&self.state);
        let signal = container.connect_size_allocate(move |_, allocation| {
            if let Some(state) = weak.upgrade() {
                let width = allocation.width().max(1);
                let height = (allocation.height() - super::CHROME_HEIGHT as i32).max(1);
                state.restoration_viewport.set(Some((width, height)));
                state.native.size_allocate(&gtk::Allocation::new(
                    0,
                    super::CHROME_HEIGHT as i32,
                    width,
                    height,
                ));
            }
        });
        self.signals
            .borrow_mut()
            .push((container.clone().upcast(), signal));
        self.state.restoration_viewport.set(Some((
            container.allocated_width(),
            container.allocated_height() - super::CHROME_HEIGHT as i32,
        )));
        self.set_bounds(
            0.,
            super::CHROME_HEIGHT,
            container.allocated_width() as f64,
            container.allocated_height() as f64 - super::CHROME_HEIGHT,
        );
    }

    /// Places the page in its container, in logical pixels.
    pub fn set_bounds(&self, x: f64, y: f64, width: f64, height: f64) {
        if self.state.restoration_viewport.get().is_some() {
            self.state.restoration_viewport.set(Some((
                width.max(1.).round() as i32,
                height.max(1.).round() as i32,
            )));
        }
        self.native.size_allocate(&gtk::Allocation::new(
            x.round() as i32,
            y.round() as i32,
            width.max(1.).round() as i32,
            height.max(1.).round() as i32,
        ));
    }

    pub fn focus(&self) {
        self.native.grab_focus();
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

    pub fn back(&self) -> Result<()> {
        let request = self
            .state
            .owner()
            .browser
            .borrow_mut()
            .back(&mut self.state.navigation.borrow_mut())?;
        if let Some(url) = request {
            self.state.load(url);
        }
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

    pub fn open_saved(&self, capture: &Capture) -> Result<()> {
        let target = capture
            .citation_url
            .clone()
            .unwrap_or_else(|| capture.document_url.clone());
        let visit = self.state.owner().browser.borrow().visit(capture.visit)?;
        let url = if target == visit.url {
            self.state
                .owner()
                .browser
                .borrow_mut()
                .restore(&mut self.state.navigation.borrow_mut(), visit.id)?
        } else {
            let source = SourceContext {
                page_url: capture.document_url.clone(),
                citation_url: capture.citation_url.clone(),
                link_text: capture.title.clone(),
                excerpt: capture.text.chars().take(1000).collect(),
            };
            self.state.owner().browser.borrow_mut().follow_link(
                &mut self.state.navigation.borrow_mut(),
                visit.id,
                target,
                source,
            )?
        };
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
            | Issue::Retiring { ticket, .. } => Some(ticket),
        }
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
                let ticket = navigation.native_request(url.clone());
                *self.issue.borrow_mut() = Issue::Loading(ticket);
                ticket
            }
        };
        if matches!(*self.issue.borrow(), Issue::Issued { .. }) {
            *self.issue.borrow_mut() = Issue::Loading(ticket);
        }
        navigation.started(ticket);
    }

    fn accept_issue(&self) {
        if let Issue::Issued { decided, .. } = &mut *self.issue.borrow_mut() {
            *decided = true;
        }
    }

    fn loading_ticket(&self) -> Option<LoadTicket> {
        match *self.issue.borrow() {
            Issue::Loading(ticket) | Issue::Retiring { ticket, .. } => Some(ticket),
            _ => None,
        }
    }

    fn commit_attempt(&self, url: Url) -> Result<bool> {
        let Some(ticket) = self.loading_ticket() else {
            return Ok(false);
        };
        let committed = self.owner().browser.borrow_mut().commit_load(
            &mut self.navigation.borrow_mut(),
            ticket,
            url,
            Visibility::Selected,
        )?;
        if committed {
            let visit = self
                .navigation
                .borrow()
                .current
                .as_ref()
                .expect("committed document")
                .visit;
            self.owner().committed(ticket, visit);
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
            self.owner().receive(event);
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

    fn load(self: &Rc<Self>, url: Url) {
        // Every caller registers its intent first; an address with no attempt has no visit to
        // commit to.
        let Some(requested) = self.navigation.borrow().pending_ticket() else {
            return;
        };
        {
            let mut issue = self.issue.borrow_mut();
            if let Issue::Retiring { next, .. } = &mut *issue {
                *next = (requested, url);
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
        let committed = self.commit_attempt(target.clone());
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

    fn decide(self: &Rc<Self>, decision: &PolicyDecision, kind: PolicyDecisionType) -> bool {
        if kind == PolicyDecisionType::Response {
            let Some(response) = decision.downcast_ref::<ResponsePolicyDecision>() else {
                return false;
            };
            let main = response.is_main_frame_main_resource();
            if main {
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
                    (url, page.ok().flatten())
                });
                self.response_page.replace(noted);
            }
            if response.is_mime_type_supported() {
                return false;
            }
            if main {
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
                    self.emit(PageEvent::Changed);
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
                                source,
                            );
                            match result {
                                Ok(true) => state.emit(PageEvent::Changed),
                                Ok(false) => {}
                                Err(error) => state.emit(PageEvent::Error(error.to_string())),
                            }
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
        let Some(tree) = forest.current().and_then(|id| forest.tree_of(id)) else {
            return false;
        };
        forest.tree_of(visit) == Some(tree) && forest.open().iter().any(|open| open.id == tree)
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
        self.navigation.borrow_mut().same_document(url);
        if let Some(document) = self.ticket() {
            self.bind_document();
            let result = self
                .owner()
                .browser
                .borrow_mut()
                .select_document(&self.navigation.borrow(), &document);
            self.report(result);
            self.emit(PageEvent::Changed);
        }
    }

    /// Records which visit WebKit's current history entry shows.
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
        let method = self.main_frame_post.take();
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
                    && !entry.needs_get
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
        // A cancelled earlier load can finish first; keep the target for the visit it names.
        let Some((visit, y)) = self.pending_scroll.get() else {
            return;
        };
        if self.ticket().is_some_and(|ticket| ticket.visit == visit) {
            self.pending_scroll.set(None);
            let viewport = self
                .restoration_viewport
                .get()
                .map(|(width, height)| format!("[{width},{height}]"))
                .unwrap_or_else(|| "null".into());
            let script = format!(
                "JSON.stringify(globalThis.__canopyMemory?.restoreScroll({y}, {viewport}) ?? null)"
            );
            self.evaluate(&script, |_, _| {});
        }
    }

    /// The page a download from `source` that starts now belongs to.
    fn download_page(&self, source: Option<&Url>) -> Option<VisitId> {
        let noted = self
            .response_page
            .borrow_mut()
            .take_if(|(url, _)| Some(&*url) == source);
        match noted {
            Some((_, page)) => page,
            None => {
                let page = self
                    .owner()
                    .browser
                    .borrow()
                    .download_page(&self.navigation.borrow(), source);
                page.ok().flatten()
            }
        }
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
