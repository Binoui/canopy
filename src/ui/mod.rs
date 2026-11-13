mod reader;

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};

use anyhow::Result;
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

pub use crate::chrome::register_font;
use crate::{
    browser::{Browser, NavigationTarget, SearchHit, Snapshot, VisitId, VisitPreview},
    chrome::{
        ChromeFrame, ChromeHost, ChromeIntent, NativeChrome, PaletteScope, PaletteSeed, Toast,
    },
    engine::{Engine, EngineEvent},
    forest::ForestProjection,
};

const CHROME_HEIGHT: i32 = 94;

pub struct Ui {
    pub window: gtk::Window,
    pub engine: Engine,
    browser: Rc<RefCell<Browser>>,
    chrome: NativeChrome,
    refresh_scheduled: Cell<bool>,
    /// A queued page activated by "New branch here", whose palette opens once it commits.
    branch_after_commit: Cell<Option<VisitId>>,
    path_after_commit: RefCell<Option<PathContinuation>>,
    me: Weak<Ui>,
}

struct PathContinuation {
    previous: Option<VisitId>,
    second: url::Url,
}

impl Ui {
    pub fn new(data_dir: &Path) -> Result<Rc<Self>> {
        let browser = Rc::new(RefCell::new(Browser::open(
            &data_dir.join("forest.sqlite3"),
        )?));
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Canopy");
        window.set_default_size(1240, 800);
        window.set_widget_name("canopy");
        let overlay = gtk::Overlay::new();
        window.add(&overlay);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_size_request(-1, CHROME_HEIGHT);
        column.pack_start(&spacer, false, false, 0);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.pack_start(&content, true, true, 0);
        overlay.add(&column);
        let engine = Engine::new(&content, browser.clone(), &data_dir.join("profile"))?;
        let ui = Rc::new_cyclic(|me: &Weak<Ui>| {
            let host: Weak<dyn ChromeHost> = me.clone();
            let chrome = NativeChrome::mount(&overlay, &engine.native, host);
            Ui {
                window,
                engine,
                browser,
                chrome,
                refresh_scheduled: Cell::new(false),
                branch_after_commit: Cell::new(None),
                path_after_commit: RefCell::new(None),
                me: me.clone(),
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.engine.observe(move |event| {
            let Some(ui) = weak.upgrade() else { return };
            match event {
                EngineEvent::Changed => ui.schedule_refresh(),
                EngineEvent::Loading(false) => {
                    let current = ui.snapshot().ok().and_then(|s| s.current);
                    if ui.branch_after_commit.get() != current {
                        ui.branch_after_commit.set(None);
                    }
                    ui.schedule_refresh();
                }
                EngineEvent::Loading(true) => {}
                EngineEvent::Error(message) => {
                    ui.branch_after_commit.set(None);
                    ui.path_after_commit.borrow_mut().take();
                    ui.chrome.toast(Toast::Error(message));
                }
                EngineEvent::Remembered(_) => ui
                    .chrome
                    .toast(Toast::Info("Selection remembered on this computer.".into())),
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_key_press_event(move |_, event| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |ui| ui.shortcut(event))
        });
        ui.refresh();
        Ok(ui)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        self.browser.borrow().snapshot()
    }

    pub fn search_hits(&self, query: &str) -> Result<Vec<SearchHit>> {
        self.browser.borrow().search(query)
    }

    pub fn resume(&self) -> Result<()> {
        self.engine.reload()
    }

    fn read_frame(&self) -> Result<ChromeFrame> {
        let browser = self.browser.borrow();
        Ok(ChromeFrame {
            forest: ForestProjection::from_snapshot(browser.snapshot()?, browser.tree_states()?)?,
            capture_enabled: browser.capture_enabled()?,
            theme: browser.theme_mode()?,
            suggestions: browser.start_suggestions()?,
        })
    }

    /// Bursts of engine events collapse into one frame on the next idle.
    fn schedule_refresh(&self) {
        if self.refresh_scheduled.replace(true) {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(ui) = weak.upgrade() {
                ui.refresh_scheduled.set(false);
                ui.refresh();
            }
        });
    }

    fn refresh(&self) {
        match self.read_frame() {
            Ok(frame) => {
                let title = frame
                    .forest
                    .current()
                    .and_then(|id| frame.forest.visit(id))
                    .map_or_else(
                        || "Canopy".to_owned(),
                        |v| format!("{} · Canopy", v.label()),
                    );
                self.window.set_title(&title);
                let current = frame.forest.current();
                self.chrome.present(frame);
                if !self.engine.native.is_loading() {
                    let path = self.path_after_commit.borrow_mut().take();
                    if let Some(path) = path {
                        if let Some(root) = current.filter(|id| Some(*id) != path.previous) {
                            self.report(
                                self.engine
                                    .navigate(path.second, NavigationTarget::ChildOf(root)),
                            );
                        }
                    }
                }
                if current.is_some()
                    && self.branch_after_commit.get() == current
                    && !self.engine.native.is_loading()
                {
                    self.branch_after_commit.set(None);
                    self.chrome.open_palette(
                        current.map_or(PaletteScope::NewTree, PaletteScope::From),
                        PaletteSeed::Empty,
                    );
                }
            }
            Err(error) => self.report_error(&error.to_string()),
        }
    }

    fn report(&self, result: Result<()>) {
        if let Err(error) = result {
            self.report_error(&error.to_string());
        }
    }

    fn report_error(&self, message: &str) {
        self.chrome.toast(Toast::Error(message.to_owned()));
    }

    fn open_palette(&self, seed: PaletteSeed) {
        let current = self
            .browser
            .borrow()
            .snapshot()
            .ok()
            .and_then(|s| s.current);
        let scope = current.map_or(PaletteScope::NewTree, PaletteScope::From);
        self.chrome.open_palette(scope, seed);
    }

    fn shortcut(&self, event: &gdk::EventKey) -> glib::Propagation {
        let Some(shortcut) = Shortcut::of(event) else {
            return glib::Propagation::Proceed;
        };
        self.path_after_commit.borrow_mut().take();
        match shortcut {
            Shortcut::Palette(seed) => self.open_palette(seed),
            Shortcut::NewTree => self.chrome.new_tree(),
            Shortcut::Forest => self.chrome.toggle_forest(),
            Shortcut::Reload => self.report(self.engine.reload()),
            Shortcut::RememberSelection => self.report(self.engine.remember_selection()),
            Shortcut::Quit => self.window.close(),
            Shortcut::Tree(index) => {
                let tree = self
                    .browser
                    .borrow()
                    .tree_states()
                    .ok()
                    .and_then(|trees| trees.get(index).map(|state| state.root));
                if let Some(tree) = tree {
                    self.dispatch(ChromeIntent::SelectTree(tree));
                }
            }
            Shortcut::Back => self.report(self.engine.back()),
            Shortcut::ToggleFocus => self.chrome.toggle_focus(),
            Shortcut::Escape if self.chrome.escape() => {}
            Shortcut::Escape => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }
}

/// Window-level keys, handled before the focused web view sees them.
enum Shortcut {
    Palette(PaletteSeed),
    NewTree,
    Forest,
    Reload,
    RememberSelection,
    Quit,
    Tree(usize),
    Back,
    ToggleFocus,
    Escape,
}

impl Shortcut {
    fn of(event: &gdk::EventKey) -> Option<Self> {
        use gdk::keys::constants as key;
        let keyval = event.keyval().to_lower();
        let control = event.state().contains(gdk::ModifierType::CONTROL_MASK);
        let alt = event.state().contains(gdk::ModifierType::MOD1_MASK);
        let digit = keyval.to_unicode().and_then(|c| c.to_digit(10));
        Some(match (control, alt) {
            (true, _) if keyval == key::k => Shortcut::Palette(PaletteSeed::Empty),
            (true, _) if keyval == key::l => Shortcut::Palette(PaletteSeed::CurrentAddress),
            (true, _) if keyval == key::t => Shortcut::NewTree,
            (true, _) if keyval == key::e => Shortcut::Forest,
            (true, _) if keyval == key::r => Shortcut::Reload,
            (true, _) if keyval == key::s => Shortcut::RememberSelection,
            (true, _) if keyval == key::q => Shortcut::Quit,
            (true, _) if digit.is_some_and(|d| d > 0) => Shortcut::Tree(digit? as usize - 1),
            (_, true) if keyval == key::Left => Shortcut::Back,
            _ if keyval == key::F6 => Shortcut::ToggleFocus,
            _ if keyval == key::Escape => Shortcut::Escape,
            _ => return None,
        })
    }
}

impl ChromeHost for Ui {
    fn dispatch(&self, intent: ChromeIntent) {
        let Some(ui) = self.me.upgrade() else { return };
        self.branch_after_commit.set(None);
        self.path_after_commit.borrow_mut().take();
        let result = match intent {
            ChromeIntent::CancelContinuation => Ok(()),
            ChromeIntent::OpenPath(path) => {
                let previous = self.snapshot().ok().and_then(|s| s.current);
                let result = self.engine.navigate(path.first, NavigationTarget::Root);
                if result.is_ok() {
                    *self.path_after_commit.borrow_mut() = Some(PathContinuation {
                        previous,
                        second: path.second,
                    });
                }
                result
            }
            ChromeIntent::SetTheme(mode) => {
                let result = self.browser.borrow_mut().set_theme_mode(mode);
                self.schedule_refresh();
                result
            }
            ChromeIntent::SelectTree(tree) => {
                let target = self.browser.borrow().tree_target(tree);
                target.and_then(|visit| self.engine.restore(visit))
            }
            ChromeIntent::Restore(visit) => self.engine.restore(visit),
            ChromeIntent::BranchFromQueued(visit) => {
                let result = self.engine.restore(visit);
                if result.is_ok() {
                    self.branch_after_commit.set(Some(visit));
                }
                result
            }
            ChromeIntent::OpenCitation(child) => self.engine.open_citation(child),
            ChromeIntent::Navigate { url, target } => self.engine.navigate(url, target),
            ChromeIntent::ReadCapture(id) => reader::show_capture(&ui, id),
            ChromeIntent::ReadCaptures(visit) => reader::show_captures(&ui, visit),
            ChromeIntent::SetCaptureEnabled(enabled) => {
                let result = self.engine.set_capture_enabled(enabled);
                self.schedule_refresh();
                result
            }
            ChromeIntent::RememberSelection => self.engine.remember_selection(),
        };
        self.report(result);
    }

    fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        self.browser.borrow().search(query)
    }

    fn preview(&self, visit: VisitId) -> Result<VisitPreview> {
        self.browser.borrow().preview(visit)
    }
}
