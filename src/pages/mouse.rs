// Adapted from wry 0.57.2. Copyright Tauri Programme within The Commons Conservancy.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// WebKitGTK does not deliver back/forward mouse buttons to page JavaScript.
use std::{cell::RefCell, rc::Rc};

use gtk::{
    gdk::{EventButton, EventMask, ModifierType},
    prelude::*,
};
use webkit2gtk::WebView;

use crate::pages::engine::{EngineView, World};

/// `webview` is `view`'s WebKit view, whose signals this connects until engine layer PR 3.
pub(super) fn setup(
    webview: &WebView,
    view: std::rc::Weak<dyn EngineView>,
    pages: std::rc::Weak<super::super::Pages>,
) -> Vec<glib::SignalHandlerId> {
    webview.add_events(EventMask::BUTTON1_MOTION_MASK | EventMask::BUTTON_PRESS_MASK);

    let bf_state = BackForwardState(Rc::new(RefCell::new(0)));

    let bf_state_c = bf_state.clone();
    let press_view = view.clone();
    let press = webview.connect_button_press_event(move |_, event| {
        let mut inhibit = false;
        let webview = press_view.upgrade();
        match event.button() {
            8 => {
                inhibit = true;
                bf_state_c.set(BACK);
                if let Some(webview) = webview {
                    webview.evaluate(
                        &create_js_mouse_event(&*webview, event, true, &bf_state_c),
                        World::Page,
                        None,
                        None,
                    );
                }
            }
            9 => {
                inhibit = true;
                bf_state_c.set(FORWARD);
                if let Some(webview) = webview {
                    webview.evaluate(
                        &create_js_mouse_event(&*webview, event, true, &bf_state_c),
                        World::Page,
                        None,
                        None,
                    );
                }
            }
            _ => {}
        }

        if inhibit {
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });

    let bf_state_c = bf_state.clone();
    let release = webview.connect_button_release_event(move |_, event| {
        let mut inhibit = false;
        let webview = view.upgrade();
        match event.button() {
            8 => {
                inhibit = true;
                bf_state_c.remove(BACK);
                let pages = pages.clone();
                let Some(webview) = webview else {
                    return gtk::glib::Propagation::Stop;
                };
                webview.evaluate(
                    &create_js_mouse_event(&*webview, event, false, &bf_state_c),
                    World::Page,
                    None,
                    Some(Box::new(move |result| {
                        if result.is_ok_and(|value| value.as_str() == "true") {
                            if let Some(pages) = pages.upgrade() {
                                if let Err(error) = pages.go(super::super::Go::Parent) {
                                    pages
                                        .receive(super::super::PageEvent::Error(error.to_string()));
                                }
                            }
                        }
                    })),
                );
            }
            9 => {
                inhibit = true;
                bf_state_c.remove(FORWARD);
                let pages = pages.clone();
                let Some(webview) = webview else {
                    return gtk::glib::Propagation::Stop;
                };
                webview.evaluate(
                    &create_js_mouse_event(&*webview, event, false, &bf_state_c),
                    World::Page,
                    None,
                    Some(Box::new(move |result| {
                        if result.is_ok_and(|value| value.as_str() == "true") {
                            if let Some(pages) = pages.upgrade() {
                                if let Err(error) = pages.go(super::super::Go::Forward) {
                                    pages
                                        .receive(super::super::PageEvent::Error(error.to_string()));
                                }
                            }
                        }
                    })),
                );
            }
            _ => {}
        }
        if inhibit {
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    vec![press, release]
}

fn create_js_mouse_event(
    webview: &dyn EngineView,
    event: &EventButton,
    pressed: bool,
    state: &BackForwardState,
) -> String {
    let event_name = if pressed { "mousedown" } else { "mouseup" };
    let button = if event.button() == 8 { 3 } else { 4 };
    // The page's own coordinates: CSS pixels. A page zoom scales CSS pixels
    // against the widget's, so a point read off the widget alone can stand
    // past the page's edge, where `elementFromPoint` finds nothing. Canopy's zoom always
    // scales the whole page, never the text alone.
    let zoom = match webview.zoom() {
        zoom if zoom > 0.0 => zoom,
        _ => 1.0,
    };
    let (x, y) = event.position();
    let (x, y) = (x / zoom, y / zoom);
    let modifers_state = event.state();
    let mut buttons = 0;
    if modifers_state.contains(ModifierType::BUTTON1_MASK) {
        buttons += 1;
    }
    if modifers_state.contains(ModifierType::BUTTON3_MASK) {
        buttons += 2;
    }
    if modifers_state.contains(ModifierType::BUTTON2_MASK) {
        buttons += 4;
    }
    if state.has(BACK) {
        buttons += 8;
    }
    if state.has(FORWARD) {
        buttons += 16;
    }
    format!(
        r#"(() => {{
        const el = document.elementFromPoint({x},{y}) ?? document.documentElement;
        const ev = new MouseEvent('{event_name}', {{
          view: window,
          button: {button},
          buttons: {buttons},
          x: {x},
          y: {y},
          bubbles: true,
          detail: {detail},
          cancelBubble: false,
          cancelable: true,
          clientX: {x},
          clientY: {y},
          composed: true,
          layerX: {x},
          layerY: {y},
          pageX: {x},
          pageY: {y},
          screenX: window.screenX + {x},
          screenY: window.screenY + {y},
          ctrlKey: {ctrl_key},
          metaKey: {meta_key},
          shiftKey: {shift_key},
          altKey: {alt_key},
        }});
        el.dispatchEvent(ev)
        return !ev.defaultPrevented;
      }})()"#,
        event_name = event_name,
        x = x,
        y = y,
        detail = event.click_count().unwrap_or(1),
        ctrl_key = modifers_state.contains(ModifierType::CONTROL_MASK),
        alt_key = modifers_state.contains(ModifierType::MOD1_MASK),
        shift_key = modifers_state.contains(ModifierType::SHIFT_MASK),
        meta_key = modifers_state.contains(ModifierType::SUPER_MASK),
        button = button,
        buttons = buttons,
    )
}

const BACK: u8 = 0b01;
const FORWARD: u8 = 0b10;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BackForwardState(Rc<RefCell<u8>>);

impl BackForwardState {
    fn set(&self, button: u8) {
        *self.0.borrow_mut() |= button
    }

    fn remove(&self, button: u8) {
        *self.0.borrow_mut() &= !button
    }

    fn has(&self, button: u8) -> bool {
        let state = *self.0.borrow();
        state & !button != state
    }
}
