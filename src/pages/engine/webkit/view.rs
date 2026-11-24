//! One WebKitGTK view behind `EngineView`.

use std::{
    any::Any,
    cell::{Cell, RefCell},
    rc::Rc,
};

use anyhow::{Context, Result};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use webkit2gtk::{
    BackForwardListExt, BackForwardListItem, BackForwardListItemExt, FindControllerExt,
    FindOptions, InputMethodContextExt, SettingsExt, SnapshotOptions, SnapshotRegion,
    UserContentInjectedFrames, UserContentManagerExt, UserScriptInjectionTime, WebViewExt,
    WebViewSessionState,
};

use super::events::{self, ObserverCell};
use crate::pages::engine::{
    EngineView, EntryId, Frames, History, HistoryItem, Observer, ScriptDone, ScriptTime,
    ViewConfig, World,
};

pub(crate) struct WebKitView {
    native: webkit2gtk::WebView,
    widget: gtk::Widget,
    manager: webkit2gtk::UserContentManager,
    /// The script message handlers the view registered, whose messages become events.
    message_names: Vec<&'static str>,
    observer: ObserverCell,
    /// WebKit signals feeding the observer, connected while there is one.
    signals: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
}

impl WebKitView {
    pub(super) fn new(context: &webkit2gtk::WebContext, config: ViewConfig) -> Result<Rc<Self>> {
        let manager = webkit2gtk::UserContentManager::new();
        for script in &config.scripts {
            let frames = match script.frames {
                Frames::Top => UserContentInjectedFrames::TopFrame,
                Frames::All => UserContentInjectedFrames::AllFrames,
            };
            let time = match script.time {
                ScriptTime::DocumentStart => UserScriptInjectionTime::Start,
            };
            manager.add_script(&match world_name(script.world) {
                Some(world) => {
                    webkit2gtk::UserScript::for_world(script.source, frames, time, world, &[], &[])
                }
                None => webkit2gtk::UserScript::new(script.source, frames, time, &[], &[]),
            });
        }
        let mut builder = webkit2gtk::WebView::builder()
            .web_context(context)
            .user_content_manager(&manager)
            .is_controlled_by_automation(false);
        if config.autoplay {
            builder = builder.website_policies(
                &webkit2gtk::WebsitePolicies::builder()
                    .autoplay(webkit2gtk::AutoplayPolicy::Allow)
                    .build(),
            );
        }
        let native = builder.build();
        if let Some(input) = native.input_method_context() {
            input.set_enable_preedit(false);
        }
        let settings = WebViewExt::settings(&native).context("Page settings are unavailable.")?;
        settings.set_enable_webgl(config.media);
        settings.set_enable_webaudio(config.media);
        settings.set_enable_back_forward_navigation_gestures(false);
        settings.set_enable_page_cache(config.page_cache);
        settings.set_user_agent(None);
        settings.set_enable_developer_extras(config.developer_extras);
        // The zoom scales the whole page, as `EngineView::zoom` says.
        settings.set_zoom_text_only(false);
        for handler in &config.message_handlers {
            match world_name(handler.world) {
                Some(world) => {
                    manager.register_script_message_handler_in_world(handler.name, world);
                }
                None => {
                    manager.register_script_message_handler(handler.name);
                }
            }
        }
        Ok(Rc::new(Self {
            widget: native.clone().upcast(),
            native,
            manager,
            message_names: config.message_handlers.iter().map(|h| h.name).collect(),
            observer: Rc::default(),
            signals: RefCell::default(),
        }))
    }

    fn list(&self) -> Option<webkit2gtk::BackForwardList> {
        self.native.back_forward_list()
    }
}

/// The name WebKit knows `world` by; the page's own world has none.
fn world_name(world: World) -> Option<&'static str> {
    match world {
        World::Page => None,
        World::ReadingMemory => Some("canopy-reading-memory"),
        World::Hold => Some("canopy-hold"),
        World::Session => Some("canopy-session"),
    }
}

const ENTRY_KEY: &str = "canopy-entry-id";

thread_local! {
    static NEXT_ENTRY: Cell<u64> = const { Cell::new(1) };
}

/// The id Canopy knows `item` by; the first look gives it one.
///
/// The id rides on WebKit's own item object. The back/forward list keeps that object while
/// it lists the entry, so an entry keeps its id for as long as Canopy can reach it. The list's
/// `changed` signal hands over removed items as those same objects, so translating them with
/// this function gives the ids Canopy holds for them.
pub(super) fn entry_id(item: &BackForwardListItem) -> EntryId {
    // SAFETY: only this function reads or writes the key, always as a u64.
    if let Some(id) = unsafe { item.data::<u64>(ENTRY_KEY) } {
        return EntryId(unsafe { *id.as_ref() });
    }
    let id = NEXT_ENTRY.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    unsafe { item.set_data(ENTRY_KEY, id) };
    EntryId(id)
}

/// [`entry_id`] for an item the `changed` signal passes as a raw pointer.
///
/// # Safety
/// `item` must point to a live `WebKitBackForwardListItem`.
pub(super) unsafe fn entry_id_of_raw(
    item: *mut webkit2gtk::ffi::WebKitBackForwardListItem,
) -> EntryId {
    let item: BackForwardListItem = unsafe { glib::translate::from_glib_none(item) };
    entry_id(&item)
}

pub(super) fn history_item(item: &BackForwardListItem) -> HistoryItem {
    HistoryItem {
        id: entry_id(item),
        uri: item.uri().map(Into::into),
    }
}

/// TEMPORARY: the WebKit view behind `view`, for content blocking only, until engine layer
/// PR 4 puts blocking behind the engine's rule store.
pub(crate) fn temporary_webkit_view(view: &dyn EngineView) -> &webkit2gtk::WebView {
    webkit_view(view)
}

/// The WebKit view behind `view`, for the GUI tests.
#[cfg(feature = "gui-smoke")]
pub(crate) fn webkit_view_for_tests(view: &dyn EngineView) -> &webkit2gtk::WebView {
    webkit_view(view)
}

fn webkit_view(view: &dyn EngineView) -> &webkit2gtk::WebView {
    &view
        .as_any()
        .downcast_ref::<WebKitView>()
        .expect("WebKit is the only engine")
        .native
}

impl EngineView for WebKitView {
    fn widget(&self) -> &gtk::Widget {
        &self.widget
    }

    fn load_uri(&self, uri: &str) {
        self.native.load_uri(uri);
    }

    fn load_html(&self, html: &str) {
        self.native.load_html(html, None);
    }

    fn stop_loading(&self) {
        self.native.stop_loading();
    }

    fn uri(&self) -> Option<String> {
        self.native.uri().map(Into::into)
    }

    fn title(&self) -> Option<String> {
        self.native.title().map(Into::into)
    }

    fn is_loading(&self) -> bool {
        self.native.is_loading()
    }

    fn estimated_progress(&self) -> f64 {
        self.native.estimated_load_progress()
    }

    fn history(&self) -> History {
        let Some(list) = self.list() else {
            return History::default();
        };
        History {
            back: list.back_list().iter().map(history_item).collect(),
            current: list.current_item().as_ref().map(history_item),
            forward: list.forward_list().iter().map(history_item).collect(),
            length: list.length(),
        }
    }

    fn history_nth(&self, offset: i32) -> Option<EntryId> {
        self.list()?.nth_item(offset).as_ref().map(entry_id)
    }

    fn go_to(&self, entry: EntryId) {
        let Some(list) = self.list() else { return };
        let mut items = list.back_list();
        items.extend(list.forward_list());
        items.extend(list.current_item());
        if let Some(item) = items.iter().find(|item| entry_id(item) == entry) {
            self.native.go_to_back_forward_list_item(item);
        }
    }

    fn session_state(&self) -> Option<glib::Bytes> {
        self.native.session_state().and_then(|s| s.serialize())
    }

    fn restore_session_state(&self, state: &glib::Bytes) {
        let session = WebViewSessionState::new(state);
        self.native.restore_session_state(&session);
    }

    fn evaluate(
        &self,
        script: &str,
        world: World,
        cancel: Option<&gio::Cancellable>,
        done: Option<ScriptDone>,
    ) {
        let world = world_name(world);
        match done {
            Some(done) => {
                self.native
                    .evaluate_javascript(script, world, None, cancel, move |result| {
                        done(result.map(|value| value.to_str().to_string()))
                    })
            }
            None => self
                .native
                .evaluate_javascript(script, world, None, cancel, |_| {}),
        }
    }

    fn call_async(
        &self,
        body: &str,
        world: World,
        cancel: Option<&gio::Cancellable>,
        done: Box<dyn FnOnce(std::result::Result<(), glib::Error>)>,
    ) {
        self.native.call_async_javascript_function(
            body,
            None,
            world_name(world),
            None,
            cancel,
            move |result| done(result.map(|_| ())),
        );
    }

    fn insert_text(&self, text: &str) {
        self.native
            .execute_editing_command_with_argument("InsertText", text);
    }

    fn find(&self, text: &str) {
        if let Some(controller) = self.native.find_controller() {
            let options = FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND;
            controller.search(text, options.bits(), u32::MAX);
        }
    }

    fn find_next(&self) {
        if let Some(controller) = self.native.find_controller() {
            controller.search_next();
        }
    }

    fn find_previous(&self) {
        if let Some(controller) = self.native.find_controller() {
            controller.search_previous();
        }
    }

    fn find_finish(&self) {
        if let Some(controller) = self.native.find_controller() {
            controller.search_finish();
        }
    }

    fn zoom(&self) -> f64 {
        self.native.zoom_level()
    }

    fn set_zoom(&self, level: f64) {
        self.native.set_zoom_level(level);
    }

    fn is_playing_audio(&self) -> bool {
        self.native.is_playing_audio()
    }

    fn is_muted(&self) -> bool {
        self.native.is_muted()
    }

    fn set_muted(&self, muted: bool) {
        self.native.set_is_muted(muted);
    }

    fn snapshot(
        &self,
        cancel: Option<&gio::Cancellable>,
        done: Box<dyn FnOnce(std::result::Result<gtk::cairo::Surface, glib::Error>)>,
    ) {
        self.native
            .snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, cancel, done);
    }

    fn set_observer(&self, observer: Option<Observer>) {
        let connect = observer.is_some() && self.signals.borrow().is_empty();
        // Dropped once the cell is free: dropping an observer may reach back into the view.
        let old = std::mem::replace(&mut *self.observer.borrow_mut(), observer);
        drop(old);
        if connect {
            *self.signals.borrow_mut() = events::connect(
                &self.native,
                &self.manager,
                &self.message_names,
                &self.observer,
            );
        } else if self.observer.borrow().is_none() {
            for (object, id) in self.signals.take() {
                // GTK can destroy the view before its owner lets the page go.
                let connected = unsafe {
                    glib::gobject_ffi::g_signal_handler_is_connected(object.as_ptr(), id.as_raw())
                        != 0
                };
                if connected {
                    object.disconnect(id);
                }
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
