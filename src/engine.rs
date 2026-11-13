use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

use anyhow::{bail, Context, Result};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use url::Url;
use webkit2gtk::{
    LoadEvent, NavigationPolicyDecision, NavigationPolicyDecisionExt, NavigationType, NetworkError,
    PolicyDecision, PolicyDecisionExt, PolicyDecisionType, ResponsePolicyDecision,
    ResponsePolicyDecisionExt, URIRequestExt, UserContentInjectedFrames, UserContentManagerExt,
    UserScript, UserScriptInjectionTime, WebContextExt, WebViewExt,
};
use wry::{WebViewBuilderExtUnix, WebViewExtUnix};

use crate::{
    address::web_url,
    browser::{
        Browser, Capture, CaptureKind, CapturePayload, NavigationTarget, SourceContext, VisitId,
    },
};

const WORLD: &str = "canopy-reading-memory";
const HELPER: &str = include_str!("capture.js");
type Observer = RefCell<Option<Box<dyn Fn(EngineEvent)>>>;

pub enum EngineEvent {
    Changed,
    Loading(bool),
    Error(String),
    Remembered(i64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Document {
    visit: VisitId,
    stored_url: Url,
    page_url: Url,
    epoch: u64,
}

struct NativeState {
    native: webkit2gtk::WebView,
    browser: Rc<RefCell<Browser>>,
    observer: Observer,
    document: RefCell<Option<Document>>,
    epoch: Cell<u64>,
    load_generation: Cell<u64>,
    foreground_generation: Cell<u64>,
    same_document_request: RefCell<Option<Url>>,
}

pub struct Engine {
    _view: wry::WebView,
    pub native: webkit2gtk::WebView,
    state: Rc<NativeState>,
}

impl Engine {
    pub fn new(
        container: &gtk::Box,
        browser: Rc<RefCell<Browser>>,
        profile: &Path,
    ) -> Result<Self> {
        std::fs::create_dir_all(profile)?;
        let mut context = wry::WebContext::new(Some(profile.to_owned()));
        let view = wry::WebViewBuilder::new_with_web_context(&mut context)
            .with_back_forward_navigation_gestures(false)
            .with_html(HOME)
            .build_gtk(container)?;
        let native = view.webview();
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
            document: RefCell::new(None),
            epoch: Cell::new(0),
            load_generation: Cell::new(0),
            foreground_generation: Cell::new(0),
            same_document_request: RefCell::new(None),
        });
        {
            let weak = Rc::downgrade(&state);
            native.connect_load_changed(move |native, event| {
                let Some(state) = weak.upgrade() else { return };
                let url = native.uri().and_then(|uri| web_url(&uri).ok());
                match event {
                    LoadEvent::Started => {
                        state.same_document_request.borrow_mut().take();
                        state.invalidate();
                        if let Some(url) = url {
                            let result = state.browser.borrow_mut().started(url);
                            state.report(result);
                            state.emit(EngineEvent::Loading(true));
                        }
                    }
                    LoadEvent::Redirected => {
                        if let Some(url) = url {
                            state.browser.borrow_mut().redirected(url);
                        }
                    }
                    LoadEvent::Committed => {
                        if let Some(url) = url {
                            let result = state.browser.borrow_mut().committed(url.clone());
                            match result {
                                Ok(true) => {
                                    state.bind_document(url);
                                    state.emit(EngineEvent::Changed);
                                }
                                Ok(false) => {}
                                Err(error) => state.emit(EngineEvent::Error(error.to_string())),
                            }
                        }
                    }
                    LoadEvent::Finished => {
                        state.complete_same_document();
                        state.update_title();
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
            native.connect_load_failed(move |_, _, uri, error| {
                let Some(state) = weak.upgrade() else {
                    return true;
                };
                let cancelled = error.matches(NetworkError::Cancelled);
                if let Ok(url) = web_url(uri) {
                    state.browser.borrow_mut().failed(&url, cancelled);
                }
                if !cancelled {
                    state.recover_document();
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
            native.connect_load_failed_with_tls_errors(move |_, uri, _, _| {
                if let Some(state) = weak.upgrade() {
                    if let Ok(url) = web_url(uri) {
                        state.browser.borrow_mut().failed(&url, false);
                    }
                    state.recover_document();
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
        {
            let weak = Rc::downgrade(&state);
            native.connect_uri_notify(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.complete_same_document();
                }
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
        if let Some(context) = native.context() {
            let weak = Rc::downgrade(&state);
            context.connect_download_started(move |_, download| {
                use webkit2gtk::DownloadExt;
                download.cancel();
                if let Some(state) = weak.upgrade() {
                    state.emit(EngineEvent::Error(
                        "Downloads are not supported yet.".into(),
                    ));
                }
            });
        }
        Ok(Self {
            _view: view,
            native,
            state,
        })
    }

    pub fn observe(&self, callback: impl Fn(EngineEvent) + 'static) {
        *self.state.observer.borrow_mut() = Some(Box::new(callback));
    }

    pub fn navigate(&self, url: Url, target: NavigationTarget) -> Result<()> {
        let url = self.state.browser.borrow_mut().navigate(url, target)?;
        self.state.load(url);
        Ok(())
    }

    pub fn restore(&self, id: VisitId) -> Result<()> {
        let url = self.state.browser.borrow_mut().restore(id)?;
        self.state.load(url);
        Ok(())
    }

    pub fn back(&self) -> Result<()> {
        let request = self.state.browser.borrow_mut().back()?;
        if let Some(url) = request {
            self.state.load(url);
        }
        Ok(())
    }

    pub fn reload(&self) -> Result<()> {
        let request = self.state.browser.borrow_mut().reload()?;
        if let Some(url) = request {
            self.state.load(url);
        }
        Ok(())
    }

    pub fn open_citation(&self, child: VisitId) -> Result<()> {
        let request = self.state.browser.borrow_mut().open_citation(child)?;
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
            self.state.browser.borrow_mut().restore(visit.id)?
        } else {
            let source = SourceContext {
                page_url: capture.document_url.clone(),
                citation_url: capture.citation_url.clone(),
                link_text: capture.title.clone(),
                excerpt: capture.text.chars().take(1000).collect(),
            };
            self.state
                .browser
                .borrow_mut()
                .follow_link(visit.id, target, source)?
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

    fn invalidate(&self) {
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.document.borrow_mut().take();
    }

    fn bind_document(&self, page_url: Url) {
        let current = self
            .browser
            .borrow()
            .snapshot()
            .ok()
            .and_then(|snapshot| snapshot.current_visit().cloned());
        if let Some(visit) = current {
            *self.document.borrow_mut() = Some(Document {
                visit: visit.id,
                stored_url: visit.url,
                page_url,
                epoch: self.epoch.get(),
            });
        }
    }

    fn recover_document(&self) {
        if let Some(url) = self.native.uri().and_then(|uri| web_url(&uri).ok()) {
            let current = self
                .browser
                .borrow()
                .snapshot()
                .ok()
                .and_then(|snapshot| snapshot.current_visit().cloned());
            if current.is_some_and(|visit| visit.url == url) {
                self.bind_document(url);
            }
        }
    }

    fn ticket(&self) -> Option<Document> {
        let mut ticket = self.document.borrow().clone()?;
        let actual = self.native.uri().and_then(|uri| web_url(&uri).ok())?;
        if ticket.epoch != self.epoch.get() || actual.origin() != ticket.stored_url.origin() {
            return None;
        }
        ticket.page_url = actual;
        Some(ticket)
    }

    fn valid(&self, ticket: &Document) -> bool {
        self.ticket().as_ref() == Some(ticket)
    }

    fn load(self: &Rc<Self>, url: Url) {
        let generation = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(generation);
        self.foreground_generation
            .set(self.foreground_generation.get().wrapping_add(1));
        self.same_document_request.borrow_mut().take();
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                if state.load_generation.get() == generation {
                    let current = state.native.uri().and_then(|uri| web_url(&uri).ok());
                    if state.ticket().is_some()
                        && !state.native.is_loading()
                        && current
                            .is_some_and(|current| current != url && same_resource(&current, &url))
                    {
                        *state.same_document_request.borrow_mut() = Some(url.clone());
                    }
                    state.native.load_uri(url.as_str());
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
        self.invalidate();
        let started = self.browser.borrow_mut().started(target.clone());
        if let Err(error) = started {
            self.emit(EngineEvent::Error(error.to_string()));
            return;
        }
        let committed = self.browser.borrow_mut().committed(target.clone());
        match committed {
            Ok(true) => {
                self.bind_document(target);
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
            if decision
                .downcast_ref::<ResponsePolicyDecision>()
                .is_some_and(|response| !response.is_mime_type_supported())
            {
                decision.ignore();
                self.emit(EngineEvent::Error(
                    "Downloads are not supported yet.".into(),
                ));
                return true;
            }
            return false;
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
            decision.ignore();
            self.emit(EngineEvent::Error(
                "This kind of link is not supported.".into(),
            ));
            return true;
        };
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
                            let result = state.browser.borrow_mut().set_source_context(
                                child,
                                ticket.visit,
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
        if action.navigation_type() == NavigationType::Reload {
            let result = self.browser.borrow_mut().reload().map(|_| ());
            self.report(result);
        } else if action.navigation_type() == NavigationType::BackForward {
            decision.ignore();
            let result = self.browser.borrow_mut().back();
            match result {
                Ok(Some(url)) => self.load(url),
                Ok(None) => {}
                Err(error) => self.emit(EngineEvent::Error(error.to_string())),
            }
            return true;
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
                    let request = state
                        .browser
                        .borrow_mut()
                        .follow_link(ticket.visit, url, source);
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

    fn update_title(&self) {
        if let (Some(uri), Some(title)) = (self.native.uri(), self.native.title()) {
            if let Ok(url) = web_url(&uri) {
                let result = self.browser.borrow_mut().title_changed(&url, &title);
                self.report(result);
                self.emit(EngineEvent::Changed);
            }
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
            let result =
                state
                    .browser
                    .borrow_mut()
                    .save_capture(ticket.visit, &ticket.stored_url, payload);
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
