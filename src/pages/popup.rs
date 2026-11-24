//! Windows that pages open with `window.open`.
//!
//! A sign-in or payment popup only works when it shares the opener's web process: the site
//! reads `window.opener`, posts its answer back, and closes the popup. So every `window.open`
//! gets a related view. A window the page gave its own size stays a small window above Canopy
//! and is never stored in a tree. Any other `window.open` of a web address is handed to the
//! opener as a waiting child page, the way a middle-clicked link is.

use std::rc::{Rc, Weak};

use gtk::prelude::*;
use url::Url;
use webkit2gtk::{URIRequestExt, UserContentManager, WebView, WebViewExt, WindowPropertiesExt};

use crate::{address::web_url, pages::native::PageContext};

const DEFAULT_SIZE: (i32, i32) = (520, 680);
const MIN_SIZE: (i32, i32) = (320, 240);
const MAX_SIZE: (i32, i32) = (1400, 1000);

/// What the opener does with a `window.open` that asked for an ordinary window.
pub(super) type AsPage = Rc<dyn Fn(Url)>;

/// Answers `opener`'s `window.open` calls. Without `as_page`, every window opens as a popup.
pub(super) fn follow(
    opener: &WebView,
    context: Weak<PageContext>,
    as_page: Option<AsPage>,
) -> glib::SignalHandlerId {
    opener.connect_create(move |opener, action| {
        let context = context.upgrade()?;
        let target = action
            .request()
            .and_then(|request| request.uri())
            .and_then(|uri| web_url(&uri).ok());
        Some(open(opener, &context, target, as_page.clone()).upcast())
    })
}

fn open(
    opener: &WebView,
    context: &Rc<PageContext>,
    target: Option<Url>,
    as_page: Option<AsPage>,
) -> WebView {
    // A related view shares the opener's process and session. Its own content manager keeps
    // Canopy's page scripts out: their messages would be read as the opener's.
    let manager = UserContentManager::new();
    let mut builder = WebView::builder()
        .related_view(opener)
        .user_content_manager(&manager);
    if let Some(settings) = WebViewExt::settings(opener) {
        builder = builder.settings(&settings);
    }
    let popup = builder.build();
    context.blocking.follow(&popup, &manager);
    follow(&popup, Rc::downgrade(context), None);
    let parent = opener
        .toplevel()
        .and_then(|widget| widget.downcast::<gtk::Window>().ok());
    popup.connect_ready_to_show(move |popup| {
        let size = requested(popup, parent.as_ref());
        if size.is_none() {
            if let (Some(url), Some(as_page)) = (target.clone(), as_page.as_ref()) {
                popup.stop_loading();
                // SAFETY: the popup was never placed in a container, and nothing else
                // holds it after WebKit's create handler returned it.
                unsafe { popup.destroy() };
                as_page(url);
                return;
            }
        }
        show(popup, parent.as_ref(), size);
    });
    popup
}

/// The size the page asked for, when it asked for a popup. WebKit reports every window
/// feature as visible, and a `window.open` without a size gets the opener's window size.
fn requested(popup: &WebView, parent: Option<&gtk::Window>) -> Option<(i32, i32)> {
    let geometry = popup.window_properties()?.geometry();
    let size = (geometry.width(), geometry.height());
    (size.0 > 0 && size.1 > 0 && parent.is_none_or(|parent| parent.size() != size)).then_some(size)
}

fn show(popup: &WebView, parent: Option<&gtk::Window>, size: Option<(i32, i32)>) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    if let Some(parent) = parent {
        window.set_transient_for(Some(parent));
        window.set_destroy_with_parent(true);
        window.set_position(gtk::WindowPosition::CenterOnParent);
    }
    window.set_type_hint(gdk::WindowTypeHint::Dialog);

    // Popups hide the address bar on purpose; the site must still be readable before a
    // password goes into it.
    let address = gtk::Label::new(Some(" "));
    address.set_xalign(0.0);
    address.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    address.set_margin_start(10);
    address.set_margin_end(10);
    address.set_margin_top(5);
    address.set_margin_bottom(5);
    address.style_context().add_class("dim-label");
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.pack_start(&address, false, false, 0);
    column.pack_start(popup, true, true, 0);
    window.add(&column);
    column.show_all();
    // The page asked for the size of its own area, so the address line comes on top of it.
    let (width, height) = size.unwrap_or(DEFAULT_SIZE);
    window.set_default_size(
        width.clamp(MIN_SIZE.0, MAX_SIZE.0),
        height.clamp(MIN_SIZE.1, MAX_SIZE.1) + address.preferred_height().1,
    );

    let describe = {
        let window = window.downgrade();
        let address = address.downgrade();
        move |popup: &WebView| {
            let (Some(window), Some(address)) = (window.upgrade(), address.upgrade()) else {
                return;
            };
            let origin = popup
                .uri()
                .and_then(|uri| web_url(&uri).ok())
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_default();
            address.set_text(&origin);
            let title = popup.title().map(|t| t.to_string()).unwrap_or_default();
            window.set_title(match (title.trim().is_empty(), origin.is_empty()) {
                (false, _) => title.trim(),
                (true, false) => &origin,
                (true, true) => "Canopy",
            });
        }
    };
    describe(popup);
    let on_uri = describe.clone();
    popup.connect_uri_notify(move |popup| on_uri(popup));
    popup.connect_title_notify(move |popup| describe(popup));

    let closing = window.downgrade();
    popup.connect_close(move |_| {
        if let Some(window) = closing.upgrade() {
            window.close();
        }
    });
    window.connect_key_press_event(|window, event| {
        let ctrl = event.state().contains(gdk::ModifierType::CONTROL_MASK);
        if ctrl
            && matches!(
                event.keyval(),
                gdk::keys::constants::w | gdk::keys::constants::W
            )
        {
            window.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.show_all();
    popup.grab_focus();
}
