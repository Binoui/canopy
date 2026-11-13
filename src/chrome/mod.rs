mod card;
mod favicon;
mod graph;
mod icons;
mod memory;
mod palette;
mod presentation;
mod strip;
mod tabs;
mod theme;

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use gtk::{gdk, pango, prelude::*};
use url::Url;

pub use theme::register_font;

use crate::{
    browser::{
        NavigationTarget, SearchHit, ThemeMode, TreeId, Visit, VisitId, VisitPreview, VisitState,
    },
    forest::ForestProjection,
};
use card::{Card, CardEvent};
use favicon::Favicons;
use memory::{Memory, MemoryEvent};
use palette::{Palette, PaletteEvent};
use presentation::{Input, Layer, Output, Presentation};
use strip::{Strip, StripEvent};
use tabs::{TabEvent, Tabs};
use theme::Shade;

const HEADER: i32 = 42;
/// Room around the strip and the card for their shadows. Their frames pass pointer
/// input through, so the shadow never covers a click on the page.
const STRIP_SHADOW: i32 = 24;
const CARD_SHADOW: (i32, i32, i32) = (18, 6, 30);

/// Everything the chrome shows, read from SQLite in one go.
pub struct ChromeFrame {
    pub forest: ForestProjection,
    pub capture_enabled: bool,
    pub theme: ThemeMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteScope {
    NewTree,
    From(VisitId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteSeed {
    Empty,
    CurrentAddress,
}

/// What the chrome asks the browser to do.
#[derive(Clone, Debug, PartialEq)]
pub enum ChromeIntent {
    SelectTree(TreeId),
    Restore(VisitId),
    /// Activate a queued page, then open a palette anchored on it once it commits.
    BranchFromQueued(VisitId),
    Navigate {
        url: Url,
        target: NavigationTarget,
    },
    OpenCitation(VisitId),
    ReadCapture(i64),
    ReadCaptures(VisitId),
    SetCaptureEnabled(bool),
    RememberSelection,
}

pub enum Toast {
    Info(String),
    Error(String),
}

/// The side of the browser the chrome talks to: commands, and read-only queries.
pub trait ChromeHost {
    fn dispatch(&self, intent: ChromeIntent);
    fn search(&self, query: &str) -> Result<Vec<SearchHit>>;
    fn preview(&self, visit: VisitId) -> Result<VisitPreview>;
}

pub struct NativeChrome {
    inner: Rc<Inner>,
}

struct Inner {
    page: gtk::Widget,
    css: gtk::CssProvider,
    theme: Cell<ThemeMode>,
    shade: Cell<Option<Shade>>,
    tabs: Rc<Tabs>,
    strip: Rc<Strip>,
    card: Rc<Card>,
    card_x: Cell<i32>,
    palette: Rc<Palette>,
    memory: Rc<Memory>,
    toast: gtk::Revealer,
    toast_label: gtk::Label,
    toast_generation: Cell<u64>,
    icons_stale: Cell<bool>,
    presentation: RefCell<Presentation>,
    forest: RefCell<Option<Rc<ForestProjection>>>,
    host: Weak<dyn ChromeHost>,
}

impl NativeChrome {
    /// Adds the chrome above `overlay`'s main child, whose first 94 px it covers.
    pub fn mount(
        overlay: &gtk::Overlay,
        page: &webkit2gtk::WebView,
        host: Weak<dyn ChromeHost>,
    ) -> Self {
        let css = gtk::CssProvider::new();
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let favicons = Favicons::new(page);
        let inner = Rc::new_cyclic(|weak: &Weak<Inner>| {
            let w = weak.clone();
            let tabs = Tabs::new(favicons.clone(), move |event| {
                if let Some(inner) = w.upgrade() {
                    inner.tab_event(event);
                }
            });
            let w = weak.clone();
            let memory = Memory::new(move |event| {
                if let Some(inner) = w.upgrade() {
                    inner.memory_event(event);
                }
            });
            let w = weak.clone();
            let strip = Strip::new(favicons.clone(), &memory.button, move |event| {
                if let Some(inner) = w.upgrade() {
                    inner.strip_event(event);
                }
            });
            let w = weak.clone();
            let card = Card::new(favicons.clone(), move |event| {
                if let Some(inner) = w.upgrade() {
                    inner.card_event(event);
                }
            });
            let search_host = host.clone();
            let w = weak.clone();
            let palette = Palette::new(
                favicons.clone(),
                page.clone().upcast(),
                move |query| {
                    search_host
                        .upgrade()
                        .and_then(|host| host.search(query).ok())
                        .unwrap_or_default()
                },
                move |event| {
                    if let Some(inner) = w.upgrade() {
                        inner.palette_event(event);
                    }
                },
            );
            let toast = gtk::Revealer::new();
            toast.set_widget_name("toast-revealer");
            toast.set_transition_type(gtk::RevealerTransitionType::Crossfade);
            toast.set_transition_duration(200);
            let toast_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            toast_box.set_widget_name("toast");
            let toast_label = gtk::Label::new(None);
            toast_label.set_widget_name("toast-label");
            toast_label.set_line_wrap(true);
            toast_label.set_max_width_chars(70);
            toast_box.pack_start(&toast_label, false, false, 0);
            toast.add(&toast_box);
            toast.set_no_show_all(true);
            toast_box.show_all();
            Inner {
                page: page.clone().upcast(),
                css,
                theme: Cell::new(ThemeMode::System),
                shade: Cell::new(None),
                tabs,
                strip,
                card,
                card_x: Cell::new(0),
                palette,
                memory,
                toast,
                toast_label,
                toast_generation: Cell::new(0),
                icons_stale: Cell::new(false),
                presentation: RefCell::new(Presentation::default()),
                forest: RefCell::new(None),
                host,
            }
        });
        overlay.add_overlay(&inner.tabs.root);
        let strip_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        strip_frame.set_widget_name("strip-frame");
        inner.strip.root.set_margin_bottom(STRIP_SHADOW);
        strip_frame.pack_start(&inner.strip.root, true, true, 0);
        let weak = Rc::downgrade(&inner);
        strip_frame.connect_draw(move |frame, cr| {
            if let Some(inner) = weak.upgrade() {
                inner.strip.paint_shadow(cr, frame.allocated_width());
            }
            glib::Propagation::Proceed
        });
        overlay.add_overlay(&strip_frame);
        overlay.set_overlay_pass_through(&strip_frame, true);
        let (x, top, bottom) = CARD_SHADOW;
        inner.card.frame.set_margin_start(x);
        inner.card.frame.set_margin_end(x);
        inner.card.frame.set_margin_top(top);
        inner.card.frame.set_margin_bottom(bottom);
        let card_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card_frame.set_widget_name("card-frame");
        card_frame.set_no_show_all(true);
        card_frame.pack_start(&inner.card.frame, true, true, 0);
        overlay.add_overlay(&card_frame);
        overlay.set_overlay_pass_through(&card_frame, true);
        overlay.add_overlay(&inner.palette.root);
        overlay.add_overlay(&inner.toast);
        overlay.set_overlay_pass_through(&inner.toast, true);
        let weak = Rc::downgrade(&inner);
        overlay.connect_get_child_position(move |overlay, widget| {
            let inner = weak.upgrade()?;
            let width = overlay.allocated_width();
            let height = overlay.allocated_height();
            let rect = match widget.widget_name().as_str() {
                "tabs" => (0, 0, width, HEADER),
                "strip-frame" => (
                    0,
                    HEADER,
                    width,
                    inner.strip.allocation_height() + STRIP_SHADOW,
                ),
                "card-frame" => {
                    let (x, top, bottom) = CARD_SHADOW;
                    let y = HEADER + graph::OPEN_HEIGHT as i32 + 8 - top;
                    let (_, natural) = widget.preferred_height_for_width(card::WIDTH + 2 * x);
                    (
                        inner.card_x.get() - x,
                        y,
                        card::WIDTH + 2 * x,
                        natural.min(height - y - 8 + bottom),
                    )
                }
                "palette" => (0, 0, width, height),
                "toast-revealer" => {
                    let (_, natural) = widget.preferred_size();
                    let w = natural.width.min(width - 40);
                    let h = natural.height;
                    ((width - w) / 2, height - h - 20, w, h)
                }
                _ => return None,
            };
            Some(gdk::Rectangle::new(rect.0, rect.1, rect.2, rect.3))
        });
        if let Some(settings) = gtk::Settings::default() {
            for property in ["gtk-application-prefer-dark-theme", "gtk-theme-name"] {
                let weak = Rc::downgrade(&inner);
                settings.connect_notify_local(Some(property), move |_, _| {
                    if let Some(inner) = weak.upgrade() {
                        inner.apply_theme();
                    }
                });
            }
        }
        inner.apply_theme();
        let weak = Rc::downgrade(&inner);
        favicons.connect_changed(move || {
            if let Some(inner) = weak.upgrade() {
                inner.schedule_icon_refresh();
            }
        });
        Self { inner }
    }

    pub fn present(&self, frame: ChromeFrame) {
        let inner = &self.inner;
        let forest = Rc::new(frame.forest);
        *inner.forest.borrow_mut() = Some(forest.clone());
        inner.tabs.present(&forest);
        inner.strip.present(forest.clone());
        inner.palette.present(forest.clone());
        if let Some(id) = inner.card.visit() {
            match forest.visit(id) {
                Some(_) => inner.show_card(id),
                None => inner.step(Input::Activated),
            }
        }
        inner.theme.set(frame.theme);
        inner.apply_theme();
        inner
            .memory
            .present(frame.capture_enabled, forest.current(), inner.shade());
    }

    pub fn open_palette(&self, scope: PaletteScope, seed: PaletteSeed) {
        self.inner.open_palette(scope, seed);
    }

    pub fn toast(&self, toast: Toast) {
        self.inner.toast(toast);
    }

    /// F6 moves keyboard focus between the page and the current node.
    pub fn toggle_focus(&self) {
        if self.inner.strip.has_focus_inside() || !self.inner.strip.focus_current() {
            self.inner.page.grab_focus();
        }
    }

    /// Closes the topmost chrome surface. Returns false when there was none, so the page
    /// can have the key.
    pub fn escape(&self) -> bool {
        if self.inner.palette.is_open() {
            self.inner.palette.close();
            return true;
        }
        false
    }
}

impl Inner {
    fn shade(&self) -> Shade {
        self.shade.get().unwrap_or(Shade::Light)
    }

    /// Resolves `session.theme_mode`, with `system` following GTK's dark preference.
    fn apply_theme(&self) {
        let shade = match self.theme.get() {
            ThemeMode::Light => Shade::Light,
            ThemeMode::Dark => Shade::Dark,
            ThemeMode::System => {
                let settings = gtk::Settings::default();
                let prefers_dark = settings.as_ref().is_some_and(|s| {
                    s.is_gtk_application_prefer_dark_theme()
                        || s.gtk_theme_name()
                            .is_some_and(|name| name.to_lowercase().contains("dark"))
                });
                if prefers_dark {
                    Shade::Dark
                } else {
                    Shade::Light
                }
            }
        };
        if self.shade.replace(Some(shade)) == Some(shade) {
            return;
        }
        self.css
            .load_from_data(shade.css().as_bytes())
            .expect("the generated chrome CSS parses");
        self.strip.set_shade(shade);
    }

    fn step(self: &Rc<Self>, input: Input) {
        let outputs = self.presentation.borrow_mut().step(input);
        for output in outputs {
            match output {
                Output::Schedule(timer, delay, token) => {
                    let weak = Rc::downgrade(self);
                    glib::timeout_add_local_once(delay, move || {
                        if let Some(inner) = weak.upgrade() {
                            inner.step(Input::Fired(timer, token));
                        }
                    });
                }
                Output::Strip { open } => self.strip.set_open(open),
                Output::ShowCard(id) => self.show_card(id),
                Output::HideCard => self.card.hide(),
            }
        }
    }

    /// Icons arrive in bursts while a page loads; redraw them once per idle.
    fn schedule_icon_refresh(self: &Rc<Self>) {
        if self.icons_stale.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(inner) = weak.upgrade() else { return };
            inner.icons_stale.set(false);
            let forest = inner.forest.borrow().clone();
            if let Some(forest) = forest {
                inner.tabs.present(&forest);
                inner.strip.refresh_icons();
                if let Some(id) = inner.card.visit() {
                    inner.show_card(id);
                }
            }
        });
    }

    fn show_card(&self, id: VisitId) {
        let forest = self.forest.borrow().clone();
        let (Some(forest), Some(rect)) = (forest, self.strip.node_rect(id)) else {
            return;
        };
        let Some(preview) = self.host.upgrade().and_then(|host| host.preview(id).ok()) else {
            return;
        };
        let width = self.strip.root.allocated_width();
        let x = rect.x() + rect.width() / 2 - card::WIDTH / 2;
        self.card_x.set(x.min(width - card::WIDTH - 8).max(8));
        self.card.show(&forest, id, preview);
        self.card.frame.queue_resize();
    }

    fn card_event(self: &Rc<Self>, event: CardEvent) {
        match event {
            CardEvent::Enter => self.step(Input::EnterCard),
            CardEvent::Leave => self.step(Input::LeaveCard),
            CardEvent::Open(id) => {
                self.step(Input::Activated);
                self.emit(ChromeIntent::Restore(id));
            }
            CardEvent::Branch(id) => {
                self.step(Input::Activated);
                let queued = self
                    .forest
                    .borrow()
                    .as_ref()
                    .and_then(|forest| forest.visit(id))
                    .is_some_and(|visit| visit.state == VisitState::Queued);
                if queued {
                    self.emit(ChromeIntent::BranchFromQueued(id));
                } else {
                    self.open_palette(PaletteScope::From(id), PaletteSeed::Empty);
                }
            }
            CardEvent::BackToSource(parent) => {
                self.step(Input::Activated);
                self.emit(ChromeIntent::Restore(parent));
            }
            CardEvent::OpenComment(child) => {
                self.step(Input::Activated);
                self.emit(ChromeIntent::OpenCitation(child));
            }
        }
    }

    fn emit(&self, intent: ChromeIntent) {
        if let Some(host) = self.host.upgrade() {
            host.dispatch(intent);
        }
    }

    fn open_palette(self: &Rc<Self>, scope: PaletteScope, seed: PaletteSeed) {
        self.step(Input::Layer(Layer::Palette));
        self.palette.open(scope, seed);
    }

    fn current(&self) -> Option<VisitId> {
        self.forest.borrow().as_ref().and_then(|f| f.current())
    }

    fn tab_event(self: &Rc<Self>, event: TabEvent) {
        match event {
            TabEvent::Select(tree) => self.emit(ChromeIntent::SelectTree(tree)),
            TabEvent::NewTree => self.open_palette(PaletteScope::NewTree, PaletteSeed::Empty),
            TabEvent::Search => {
                let scope = self
                    .current()
                    .map_or(PaletteScope::NewTree, PaletteScope::From);
                self.open_palette(scope, PaletteSeed::Empty);
            }
        }
    }

    fn strip_event(self: &Rc<Self>, event: StripEvent) {
        match event {
            StripEvent::Enter => self.step(Input::EnterStrip),
            StripEvent::Leave => self.step(Input::LeaveStrip),
            StripEvent::EnterNode(id) => self.step(Input::EnterNode(id)),
            StripEvent::LeaveNode => self.step(Input::LeaveNode),
            StripEvent::Activate(id) => {
                self.step(Input::Activated);
                if Some(id) != self.current() {
                    self.emit(ChromeIntent::Restore(id));
                }
            }
            StripEvent::Menu(open) => self.menu(open),
        }
    }

    fn memory_event(self: &Rc<Self>, event: MemoryEvent) {
        match event {
            MemoryEvent::Run(intent) => self.emit(intent),
            MemoryEvent::Menu(open) => self.menu(open),
        }
    }

    fn menu(self: &Rc<Self>, open: bool) {
        if self.presentation.borrow().layer() != Layer::Palette {
            self.step(Input::Layer(if open { Layer::Menu } else { Layer::None }));
        }
    }

    fn palette_event(self: &Rc<Self>, event: PaletteEvent) {
        match event {
            PaletteEvent::Run(intent) => self.emit(intent),
            PaletteEvent::Closed => self.step(Input::Layer(Layer::None)),
        }
    }

    fn toast(self: &Rc<Self>, toast: Toast) {
        let (text, duration) = match &toast {
            Toast::Info(text) => (text.as_str(), 2600),
            Toast::Error(text) => {
                self.memory.record_error(text);
                (text.as_str(), 5000)
            }
        };
        self.toast_label.set_text(text);
        self.toast.show();
        self.toast.set_reveal_child(true);
        let generation = self.toast_generation.get() + 1;
        self.toast_generation.set(generation);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(duration), move || {
            if let Some(inner) = weak.upgrade() {
                if inner.toast_generation.get() == generation {
                    inner.toast.set_reveal_child(false);
                }
            }
        });
    }
}

/// A clickable row with a page's icon, title and state, as used in the "+N" lists.
fn visit_row(favicons: &Rc<Favicons>, visit: &Visit, scale: i32) -> gtk::Button {
    let row = gtk::Button::new();
    row.style_context().add_class("menu-row");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    let surface = favicons.surface(visit, 16, scale);
    content.pack_start(&gtk::Image::from_surface(Some(&surface)), false, false, 0);
    let title = gtk::Label::new(Some(visit.label()));
    title.set_xalign(0.);
    title.set_ellipsize(pango::EllipsizeMode::End);
    title.set_max_width_chars(34);
    content.pack_start(&title, true, true, 0);
    let detail = gtk::Label::new(Some(&match visit.state {
        VisitState::Queued => "pending".to_owned(),
        VisitState::Opened => ago(visit.opened_at.unwrap_or(visit.created_at)),
    }));
    detail.style_context().add_class("menu-detail");
    content.pack_start(&detail, false, false, 0);
    row.add(&content);
    row
}

fn ago(timestamp_ms: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let minutes = (now - timestamp_ms).max(0) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", (minutes + 30) / 60),
        _ => match (minutes + 720) / 1440 {
            1 => "yesterday".into(),
            days => format!("{days} days ago"),
        },
    }
}
