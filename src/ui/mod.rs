use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};

use anyhow::Result;
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

use crate::{
    browser::{
        Browser, ImportedPage, NavigationTarget, SearchHit, Snapshot, VisitId, VisitPreview,
    },
    chrome::{
        ChromeFrame, ChromeHost, ChromeIntent, Favicons, HtmlChrome, PaletteScope, PaletteSeed,
        Toast, CHROME_HEIGHT,
    },
    engine::{DownloadEvent, Engine, EngineEvent},
    forest::ForestProjection,
};

pub struct Ui {
    pub window: gtk::Window,
    pub engine: Engine,
    pub chrome: HtmlChrome,
    browser: Rc<RefCell<Browser>>,
    refresh_scheduled: Cell<bool>,
    /// A queued page activated by "New branch here", whose palette opens once it commits.
    branch_after_commit: Cell<Option<VisitId>>,
    path_after_commit: RefCell<Option<PathContinuation>>,
    me: Weak<Ui>,
    inbox: crate::cookies::Inbox,
    cookie_monitor: gio::FileMonitor,
    draining_cookies: Cell<bool>,
    cookies_pending: Cell<bool>,
    reload_after_cookies: Cell<bool>,
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
        // wry sizes its views with size requests; inside this wrapper they cannot pin the
        // window's minimum size, so a tiling compositor can still shrink it.
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::External)
            .build();
        let fixed = gtk::Fixed::new();
        scroller.add(&fixed);
        window.add(&scroller);
        let engine = Engine::new(&fixed, browser.clone(), &data_dir.join("profile"))?;
        let chrome = HtmlChrome::mount(&fixed, Favicons::new(&engine.native))?;
        use gio::prelude::FileExt;
        let inbox = crate::cookies::Inbox::new(data_dir);
        inbox.prepare()?;
        let cookie_monitor = gio::File::for_path(inbox.directory())
            .monitor_directory(gio::FileMonitorFlags::NONE, None::<&gio::Cancellable>)?;
        let ui = Rc::new_cyclic(|me: &Weak<Ui>| {
            let host: Weak<dyn ChromeHost> = me.clone();
            chrome.attach(host);
            Ui {
                window,
                engine,
                chrome,
                browser,
                refresh_scheduled: Cell::new(false),
                branch_after_commit: Cell::new(None),
                path_after_commit: RefCell::new(None),
                me: me.clone(),
                inbox,
                cookie_monitor,
                draining_cookies: Cell::new(false),
                cookies_pending: Cell::new(false),
                reload_after_cookies: Cell::new(false),
            }
        });
        let weak = Rc::downgrade(&ui);
        scroller.connect_size_allocate(move |_, allocation| {
            if let Some(ui) = weak.upgrade() {
                let (width, height) = (
                    f64::from(allocation.width()),
                    f64::from(allocation.height()),
                );
                ui.engine
                    .set_bounds(0., CHROME_HEIGHT, width, height - CHROME_HEIGHT);
                ui.chrome.resize(width, height);
                ui.chrome.raise();
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
                EngineEvent::Download(event) => ui.chrome.toast(download_toast(event)),
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_key_press_event(move |_, event| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |ui| ui.shortcut(event))
        });
        if let Some(settings) = gtk::Settings::default() {
            for property in ["gtk-application-prefer-dark-theme", "gtk-theme-name"] {
                let weak = Rc::downgrade(&ui);
                settings.connect_notify_local(Some(property), move |_, _| {
                    if let Some(ui) = weak.upgrade() {
                        ui.chrome.set_system_dark(prefers_dark());
                    }
                });
            }
        }
        let weak = Rc::downgrade(&ui);
        ui.cookie_monitor.connect_changed(move |_, _, _, _| {
            if let Some(ui) = weak.upgrade() {
                ui.drain_cookies();
            }
        });
        ui.chrome.set_system_dark(prefers_dark());
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
        self.reload_after_cookies.set(true);
        self.drain_cookies();
        Ok(())
    }

    fn drain_cookies(&self) {
        if self.draining_cookies.replace(true) {
            self.cookies_pending.set(true);
            return;
        }
        let batches = match self.inbox.collect() {
            Ok(batches) => batches,
            Err(_) => {
                self.draining_cookies.set(false);
                self.report_error("Could not read imported cookies.");
                if self.reload_after_cookies.replace(false) {
                    self.report(self.engine.reload());
                }
                return;
            }
        };
        let weak = self.me.clone();
        self.engine.apply_cookie_batches(batches, move |result| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.draining_cookies.set(false);
            match result {
                Ok(0) => {}
                Ok(count) => ui
                    .chrome
                    .toast(Toast::Info(format!("Added {count} cookies from Chrome."))),
                Err(_) => ui.report_error("Could not add imported cookies."),
            }
            if ui.cookies_pending.replace(false) {
                ui.drain_cookies();
            } else if ui.reload_after_cookies.replace(false) {
                ui.report(ui.engine.reload());
            }
        });
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
            ChromeIntent::ReadCapture(id) => ui.read_capture(id),
            ChromeIntent::ReadCaptures(visit) => ui.read_captures(visit),
            ChromeIntent::OpenSaved(id) => {
                let capture = self.browser.borrow().capture(id);
                capture.and_then(|capture| self.engine.open_saved(&capture))
            }
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

    fn search_imported(&self, query: &str) -> Result<Vec<ImportedPage>> {
        self.browser.borrow().search_imported(query, 24)
    }

    fn preview(&self, visit: VisitId) -> Result<VisitPreview> {
        self.browser.borrow().preview(visit)
    }

    fn focus_page(&self) {
        self.engine.focus();
        self.chrome.raise();
    }
}

impl Ui {
    fn read_capture(&self, id: i64) -> Result<()> {
        let browser = self.browser.borrow();
        let capture = browser.capture(id)?;
        let visit = browser.visit(capture.visit)?;
        drop(browser);
        self.chrome.read(&capture, &visit);
        Ok(())
    }

    fn read_captures(&self, visit: VisitId) -> Result<()> {
        let captures = self.browser.borrow().captures(visit)?;
        match captures.as_slice() {
            [] => anyhow::bail!(
                "No saved text for this page yet. Select some text and remember it, or turn on Remember pages."
            ),
            [only] => self.read_capture(only.id),
            several => {
                self.chrome.choose(several);
                Ok(())
            }
        }
    }
}

/// Whether the desktop prefers dark, which `Theme: System` follows.
fn prefers_dark() -> bool {
    gtk::Settings::default().is_some_and(|s| {
        s.is_gtk_application_prefer_dark_theme()
            || s.gtk_theme_name()
                .is_some_and(|name| name.to_lowercase().contains("dark"))
    })
}

fn download_toast(event: DownloadEvent) -> Toast {
    let name = |path: &Path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    match event {
        DownloadEvent::Started(name) => Toast::Info(format!("Downloading {name}…")),
        DownloadEvent::Saved(path) => Toast::Info(format!(
            "Saved {} in {}.",
            name(&path),
            path.parent().map(name).unwrap_or_default()
        )),
        DownloadEvent::Failed { name, reason } => Toast::Error(format!(
            "Could not download {}. {reason}",
            name.as_deref().unwrap_or("the file")
        )),
    }
}
