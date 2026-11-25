//! Windows that pages open with `window.open`.
//!
//! A sign-in or payment popup only works when it shares the opener's web process: the site
//! reads `window.opener`, posts its answer back, and closes the popup. So every `window.open`
//! gets a related view. A window the page gave its own size stays a small window above Canopy
//! and is never stored in a tree. Any other `window.open` of a web address is handed to the
//! opener as a waiting child page, the way a middle-clicked link is.

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use url::Url;

use crate::{
    address::web_url,
    pages::{
        context::PageContext,
        engine::{EngineView, NewWindow, Observer, ViewEvent},
    },
};

const DEFAULT_SIZE: (i32, i32) = (520, 680);
const MIN_SIZE: (i32, i32) = (320, 240);
const MAX_SIZE: (i32, i32) = (1400, 1000);

/// What the opener does with a `window.open` that asked for an ordinary window.
pub(super) type AsPage = Rc<dyn Fn(Url)>;

/// Answers a page's request for a new window. `opener` is the page's widget. Without
/// `as_page`, every window opens as a popup.
pub(super) fn open(
    opener: &gtk::Widget,
    request: &dyn NewWindow,
    context: &Rc<PageContext>,
    as_page: Option<AsPage>,
) {
    let target = request.uri().and_then(|uri| web_url(&uri).ok());
    let Ok(popup) = request.open() else {
        return;
    };
    let parent = opener
        .toplevel()
        .and_then(|widget| widget.downcast::<gtk::Window>().ok())
        .map(|window| window.downgrade());
    let blocking = context.blocking.follow(&popup);
    // The widget keeps the view while it lives; the view's observer reaches it only weakly.
    let keep = RefCell::new(Some(popup.clone()));
    popup.widget().connect_destroy(move |_| {
        if let Some(view) = keep.take() {
            view.set_observer(None);
        }
    });
    let view = Rc::downgrade(&popup);
    let context = Rc::downgrade(context);
    let shown: RefCell<Option<Shown>> = RefCell::default();
    let observer: Observer = Rc::new(move |event| {
        let Some(view) = view.upgrade() else {
            return false;
        };
        match event {
            ViewEvent::Load(event) => {
                if let Some(follow) = &blocking {
                    follow(event);
                }
            }
            ViewEvent::Create(request) => {
                if let Some(context) = context.upgrade() {
                    open(view.widget(), &*request, &context, None);
                }
            }
            ViewEvent::ReadyToShow { size } => {
                if size.is_none() {
                    if let (Some(url), Some(as_page)) = (target.clone(), as_page.as_ref()) {
                        view.stop_loading();
                        // SAFETY: the view was never placed in a container, and only the
                        // engine and this module hold it; neither uses it once destroyed.
                        unsafe { view.widget().destroy() };
                        as_page(url);
                        return false;
                    }
                }
                let parent = parent.as_ref().and_then(|parent| parent.upgrade());
                *shown.borrow_mut() = Some(show(&*view, parent.as_ref(), size));
            }
            ViewEvent::UriChanged | ViewEvent::TitleChanged => {
                if let Some(shown) = shown.borrow().as_ref() {
                    shown.describe(&*view);
                }
            }
            ViewEvent::Close => {
                let window = shown
                    .borrow()
                    .as_ref()
                    .and_then(|shown| shown.window.upgrade());
                if let Some(window) = window {
                    window.close();
                }
            }
            _ => {}
        }
        false
    });
    popup.set_observer(Some(observer));
}

/// The window around a shown popup.
struct Shown {
    window: glib::WeakRef<gtk::Window>,
    address: glib::WeakRef<gtk::Label>,
}

impl Shown {
    /// Shows the popup's origin above it and names the window after its page.
    fn describe(&self, popup: &dyn EngineView) {
        let (Some(window), Some(address)) = (self.window.upgrade(), self.address.upgrade()) else {
            return;
        };
        let origin = popup
            .uri()
            .and_then(|uri| web_url(&uri).ok())
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_default();
        address.set_text(&origin);
        let title = popup.title().unwrap_or_default();
        window.set_title(match (title.trim().is_empty(), origin.is_empty()) {
            (false, _) => title.trim(),
            (true, false) => &origin,
            (true, true) => "Canopy",
        });
    }
}

fn show(popup: &dyn EngineView, parent: Option<&gtk::Window>, size: Option<(i32, i32)>) -> Shown {
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
    column.pack_start(popup.widget(), true, true, 0);
    window.add(&column);
    column.show_all();
    // The page asked for the size of its own area, so the address line comes on top of it.
    let (width, height) = size.unwrap_or(DEFAULT_SIZE);
    window.set_default_size(
        width.clamp(MIN_SIZE.0, MAX_SIZE.0),
        height.clamp(MIN_SIZE.1, MAX_SIZE.1) + address.preferred_height().1,
    );

    let shown = Shown {
        window: window.downgrade(),
        address: address.downgrade(),
    };
    shown.describe(popup);
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
    popup.widget().grab_focus();
    shown
}
