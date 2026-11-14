use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};

use anyhow::Result;
use gtk::prelude::*;
use url::Url;
use webkit2gtk::WebViewExt;

use crate::{
    browser::{
        Browser, Download, ImportedPage, Landing, NavigationTarget, SearchHit, Snapshot, VisitId,
        VisitPreview,
    },
    chrome::{
        ChromeFrame, ChromeHost, ChromeIntent, Favicons, HtmlChrome, PageFile, PaletteScope,
        PaletteSeed, Toast, CHROME_HEIGHT,
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
    /// Set from start until the first cookie drain ends, so no page loads without them.
    after_cookies: Cell<Option<StartPage>>,
}

#[derive(Clone, Copy)]
enum StartPage {
    Reload,
    Show(VisitId),
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
        {
            use webkit2gtk::{WebContextExt, WebViewExt};
            let confined = [engine.native.context(), chrome.native().context()]
                .into_iter()
                .all(|context| context.is_some_and(|context| context.is_sandbox_enabled()));
            if !confined {
                eprintln!(
                    "Canopy: WebKit's sandbox is off. Pages run with your full user permissions."
                );
            }
        }
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
                after_cookies: Cell::new(None),
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

    pub fn downloads(&self) -> Result<Vec<Download>> {
        self.browser.borrow().downloads()
    }

    /// Restores the forest, then opens `urls` as new trees in place of reloading the last page.
    pub fn resume(&self, urls: &[Url]) -> Result<()> {
        self.after_cookies.set(Some(StartPage::Reload));
        // A failed open is already on screen as an error, and the window still starts.
        let _ = self.open(urls);
        self.drain_cookies();
        Ok(())
    }

    /// Plants each URL as a new tree and shows the last one. A failure is shown in the
    /// window and returned, so the launch that sent the URLs can fail too.
    pub fn open(&self, urls: &[Url]) -> Result<()> {
        let shown = self.plant(urls);
        if let Err(error) = &shown {
            self.report_error(&error.to_string());
        }
        shown
    }

    fn plant(&self, urls: &[Url]) -> Result<()> {
        let roots = self.browser.borrow_mut().add_roots(urls)?;
        let Some(last) = roots.last().copied() else {
            return Ok(());
        };
        self.schedule_refresh();
        self.branch_after_commit.set(None);
        self.path_after_commit.borrow_mut().take();
        if self.after_cookies.get().is_some() {
            self.after_cookies.set(Some(StartPage::Show(last)));
            return Ok(());
        }
        self.engine.restore(last)
    }

    /// Raises the window, using the launching app's activation token when it passed one.
    pub fn bring_forward(&self, activation: Option<&str>) {
        if let Some(token) = activation {
            self.window.set_startup_id(token);
        }
        self.window.present();
    }

    fn show_start_page(&self) {
        let result = match self.after_cookies.take() {
            None => Ok(()),
            Some(StartPage::Reload) => self.engine.reload(),
            Some(StartPage::Show(visit)) => self.engine.restore(visit),
        };
        self.report(result);
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
                self.show_start_page();
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
            } else {
                ui.show_start_page();
            }
        });
    }

    fn read_frame(&self) -> Result<ChromeFrame> {
        let browser = self.browser.borrow();
        let forest = ForestProjection::from_snapshot(browser.snapshot()?, browser.tree_states()?)?;
        // Only the current tree is drawn with its files, so only those are looked for on disk.
        let tree = forest.current().and_then(|id| forest.tree_of(id));
        let files = browser
            .downloads()?
            .into_iter()
            .filter(|download| tree.is_some() && forest.tree_of(download.visit) == tree)
            .map(|download| PageFile {
                missing: !download.path.exists(),
                download,
            })
            .collect();
        Ok(ChromeFrame {
            forest,
            capture_enabled: browser.capture_enabled()?,
            theme: browser.theme_mode()?,
            suggestions: browser.start_suggestions()?,
            files,
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
            Shortcut::SettleTree => self.chrome.settle_current(),
            Shortcut::NewTree => self.chrome.new_tree(),
            Shortcut::Forest => self.chrome.toggle_forest(),
            Shortcut::Reload => self.report(self.engine.reload()),
            Shortcut::RememberSelection => self.report(self.engine.remember_selection()),
            Shortcut::Quit => self.window.close(),
            Shortcut::Tree(index) => {
                let tree = self.chrome.tab(index);
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Shortcut {
    Palette(PaletteSeed),
    NewTree,
    SettleTree,
    Forest,
    Reload,
    RememberSelection,
    Quit,
    Tree(usize),
    Back,
    ToggleFocus,
    Escape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Modifiers(gdk::ModifierType);

impl Modifiers {
    const NONE: Self = Self(gdk::ModifierType::empty());
    const CONTROL: Self = Self(gdk::ModifierType::CONTROL_MASK);
    const ALT: Self = Self(gdk::ModifierType::MOD1_MASK);
    const CONTROL_SHIFT: Self =
        Self(gdk::ModifierType::CONTROL_MASK.union(gdk::ModifierType::SHIFT_MASK));

    fn held(state: gdk::ModifierType) -> Self {
        use gdk::ModifierType as M;
        Self(
            state
                & (M::CONTROL_MASK
                    | M::MOD1_MASK
                    | M::SHIFT_MASK
                    | M::MOD3_MASK
                    | M::MOD4_MASK
                    | M::MOD5_MASK
                    | M::SUPER_MASK
                    | M::HYPER_MASK
                    | M::META_MASK),
        )
    }
}

impl Shortcut {
    fn of(event: &gdk::EventKey) -> Option<Self> {
        Self::matching(Modifiers::held(event.state()), event.keyval())
    }

    fn matching(modifiers: Modifiers, keyval: gdk::keys::Key) -> Option<Self> {
        use gdk::keys::constants as key;
        let keyval = keyval.to_lower();
        let bindings = [
            (
                Modifiers::CONTROL,
                key::k,
                Self::Palette(PaletteSeed::Empty),
            ),
            (
                Modifiers::CONTROL,
                key::l,
                Self::Palette(PaletteSeed::CurrentAddress),
            ),
            (Modifiers::CONTROL, key::t, Self::NewTree),
            (Modifiers::CONTROL, key::w, Self::SettleTree),
            (Modifiers::CONTROL, key::e, Self::Forest),
            (Modifiers::CONTROL, key::r, Self::Reload),
            (Modifiers::CONTROL_SHIFT, key::r, Self::Reload),
            (Modifiers::NONE, key::F5, Self::Reload),
            (Modifiers::CONTROL, key::s, Self::RememberSelection),
            (Modifiers::CONTROL, key::q, Self::Quit),
            (Modifiers::ALT, key::Left, Self::Back),
            (Modifiers::NONE, key::F6, Self::ToggleFocus),
            (Modifiers::NONE, key::Escape, Self::Escape),
        ];
        if let Some((_, _, action)) = bindings
            .into_iter()
            .find(|(held, key, _)| *held == modifiers && *key == keyval)
        {
            return Some(action);
        }
        // AZERTY produces digit keyvals with Shift held.
        if matches!(modifiers, Modifiers::CONTROL | Modifiers::CONTROL_SHIFT) {
            let digit = keyval.to_unicode()?.to_digit(10)?;
            if (1..=9).contains(&digit) {
                return Some(Self::Tree(digit as usize - 1));
            }
        }
        None
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
            ChromeIntent::RenameTree(tree, name) => {
                let result = self.browser.borrow_mut().rename_tree(tree, &name);
                self.schedule_refresh();
                result
            }
            ChromeIntent::SettleCurrent => {
                if let Some(tree) = self.chrome.current_tree() {
                    self.dispatch(ChromeIntent::SettleTree(tree));
                }
                Ok(())
            }
            ChromeIntent::SettleTree(tree) => {
                let landing = self.browser.borrow_mut().settle_tree(tree);
                self.schedule_refresh();
                landing.and_then(|landing| match landing {
                    Landing::Unchanged => Ok(()),
                    Landing::Restore(visit) => self.engine.restore(visit),
                    Landing::Start => {
                        self.engine.blank();
                        Ok(())
                    }
                })
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
            ChromeIntent::OpenFile(id) => ui.open_file(id, false),
            ChromeIntent::OpenFileFolder(id) => ui.open_file(id, true),
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

impl Ui {
    /// Hands a downloaded file, or the folder it was saved in, to the desktop's usual app.
    fn open_file(&self, id: i64, folder: bool) -> Result<()> {
        let download = self.browser.borrow().download(id)?;
        let name = download.name();
        let directory = download.path.parent().unwrap_or(Path::new("/"));
        let directory_name = directory
            .file_name()
            .map_or_else(|| "its folder".into(), |name| name.to_string_lossy());
        let target = if folder { directory } else { &download.path };
        if !target.exists() {
            // The strip may still draw the file as present.
            self.schedule_refresh();
            if folder {
                anyhow::bail!("{directory_name}, where {name} was saved, no longer exists.");
            }
            anyhow::bail!("{name} is no longer in {directory_name}. It was moved or deleted.");
        }
        let launch = self.window.display().app_launch_context();
        gio::AppInfo::launch_default_for_uri(&gio::File::for_path(target).uri(), launch.as_ref())
            .map_err(|error| {
                let shown = if folder { &*directory_name } else { &*name };
                anyhow::anyhow!("Could not open {shown}. {}", error.message())
            })
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

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    use gdk::{keys::constants as key, ModifierType as M};

    #[test]
    fn window_shortcuts_require_exact_modifiers() {
        let bindings = [
            (
                key::k,
                Modifiers::CONTROL,
                Shortcut::Palette(PaletteSeed::Empty),
            ),
            (
                key::l,
                Modifiers::CONTROL,
                Shortcut::Palette(PaletteSeed::CurrentAddress),
            ),
            (key::t, Modifiers::CONTROL, Shortcut::NewTree),
            (key::w, Modifiers::CONTROL, Shortcut::SettleTree),
            (key::e, Modifiers::CONTROL, Shortcut::Forest),
            (key::r, Modifiers::CONTROL, Shortcut::Reload),
            (key::r, Modifiers::CONTROL_SHIFT, Shortcut::Reload),
            (key::F5, Modifiers::NONE, Shortcut::Reload),
            (key::s, Modifiers::CONTROL, Shortcut::RememberSelection),
            (key::q, Modifiers::CONTROL, Shortcut::Quit),
            (key::Left, Modifiers::ALT, Shortcut::Back),
            (key::F6, Modifiers::NONE, Shortcut::ToggleFocus),
            (key::Escape, Modifiers::NONE, Shortcut::Escape),
        ];
        for (key, required, _) in bindings.iter().copied() {
            for bits in 0..8 {
                let state = if bits & 1 != 0 {
                    M::SHIFT_MASK
                } else {
                    M::empty()
                } | if bits & 2 != 0 {
                    M::CONTROL_MASK
                } else {
                    M::empty()
                } | if bits & 4 != 0 {
                    M::MOD1_MASK
                } else {
                    M::empty()
                };
                let held = Modifiers::held(state);
                let expected = bindings.iter().find_map(|(bound_key, modifiers, action)| {
                    (*bound_key == key && *modifiers == held).then_some(*action)
                });
                assert_eq!(Shortcut::matching(held, key), expected, "{state:?} {key:?}");
                assert_eq!(Shortcut::matching(held, key.to_upper()), expected);
                assert_eq!(
                    Shortcut::matching(Modifiers::held(state | M::LOCK_MASK | M::MOD2_MASK), key),
                    expected,
                    "lock modifiers must not affect {state:?} {key:?}"
                );
            }
            for extra in [
                M::MOD3_MASK,
                M::MOD4_MASK,
                M::MOD5_MASK,
                M::SUPER_MASK,
                M::HYPER_MASK,
                M::META_MASK,
            ] {
                assert_eq!(
                    Shortcut::matching(Modifiers::held(required.0 | extra), key),
                    None
                );
            }
        }
        assert_eq!(
            Shortcut::matching(Modifiers::CONTROL, key::K),
            Some(Shortcut::Palette(PaletteSeed::Empty))
        );
    }

    #[test]
    fn digit_tree_shortcuts_accept_shifted_azerty_keyvals() {
        let digits = [
            key::_1,
            key::_2,
            key::_3,
            key::_4,
            key::_5,
            key::_6,
            key::_7,
            key::_8,
            key::_9,
        ];
        for (index, key) in digits.into_iter().enumerate() {
            for modifiers in [Modifiers::CONTROL, Modifiers::CONTROL_SHIFT] {
                assert_eq!(
                    Shortcut::matching(modifiers, key),
                    Some(Shortcut::Tree(index))
                );
            }
            for state in [
                M::empty(),
                M::SHIFT_MASK,
                M::CONTROL_MASK | M::MOD1_MASK,
                M::CONTROL_MASK | M::SHIFT_MASK | M::MOD1_MASK,
            ] {
                assert_eq!(Shortcut::matching(Modifiers::held(state), key), None);
            }
        }
        for key in [
            key::_0,
            key::ampersand,
            key::eacute,
            key::quotedbl,
            key::apostrophe,
            key::parenleft,
            key::minus,
            key::egrave,
            key::underscore,
            key::ccedilla,
            key::F5,
            key::F11,
            key::f,
        ] {
            assert_eq!(Shortcut::matching(Modifiers::CONTROL, key), None);
        }
    }
}
