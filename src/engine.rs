use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{bail, Context, Result};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use url::Url;
use webkit2gtk::{
    BackForwardListExt, BackForwardListItem, BackForwardListItemExt, DownloadExt, LoadEvent,
    NavigationPolicyDecision, NavigationPolicyDecisionExt, NavigationType, NetworkError,
    PolicyDecision, PolicyDecisionExt, PolicyDecisionType, PolicyError, ResponsePolicyDecision,
    ResponsePolicyDecisionExt, URIRequestExt, UserContentInjectedFrames, UserContentManagerExt,
    UserScript, UserScriptInjectionTime, WebContextExt, WebViewExt,
};
mod mouse;

use crate::{
    address::web_url,
    browser::{
        Browser, Capture, CaptureKind, CapturePayload, Document, LoadTicket, Navigation,
        NavigationTarget, SourceContext, Visibility, VisitId,
    },
};

const WORLD: &str = "canopy-reading-memory";
const HELPER: &str = include_str!("capture.js");
/// Script message the helper posts, debounced, when the page scrolls.
const SCROLLED: &str = "canopyScrolled";
type Observer = RefCell<Option<Box<dyn Fn(EngineEvent)>>>;

pub enum EngineEvent {
    Changed,
    Loading(bool),
    Error(String),
    Remembered(i64),
    Download(DownloadEvent),
}

pub enum DownloadEvent {
    /// Carries the file name the download asks for. The saved name can differ.
    Started(String),
    Saved(PathBuf),
    Failed {
        name: Option<String>,
        reason: String,
    },
}

/// One download's progress through WebKit's signals, which end with `finished` either way.
enum Transfer {
    Deciding,
    Saving {
        staging: crate::download::Staging,
        name: String,
    },
    Failed {
        name: Option<String>,
        reason: String,
    },
    Over,
}

struct HistoryEntry {
    item: BackForwardListItem,
    visit: VisitId,
    document: u64,
}

enum Issue {
    Idle,
    Issued(LoadTicket),
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
            Self::Issued(ticket) => {
                *self = Self::Issued(ticket);
                None
            }
            Self::Idle => None,
        }
    }
}

#[cfg(test)]
mod attempt_tests {
    use super::*;

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

struct NativeState {
    native: webkit2gtk::WebView,
    browser: Rc<RefCell<Browser>>,
    observer: Observer,
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
    /// The address of the main frame's last response that has not become a page, and the
    /// page its file belongs to if it becomes a download instead. WebKit may report that
    /// load as failed before it reports the download, and the navigation is forgotten then.
    response_page: RefCell<Option<(Url, Option<VisitId>)>>,
    reload_candidate: RefCell<Option<Url>>,
}

pub struct Engine {
    pub native: webkit2gtk::WebView,
    state: Rc<NativeState>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        // GTK's parent retains the widget after the engine releases its handles.
        unsafe { self.native.destroy() };
    }
}

impl Engine {
    pub fn new(
        container: &gtk::Fixed,
        browser: Rc<RefCell<Browser>>,
        profile: &Path,
    ) -> Result<Self> {
        std::fs::create_dir_all(profile)?;
        use webkit2gtk::{
            CookieManagerExt, CookiePersistentStorage, InputMethodContextExt, SettingsExt,
            WebsiteDataManager, WebsiteDataManagerExt,
        };
        let data = WebsiteDataManager::builder()
            .base_data_directory(profile.to_string_lossy())
            .base_cache_directory(profile.to_string_lossy())
            .build();
        data.cookie_manager()
            .context("The cookie manager is unavailable.")?
            .set_persistent_storage(
                &profile.join("cookies").to_string_lossy(),
                CookiePersistentStorage::Text,
            );
        let context = webkit2gtk::WebContext::builder()
            .website_data_manager(&data)
            .build();
        context.set_automation_allowed(false);
        context.set_use_system_appearance_for_scrollbars(false);
        // WebKitGTK 2.54 requests icons only through the context's database.
        #[allow(deprecated)]
        context.set_favicon_database_directory(Some(&profile.join("favicons").to_string_lossy()));
        let native = webkit2gtk::WebView::builder()
            .web_context(&context)
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
        native.connect_close(|view| unsafe { view.destroy() });
        mouse::setup(&native);
        native.set_size_request(200, 200);
        container.put(&native, 0, 0);
        native.load_html(HOME, None);
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
            browser,
            observer: RefCell::new(None),
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
            response_page: RefCell::new(None),
            reload_candidate: RefCell::new(None),
        });
        manager.register_script_message_handler_in_world(SCROLLED, WORLD);
        {
            let weak = Rc::downgrade(&state);
            manager.connect_script_message_received(Some(SCROLLED), move |_, message| {
                if let (Some(state), Some(value)) = (weak.upgrade(), message.js_value()) {
                    state.scrolled(&value.to_str());
                }
            });
        }
        {
            let weak = Rc::downgrade(&state);
            native.connect_load_changed(move |native, event| {
                let Some(state) = weak.upgrade() else { return };
                let url = native.uri().and_then(|uri| web_url(&uri).ok());
                match event {
                    LoadEvent::Started => {
                        let reload = state.reload_candidate.borrow_mut().take();
                        if reload.is_some() && state.navigation.borrow().pending_ticket().is_none()
                        {
                            let result = state
                                .browser
                                .borrow_mut()
                                .reload(&mut state.navigation.borrow_mut());
                            if let Some(ticket) = state.navigation.borrow().pending_ticket() {
                                *state.issue.borrow_mut() = Issue::Issued(ticket);
                            }
                            state.report(result.map(|_| ()));
                        }

                        state.same_document_request.borrow_mut().take();
                        state.traversal.borrow_mut().take();
                        let issued = state.issued.borrow_mut().take();
                        if let Some(url) = issued.or(url) {
                            state.start_attempt(url);
                            state.emit(EngineEvent::Loading(true));
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
                                    state.emit(EngineEvent::Changed);
                                }
                                Ok(false) => {}
                                Err(error) => state.emit(EngineEvent::Error(error.to_string())),
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
                        state.emit(EngineEvent::Loading(false));
                        let enabled = state.browser.borrow().capture_enabled().unwrap_or(false);
                        if enabled {
                            state.capture(CaptureKind::Page, false);
                            if let Some(ticket) = state.ticket() {
                                for seconds in [1, 3] {
                                    let weak = Rc::downgrade(&state);
                                    let ticket = ticket.clone();
                                    glib::timeout_add_local_once(
                                        std::time::Duration::from_secs(seconds),
                                        move || {
                                            if let Some(state) = weak.upgrade() {
                                                if state.valid(&ticket)
                                                    && state
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
            });
        }
        {
            let weak = Rc::downgrade(&state);
            native.connect_load_failed(move |_, _, _uri, error| {
                let Some(state) = weak.upgrade() else {
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
                    state.emit(EngineEvent::Loading(false));
                } else if !cancelled {
                    state.emit(EngineEvent::Error(format!(
                        "Could not load this page. {}",
                        error.message()
                    )));
                    state.emit(EngineEvent::Loading(false));
                }
                true
            });
        }
        {
            let weak = Rc::downgrade(&state);
            native.connect_load_failed_with_tls_errors(move |_, _uri, _, _| {
                if let Some(state) = weak.upgrade() {
                    let ticket = state.active_ticket();
                    if ticket.is_none() || ticket != state.navigation.borrow().pending_ticket() {
                        return true;
                    }
                    if let Some(ticket) = ticket {
                        state.navigation.borrow_mut().failed(ticket);
                    }
                    state.emit(EngineEvent::Error(
                        "HTTPS certificate rejected. The connection was not opened.".into(),
                    ));
                    state.emit(EngineEvent::Loading(false));
                }
                true
            });
        }
        {
            let weak = Rc::downgrade(&state);
            native.connect_title_notify(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.update_title();
                }
            });
        }
        if let Some(list) = native.back_forward_list() {
            let weak = Rc::downgrade(&state);
            list.connect_local("changed", false, move |values| {
                let state = weak.upgrade()?;
                let list: webkit2gtk::BackForwardList = values[0].get().unwrap();
                let added: Option<BackForwardListItem> = values[1].get().unwrap();
                let current = list.current_item();
                *state.history_cursor.borrow_mut() = current.clone();
                let removed: glib::Pointer = values[2].get().unwrap_or(std::ptr::null_mut());
                // WebKit keeps removed objects readable while our bindings retain them.
                unsafe {
                    use glib::translate::FromGlibPtrNone;
                    let mut node = removed as *mut glib::ffi::GList;
                    while !node.is_null() {
                        let item = BackForwardListItem::from_glib_none(
                            (*node).data as *mut webkit2gtk::ffi::WebKitBackForwardListItem,
                        );
                        state
                            .history
                            .borrow_mut()
                            .retain(|entry| entry.item != item);
                        node = (*node).next;
                    }
                }
                if added.is_some() && state.navigation.borrow().pending_ticket().is_none() {
                    let document = state.navigation.borrow().current.clone();
                    if let Some(document) = document {
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
                    if let Some(state) = weak.upgrade().filter(|state| {
                        state.navigation.borrow().pending_ticket() == attempt
                            && *state.history_cursor.borrow() == current
                    }) {
                        state.history_changed(added.is_some());
                    }
                });
                None
            });
        }
        {
            let weak = Rc::downgrade(&state);
            native.connect_uri_notify(move |_| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let attempt = state.navigation.borrow().pending_ticket();
                let weak = Rc::downgrade(&state);
                glib::idle_add_local_once(move || {
                    if let Some(state) = weak
                        .upgrade()
                        .filter(|state| state.navigation.borrow().pending_ticket() == attempt)
                    {
                        state.history_changed(false);
                    }
                });
            });
        }
        {
            let weak = Rc::downgrade(&state);
            native.connect_is_loading_notify(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.complete_same_document();
                }
            });
        }
        {
            let weak = Rc::downgrade(&state);
            native.connect_decide_policy(move |_, decision, kind| {
                weak.upgrade()
                    .is_some_and(|state| state.decide(decision, kind))
            });
        }
        let weak = Rc::downgrade(&state);
        context.connect_download_started(move |_, download| {
            save(download, weak.clone());
        });
        Ok(Self { native, state })
    }

    /// Places the page in its container, in logical pixels.
    pub fn set_bounds(&self, x: f64, y: f64, width: f64, height: f64) {
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

    pub fn observe(&self, callback: impl Fn(EngineEvent) + 'static) {
        *self.state.observer.borrow_mut() = Some(Box::new(callback));
    }

    pub fn navigate(&self, url: Url, target: NavigationTarget) -> Result<()> {
        let url = self.state.browser.borrow_mut().navigate(
            &mut self.state.navigation.borrow_mut(),
            url,
            target,
        )?;
        self.state.load(url);
        Ok(())
    }

    pub fn restore(&self, id: VisitId) -> Result<()> {
        let url = self
            .state
            .browser
            .borrow_mut()
            .restore(&mut self.state.navigation.borrow_mut(), id)?;
        self.state.load(url);
        Ok(())
    }

    pub fn back(&self) -> Result<()> {
        let request = self
            .state
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
        let visit = self.state.browser.borrow().visit(capture.visit)?;
        let url = if target == visit.url {
            self.state
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
            self.state.browser.borrow_mut().follow_link(
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

    pub fn capture_enabled(&self) -> Result<bool> {
        self.state.browser.borrow().capture_enabled()
    }

    pub fn set_capture_enabled(&self, enabled: bool) -> Result<()> {
        self.state
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
            Issue::Issued(ticket) | Issue::Loading(ticket) | Issue::Retiring { ticket, .. } => {
                Some(ticket)
            }
        }
    }

    fn start_attempt(&self, url: Url) {
        let mut navigation = self.navigation.borrow_mut();
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
        if matches!(*self.issue.borrow(), Issue::Issued(_)) {
            *self.issue.borrow_mut() = Issue::Loading(ticket);
        }
        navigation.started(ticket);
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
        self.browser.borrow_mut().commit_load(
            &mut self.navigation.borrow_mut(),
            ticket,
            url,
            Visibility::Selected,
        )
    }

    fn emit(&self, event: EngineEvent) {
        if let Some(callback) = self.observer.borrow().as_ref() {
            callback(event);
        }
    }

    fn report(&self, result: Result<()>) {
        if let Err(error) = result {
            self.emit(EngineEvent::Error(error.to_string()));
        }
    }

    fn bind_document(&self) {
        if let Some(document) = self.navigation.borrow().current.as_ref() {
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
        {
            let mut issue = self.issue.borrow_mut();
            if let Issue::Retiring { next, .. } = &mut *issue {
                *next = (self.navigation.borrow().pending_ticket().unwrap(), url);
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
        let requested = self.navigation.borrow().pending_ticket();
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                if state.load_generation.get() == generation
                    && state.navigation.borrow().pending_ticket() == requested
                {
                    {
                        let mut issue = state.issue.borrow_mut();
                        if let Issue::Retiring { next, .. } = &mut *issue {
                            *next = (requested.unwrap(), url);
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
                            next: (requested.unwrap(), url),
                        };
                        state.native.stop_loading();
                        return;
                    }
                    if let Some(ticket) = requested {
                        *state.issue.borrow_mut() = Issue::Issued(ticket);
                    }
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
                        .browser
                        .borrow()
                        .requested_restore(&state.navigation.borrow());
                    if let Some(entry) = returning.and_then(|visit| state.entry(visit, &url)) {
                        if let Some(ticket) = requested {
                            *state.traversal.borrow_mut() = Some((ticket, entry.clone()));
                        }
                        state.native.go_to_back_forward_list_item(&entry);
                    } else {
                        if let Some(visit) = returning.filter(|_| !same_document) {
                            let saved = state.browser.borrow().scroll(visit).ok().flatten();
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
        self.start_attempt(target.clone());
        let committed = self.commit_attempt(target.clone());
        match committed {
            Ok(true) => {
                *self.issue.borrow_mut() = Issue::Idle;
                self.bind_document();
                self.update_title();
                self.emit(EngineEvent::Changed);
                if self.browser.borrow().capture_enabled().unwrap_or(false) {
                    self.capture(CaptureKind::Page, false);
                }
            }
            Ok(false) => {}
            Err(error) => self.emit(EngineEvent::Error(error.to_string())),
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
                let Some(state) = weak.upgrade() else { return };
                let value = result
                    .ok()
                    .and_then(|value| serde_json::from_str(value.to_str().as_str()).ok());
                callback(state, value);
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
            self.emit(EngineEvent::Error(
                "This kind of link is not supported.".into(),
            ));
            return true;
        };
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
            let result = self
                .browser
                .borrow_mut()
                .queue_link(ticket.visit, url.clone(), source);
            match result {
                Ok(child) => {
                    self.emit(EngineEvent::Changed);
                    self.context(&url, move |state, context| {
                        if !state.valid(&ticket) {
                            return;
                        }
                        if let Some(source) =
                            context.filter(|source| source.page_url == ticket.page_url)
                        {
                            let result = state.browser.borrow_mut().source_for_document(
                                &state.navigation.borrow(),
                                &ticket,
                                child,
                                source,
                            );
                            match result {
                                Ok(true) => state.emit(EngineEvent::Changed),
                                Ok(false) => {}
                                Err(error) => state.emit(EngineEvent::Error(error.to_string())),
                            }
                        }
                    });
                }
                Err(error) => self.emit(EngineEvent::Error(error.to_string())),
            }
            return true;
        }
        if action.navigation_type() != NavigationType::BackForward {
            self.back_forward_commit.set(false);
        }
        if action.navigation_type() == NavigationType::Reload
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
                return false;
            }
            if self.current_entry_document() == Some(self.history_document.get()) {
                return false;
            }
            if let Some(visit) = self.current_entry_visit() {
                let result = self
                    .browser
                    .borrow_mut()
                    .restore(&mut self.navigation.borrow_mut(), visit);
                if let Some(ticket) = self.navigation.borrow().pending_ticket() {
                    *self.issue.borrow_mut() = Issue::Issued(ticket);
                }
                self.report(result.map(|_| ()));
            } else {
                decision.ignore();
                let result = self
                    .browser
                    .borrow_mut()
                    .back(&mut self.navigation.borrow_mut());
                match result {
                    Ok(Some(url)) => self.load(url),
                    Ok(None) => {}
                    Err(error) => self.emit(EngineEvent::Error(error.to_string())),
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
                    let request = state.browser.borrow_mut().follow_link(
                        &mut state.navigation.borrow_mut(),
                        ticket.visit,
                        url,
                        source,
                    );
                    match request {
                        Ok(url) => state.load(url),
                        Err(error) => state.emit(EngineEvent::Error(error.to_string())),
                    }
                } else {
                    decision.use_();
                }
            });
            return true;
        }
        false
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
            let different = self
                .navigation
                .borrow()
                .current
                .as_ref()
                .is_some_and(|d| d.visit != visit);
            if different {
                let Ok(stored) = self.browser.borrow().visit(visit) else {
                    return;
                };
                self.navigation.borrow_mut().publish(visit, stored.url);
            }
        }
        self.navigation.borrow_mut().same_document(url);
        if let Some(document) = self.ticket() {
            self.bind_document();
            let result = self
                .browser
                .borrow_mut()
                .select_document(&self.navigation.borrow(), &document);
            self.report(result);
            self.emit(EngineEvent::Changed);
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
        history.retain(|entry| entry.item != current && live.contains(&entry.item));
        history.push(HistoryEntry {
            item: current,
            visit,
            document: self.history_document.get(),
        });
    }

    /// A history entry other than the current one that still shows `visit` at `url`.
    fn entry(&self, visit: VisitId, url: &Url) -> Option<BackForwardListItem> {
        let list = self.native.back_forward_list()?;
        let current = list.current_item();
        let live = live_entries(&list);
        self.history
            .borrow()
            .iter()
            .rev()
            .find(|entry| {
                entry.visit == visit
                    && Some(&entry.item) != current.as_ref()
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
        let _ = self.browser.borrow_mut().scroll_for_document(
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
            let script =
                format!("JSON.stringify(globalThis.__canopyMemory?.restoreScroll({y}) ?? null)");
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
                    .browser
                    .borrow()
                    .download_page(&self.navigation.borrow(), source);
                page.ok().flatten()
            }
        }
    }

    /// Notes on `page` that the file at `path` was saved from it.
    fn remember_download(&self, page: VisitId, path: &Path, source: Option<&Url>) -> Result<()> {
        let size = std::fs::metadata(path)?.len();
        self.browser
            .borrow_mut()
            .record_download(page, path, size, source)?;
        Ok(())
    }

    fn update_title(&self) {
        if let (Some(document), Some(title)) = (self.ticket(), self.native.title()) {
            let result = self.browser.borrow_mut().title_for_document(
                &self.navigation.borrow(),
                &document,
                &title,
            );
            self.report(result);
            self.emit(EngineEvent::Changed);
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
            if !explicit && !state.browser.borrow().capture_enabled().unwrap_or(false) {
                return;
            }
            let Some(value) = value else { return };
            if value
                .get("blocked")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                if explicit {
                    state.emit(EngineEvent::Error(
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
                    state.emit(EngineEvent::Error(
                        "Select the text of the post or article to remember.".into(),
                    ));
                }
                return;
            }
            let result = state.browser.borrow_mut().capture_for_document(
                &state.navigation.borrow(),
                &ticket,
                payload,
            );
            match result {
                Ok(Some(id)) => {
                    state.emit(EngineEvent::Changed);
                    if explicit {
                        state.emit(EngineEvent::Remembered(id));
                    }
                }
                Ok(None) => {}
                Err(error) => state.emit(EngineEvent::Error(error.to_string())),
            }
        });
    }
}

/// Saves `download` in the user's download folder, notes it on the page it came from and
/// reports it. WebKit's default would trust the server's file name.
fn save(download: &webkit2gtk::Download, state: std::rc::Weak<NativeState>) {
    let transfer = Rc::new(RefCell::new(Transfer::Deciding));
    let source = download
        .request()
        .and_then(|request| request.uri())
        .and_then(|uri| web_url(&uri).ok());
    // Decided now: once the file has arrived, another page may be on screen.
    let page = state
        .upgrade()
        .and_then(|state| state.download_page(source.as_ref()));
    {
        let transfer = transfer.clone();
        let state = state.clone();
        let emit = move |event| {
            if let Some(state) = state.upgrade() {
                state.emit(EngineEvent::Download(event));
            }
        };
        download.connect_decide_destination(move |download, suggested| {
            let name = crate::download::file_name(suggested);
            match crate::download::Staging::new(&download_directory()) {
                Ok(staging) => {
                    download.set_allow_overwrite(false);
                    download.set_destination(&staging.file().to_string_lossy());
                    emit(DownloadEvent::Started(name.clone()));
                    transfer.replace(Transfer::Saving { staging, name });
                }
                Err(error) => {
                    transfer.replace(Transfer::Over);
                    download.cancel();
                    emit(DownloadEvent::Failed {
                        name: Some(name),
                        reason: error.to_string(),
                    });
                }
            }
            true
        });
    }
    {
        let transfer = transfer.clone();
        download.connect_failed(move |_, error| {
            let reason = error.message().to_owned();
            let next = match transfer.replace(Transfer::Over) {
                Transfer::Deciding => Transfer::Failed { name: None, reason },
                Transfer::Saving { name, .. } => Transfer::Failed {
                    name: Some(name),
                    reason,
                },
                done => done,
            };
            transfer.replace(next);
        });
    }
    download.connect_finished(move |download| {
        let event = match transfer.replace(Transfer::Over) {
            Transfer::Saving { name, .. } if truncated(download) => DownloadEvent::Failed {
                name: Some(name),
                reason: "The connection closed before the whole file arrived.".into(),
            },
            Transfer::Saving { staging, name } => match staging.publish(&name) {
                Ok(path) => DownloadEvent::Saved(path),
                Err(error) => DownloadEvent::Failed {
                    name: Some(name),
                    reason: error.to_string(),
                },
            },
            Transfer::Failed { name, reason } => DownloadEvent::Failed { name, reason },
            Transfer::Deciding | Transfer::Over => return,
        };
        let Some(state) = state.upgrade() else { return };
        if let (DownloadEvent::Saved(path), Some(page)) = (&event, page) {
            match state.remember_download(page, path, source.as_ref()) {
                Ok(()) => state.emit(EngineEvent::Changed),
                Err(error) => state.emit(EngineEvent::Error(format!(
                    "The file was saved, but could not be added to its page. {error}"
                ))),
            }
        }
        state.emit(EngineEvent::Download(event));
    });
}

/// WebKit reports a transfer cut short after its headers as finished. A compressed body's
/// length counts encoded bytes, so only an unencoded one can be checked.
fn truncated(download: &webkit2gtk::Download) -> bool {
    use webkit2gtk::URIResponseExt;
    let Some(response) = download.response() else {
        return false;
    };
    let encoded = response
        .http_headers()
        .and_then(|headers| headers.one("Content-Encoding"))
        .is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity"));
    !encoded && download.received_data_length() < response.content_length()
}

fn download_directory() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads)
        .unwrap_or_else(|| glib::home_dir().join("Downloads"))
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

#[derive(Clone, Copy, Debug)]
pub struct CookieApplyError;
impl std::fmt::Display for CookieApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Could not add imported cookies.")
    }
}
impl std::error::Error for CookieApplyError {}

impl Engine {
    pub fn apply_cookie_batches(
        &self,
        batches: Vec<crate::cookies::Batch>,
        done: impl FnOnce(std::result::Result<usize, CookieApplyError>) + 'static,
    ) {
        use webkit2gtk::WebsiteDataManagerExt;
        let manager = self
            .native
            .context()
            .and_then(|c| c.website_data_manager())
            .and_then(|d| d.cookie_manager());
        let Some(manager) = manager else {
            done(Err(CookieApplyError));
            return;
        };
        CookieDrain {
            manager,
            batches: batches.into(),
            current: None,
            count: 0,
            done: Box::new(done),
        }
        .advance();
    }
}

struct CookieDrain {
    manager: webkit2gtk::CookieManager,
    batches: std::collections::VecDeque<crate::cookies::Batch>,
    current: Option<(
        crate::cookies::Batch,
        std::vec::IntoIter<crate::cookies::Cookie>,
    )>,
    count: usize,
    done: Box<dyn FnOnce(std::result::Result<usize, CookieApplyError>)>,
}
impl CookieDrain {
    fn advance(mut self) {
        use webkit2gtk::CookieManagerExt;
        loop {
            if self.current.is_none() {
                let Some(mut batch) = self.batches.pop_front() else {
                    (self.done)(Ok(self.count));
                    return;
                };
                let cookies = std::mem::take(&mut batch.cookies).into_iter();
                self.current = Some((batch, cookies));
            }
            let (_, cookies) = self.current.as_mut().expect("current batch");
            if let Some(cookie) = cookies.next() {
                // A cookie WebKit cannot represent is skipped, so it never blocks later imports.
                let Ok(mut native) = soup_cookie(&cookie) else {
                    continue;
                };
                self.manager.clone().add_cookie(
                    &mut native,
                    None::<&gio::Cancellable>,
                    move |result| {
                        if result.is_err() {
                            (self.done)(Err(CookieApplyError));
                        } else {
                            self.count += 1;
                            self.advance();
                        }
                    },
                );
                return;
            }
            let (batch, _) = self.current.take().expect("completed batch");
            if batch.accept().is_err() {
                (self.done)(Err(CookieApplyError));
                return;
            }
        }
    }
}

fn soup_cookie(
    cookie: &crate::cookies::Cookie,
) -> std::result::Result<soup::Cookie, CookieApplyError> {
    use crate::cookies::SameSite;
    let mut native = soup::Cookie::new(
        &cookie.name,
        cookie.value.expose(),
        &cookie.domain.dotted(),
        &cookie.path,
        -1,
    );
    if let Some(expires) = cookie.expires {
        let expires = glib::DateTime::from_unix_utc(expires).map_err(|_| CookieApplyError)?;
        native.set_expires(&expires);
    }
    native.set_secure(cookie.secure);
    native.set_http_only(cookie.http_only);
    native.set_same_site_policy(match cookie.same_site {
        SameSite::None => soup::SameSitePolicy::None,
        SameSite::Lax => soup::SameSitePolicy::Lax,
        SameSite::Strict => soup::SameSitePolicy::Strict,
    });
    Ok(native)
}
