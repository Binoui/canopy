mod logins;
mod save_watch;

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};

use anyhow::Result;
use gtk::prelude::*;
use url::Url;

use crate::{
    browser::{Browser, Download, ImportedPage, SearchHit, Snapshot, VisitId, VisitPreview},
    chrome::{
        ChromeFrame, ChromeHost, ChromeIntent, HtmlChrome, PageFile, PaletteScope, PaletteSeed,
        Toast,
    },
    forest::ForestProjection,
    logins::{Login, LoginProvider, SavedLogin},
    pages::{DownloadEvent, FindAction, FindTarget, Go, PageEvent, PageScaleAction, Pages},
};

pub struct Ui {
    pub window: gtk::Window,
    pub pages: Rc<Pages>,
    pub chrome: HtmlChrome,
    pub mail: Rc<crate::mail::MailWatch>,
    refresh_scheduled: Cell<bool>,
    me: Weak<Ui>,
    inbox: crate::cookies::Inbox,
    cookie_monitor: gio::FileMonitor,
    draining_cookies: Cell<bool>,
    cookies_pending: Cell<bool>,
    /// Where saved logins are kept. `None` when the vault could not be opened.
    logins: RefCell<Option<Rc<dyn LoginProvider>>>,
    /// The page on screen that shows sign-in fields, and the number of the lookup made
    /// for it, so an answer that arrives for an earlier page is dropped.
    login_page: RefCell<Option<Url>>,
    lookups: Cell<u64>,
    /// The provider asked to be unlocked before it answers for this page.
    logins_locked: Cell<bool>,
    /// The accounts offered for the page on screen, and the number of that offer.
    offered: RefCell<(u64, Vec<SavedLogin>)>,
    /// The login the user last sent to a page, until the page shows whether it was accepted.
    save_watch: RefCell<save_watch::SaveWatch>,
    /// The login offered for saving, and the number of that offer.
    saving: RefCell<Option<(u64, Login)>>,
    save_offers: Cell<u64>,
}

impl Ui {
    pub fn new(data_dir: &Path) -> Result<Rc<Self>> {
        let browser = Browser::open(&data_dir.join("forest.sqlite3"))?;
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Canopy");
        // Found in the icon theme once installed; Wayland takes the icon from the desktop entry.
        window.set_icon_name(Some("canopy"));
        window.set_default_size(1240, 800);
        window.set_widget_name("canopy");
        // wry sizes its views with size requests; inside this wrapper they cannot pin the
        // window's minimum size, so a tiling compositor can still shrink it.
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::External)
            .build();
        let fixed = crate::pages::stack::Stack::fixed();
        scroller.add(&fixed);
        window.add(&scroller);
        let pages = Pages::mount(&fixed, browser, &data_dir.join("profile"))?;
        let chrome = HtmlChrome::mount(&fixed, pages.favicons())?;
        pages.set_chrome(&chrome.native());
        {
            use webkit2gtk::{WebContextExt, WebViewExt};
            let confined = pages.sandbox_enabled()
                && chrome
                    .native()
                    .context()
                    .is_some_and(|c| c.is_sandbox_enabled());
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
        let (logins, logins_problem) = logins::provider(data_dir);
        let ui = Rc::new_cyclic(|me: &Weak<Ui>| {
            let host: Weak<dyn ChromeHost> = me.clone();
            chrome.attach(host);
            let mail =
                crate::mail::MailWatch::start(pages.clone(), crate::mail::Origin::configured(), {
                    let me = me.clone();
                    move |signed_out| {
                        if let Some(ui) = me.upgrade() {
                            ui.mail_checked(&signed_out);
                        }
                    }
                });
            Ui {
                mail,
                window,
                pages,
                chrome,
                refresh_scheduled: Cell::new(false),
                me: me.clone(),
                inbox,
                cookie_monitor,
                draining_cookies: Cell::new(false),
                cookies_pending: Cell::new(false),
                logins: RefCell::new(logins),
                login_page: RefCell::new(None),
                lookups: Cell::new(0),
                logins_locked: Cell::new(false),
                offered: RefCell::default(),
                save_watch: RefCell::default(),
                saving: RefCell::new(None),
                save_offers: Cell::new(0),
            }
        });
        let mail = Rc::downgrade(&ui.mail);
        ui.window.connect_destroy(move |_| {
            if let Some(mail) = mail.upgrade() {
                mail.stop();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_delete_event(move |_, _| {
            if let Some(ui) = weak.upgrade() {
                ui.pages.answer_form_return(false);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&ui);
        scroller.connect_size_allocate(move |_, allocation| {
            if let Some(ui) = weak.upgrade() {
                let (width, height) = (
                    f64::from(allocation.width()),
                    f64::from(allocation.height()),
                );
                ui.pages.resize(width, height);
                ui.chrome.resize(width, height);
                ui.chrome.raise();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.pages.observe(move |event| {
            let Some(ui) = weak.upgrade() else { return };
            ui.chrome.raise();
            match event {
                PageEvent::FormReturn(open) => ui.chrome.form_return(open),
                PageEvent::PageScale(value) => ui.chrome.page_scale(value),
                PageEvent::Selected => ui.refresh(),
                PageEvent::Changed | PageEvent::Loading(false) => ui.schedule_refresh(),
                PageEvent::Loading(true) => {}
                PageEvent::LoadState(value) => ui.chrome.loading(value),
                PageEvent::Waiting { parent, label } => {
                    // The pulse must land on the new count, including the first After item.
                    ui.refresh();
                    ui.chrome.waiting(parent, &label);
                }
                PageEvent::Settled(message) => ui.chrome.settled(&message),
                PageEvent::Error(message) => ui.chrome.toast(Toast::Error(message)),
                PageEvent::Remembered(_) => ui
                    .chrome
                    .toast(Toast::Info("Selection remembered on this computer.".into())),
                PageEvent::Download(event) => ui.chrome.toast(download_toast(event)),
                PageEvent::Login(event) => ui.login_event(event),
                PageEvent::OpenBranchPalette { visit } => {
                    ui.refresh();
                    ui.chrome
                        .open_palette(PaletteScope::From(visit), PaletteSeed::Empty);
                }
                PageEvent::Find(update) => ui.chrome.find(update),
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
        if let Some(problem) = logins_problem {
            ui.report_error(&problem);
        }
        Ok(ui)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        self.pages.read(|b| b.snapshot())
    }

    pub fn search_hits(&self, query: &str) -> Result<Vec<SearchHit>> {
        self.pages.read(|b| b.search(query))
    }

    pub fn downloads(&self) -> Result<Vec<Download>> {
        self.pages.read(|b| b.downloads())
    }

    /// Restores the forest, then opens `urls` as new trees in place of reloading the last page.
    pub fn resume(&self, urls: &[Url]) -> Result<()> {
        self.pages.prepare_resume(urls)?;
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
        self.pages.open_roots(urls)
    }

    /// Raises the window, using the launching app's activation token when it passed one.
    pub fn bring_forward(&self, activation: Option<&str>) {
        if let Some(token) = activation {
            self.window.set_startup_id(token);
        }
        self.window.present();
    }

    fn show_start_page(&self) {
        let result = self.pages.resume();
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
        self.pages.apply_cookie_batches(batches, move |result| {
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
        self.pages.read(|browser| {
            let forest =
                ForestProjection::from_snapshot(browser.snapshot()?, browser.tree_states()?)?;
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
            let forward = match forest.current() {
                Some(current) => browser.forward(current)?,
                None => None,
            };
            Ok(ChromeFrame {
                mail: self.mail.mail(),
                loaded: self.pages.loaded_visits(),
                live: self.pages.hidden_visits(),
                forest,
                capture_enabled: browser.capture_enabled()?,
                trees_help: browser.trees_help()?,
                theme: browser.theme_mode()?,
                declined: browser.declined()?,
                bookmarks: browser.bookmarks()?,
                files,
                forward,
            })
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

    fn mail_checked(&self, signed_out: &[String]) {
        if !signed_out.is_empty() {
            self.chrome.toast(Toast::Info(format!(
                "Signed out of Gmail as {}.",
                signed_out.join(", ")
            )));
        }
        self.schedule_refresh();
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
                if let Some(visit) = frame.forest.current().and_then(|id| frame.forest.visit(id)) {
                    self.mail.notice_page(&visit.url, &visit.title);
                }
                self.chrome.present(frame);
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
        self.pages.answer_form_return(false);
        let current = self.snapshot().ok().and_then(|s| s.current);
        let scope = current.map_or(PaletteScope::NewTree, PaletteScope::From);
        self.chrome.open_palette(scope, seed);
    }

    fn toggle_bookmark(&self) {
        use crate::browser::BookmarkToggle;
        let toast = match self.pages.toggle_current_bookmark() {
            Ok(BookmarkToggle::Added) => Toast::Info("Bookmarked".into()),
            Ok(BookmarkToggle::Removed) => Toast::Info("Bookmark removed".into()),
            Ok(BookmarkToggle::NoPage) => Toast::Info("No page to bookmark".into()),
            Err(error) => Toast::Error(error.to_string()),
        };
        self.schedule_refresh();
        self.chrome.toast(toast);
    }

    fn shortcut(&self, event: &gdk::EventKey) -> glib::Propagation {
        let Some(shortcut) = Shortcut::of(event) else {
            return glib::Propagation::Proceed;
        };
        if !matches!(shortcut, Shortcut::Escape) && self.pages.has_form_return() {
            self.pages.answer_form_return(false);
            let weak = self.me.clone();
            let event = event.clone();
            self.pages.after_form_return_cancel(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.shortcut(&event);
                }
            });
            return glib::Propagation::Stop;
        }
        self.pages.cancel_path_continuation();
        match shortcut {
            Shortcut::Palette(seed) => self.open_palette(seed),
            Shortcut::SettleTree => self.chrome.settle_current(),
            Shortcut::UndoSettle => self.report(self.pages.undo_settle()),
            Shortcut::Bookmark => self.toggle_bookmark(),
            Shortcut::NewTree => self.chrome.new_tree(),
            Shortcut::Forest => self.chrome.toggle_forest(),
            Shortcut::ZoomTree => self.chrome.toggle_tree(),
            Shortcut::PreviousTree => match self.pages.read(|b| b.previous_tree()) {
                Ok(Some(tree)) => self.dispatch(ChromeIntent::SelectTree(tree)),
                Ok(None) => {}
                Err(error) => self.report(Err(error)),
            },
            Shortcut::NextWaiting => self.chrome.next_waiting(),
            Shortcut::PageScale(action) => self.report(self.pages.page_scale(action)),
            Shortcut::Reload => {
                self.chrome.uncover();
                self.report(self.pages.reload());
            }
            Shortcut::RememberSelection => self.report(self.pages.remember_selection()),
            Shortcut::FillLogin => self.chrome.fill_login(),
            Shortcut::Quit => self.window.close(),
            Shortcut::Tree(index) => {
                let tree = self.chrome.tab(index);
                if let Some(tree) = tree {
                    self.dispatch(ChromeIntent::SelectTree(tree));
                }
            }
            Shortcut::Forward => {
                self.chrome.uncover();
                self.report(self.pages.go(Go::Forward));
            }
            Shortcut::PreviousSibling => self.report(self.pages.go(Go::Sibling { next: false })),
            Shortcut::NextSibling => self.report(self.pages.go(Go::Sibling { next: true })),
            Shortcut::Back => {
                self.chrome.uncover();
                self.report(self.pages.go(Go::Parent));
            }
            Shortcut::TreesHelp => self.dispatch(ChromeIntent::ShowTreesHelp(true)),
            Shortcut::ToggleFocus => self.chrome.toggle_focus(),
            Shortcut::Find => self.pages.find(FindTarget::Selected, FindAction::Open),
            Shortcut::FindNext => self.pages.find(FindTarget::Selected, FindAction::Next),
            Shortcut::FindPrevious => self.pages.find(FindTarget::Selected, FindAction::Previous),
            Shortcut::Escape if self.chrome.escape() => {}
            Shortcut::Escape if self.pages.stop() => {}
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
    Bookmark,
    SettleTree,
    UndoSettle,
    Forest,
    ZoomTree,
    NextWaiting,
    PreviousTree,
    PageScale(PageScaleAction),
    Reload,
    RememberSelection,
    FillLogin,
    Quit,
    Tree(usize),
    Back,
    Forward,
    PreviousSibling,
    NextSibling,
    ToggleFocus,
    Find,
    FindNext,
    FindPrevious,
    TreesHelp,
    Escape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Modifiers(gdk::ModifierType);

impl Modifiers {
    const NONE: Self = Self(gdk::ModifierType::empty());
    const CONTROL: Self = Self(gdk::ModifierType::CONTROL_MASK);
    const ALT: Self = Self(gdk::ModifierType::MOD1_MASK);
    const SHIFT: Self = Self(gdk::ModifierType::SHIFT_MASK);
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
                key::equal,
                Self::PageScale(PageScaleAction::Increase),
            ),
            (
                Modifiers::CONTROL,
                key::plus,
                Self::PageScale(PageScaleAction::Increase),
            ),
            (
                Modifiers::CONTROL_SHIFT,
                key::plus,
                Self::PageScale(PageScaleAction::Increase),
            ),
            (
                Modifiers::CONTROL_SHIFT,
                key::equal,
                Self::PageScale(PageScaleAction::Increase),
            ),
            (
                Modifiers::CONTROL,
                key::KP_Equal,
                Self::PageScale(PageScaleAction::Increase),
            ),
            (
                Modifiers::CONTROL,
                key::KP_Add,
                Self::PageScale(PageScaleAction::Increase),
            ),
            (
                Modifiers::CONTROL_SHIFT,
                key::KP_Add,
                Self::PageScale(PageScaleAction::Increase),
            ),
            (
                Modifiers::CONTROL,
                key::minus,
                Self::PageScale(PageScaleAction::Decrease),
            ),
            (
                Modifiers::CONTROL,
                key::KP_Subtract,
                Self::PageScale(PageScaleAction::Decrease),
            ),
            (
                Modifiers::CONTROL,
                key::_0,
                Self::PageScale(PageScaleAction::Reset),
            ),
            (
                Modifiers::CONTROL,
                key::KP_0,
                Self::PageScale(PageScaleAction::Reset),
            ),
            (
                Modifiers::CONTROL_SHIFT,
                key::_0,
                Self::PageScale(PageScaleAction::Reset),
            ),
            (
                Modifiers::CONTROL,
                key::agrave,
                Self::PageScale(PageScaleAction::Reset),
            ),
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
            (Modifiers::CONTROL_SHIFT, key::t, Self::UndoSettle),
            (Modifiers::CONTROL, key::d, Self::Bookmark),
            (Modifiers::CONTROL, key::w, Self::SettleTree),
            (Modifiers::CONTROL, key::e, Self::Forest),
            (Modifiers::CONTROL_SHIFT, key::e, Self::ZoomTree),
            (Modifiers::CONTROL, key::Tab, Self::NextWaiting),
            (Modifiers::CONTROL_SHIFT, key::Tab, Self::PreviousTree),
            (
                Modifiers::CONTROL_SHIFT,
                key::ISO_Left_Tab,
                Self::PreviousTree,
            ),
            (Modifiers::CONTROL, key::r, Self::Reload),
            (Modifiers::CONTROL_SHIFT, key::r, Self::Reload),
            (Modifiers::NONE, key::F5, Self::Reload),
            (Modifiers::CONTROL, key::s, Self::RememberSelection),
            (Modifiers::CONTROL_SHIFT, key::l, Self::FillLogin),
            (Modifiers::CONTROL, key::q, Self::Quit),
            (Modifiers::ALT, key::Left, Self::Back),
            (Modifiers::ALT, key::Right, Self::Forward),
            (Modifiers::ALT, key::Up, Self::PreviousSibling),
            (Modifiers::ALT, key::Down, Self::NextSibling),
            (Modifiers::NONE, key::F1, Self::TreesHelp),
            (Modifiers::NONE, key::F6, Self::ToggleFocus),
            (Modifiers::CONTROL, key::f, Self::Find),
            (Modifiers::CONTROL, key::g, Self::FindNext),
            (Modifiers::CONTROL_SHIFT, key::g, Self::FindPrevious),
            (Modifiers::NONE, key::F3, Self::FindNext),
            (Modifiers::SHIFT, key::F3, Self::FindPrevious),
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
    fn suggestions(&self, forest: &ForestProjection) -> Result<Vec<crate::suggest::Suggestion>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        self.pages.read(|b| {
            let bookmarks = b.bookmarks()?;
            let declined = b.declined()?;
            let arrivals = b.arrivals(forest, now - crate::suggest::WINDOW)?;
            let chrome = b.chrome_favorites(3 + bookmarks.len() + declined.len())?;
            let left = forest
                .current()
                .and_then(|v| forest.tree_of(v))
                .or(b.last_event_tree(forest)?);
            Ok(crate::suggest::rank(
                &crate::suggest::Inputs {
                    forest,
                    bookmarks: &bookmarks,
                    declined: &declined,
                    arrivals: &arrivals,
                    chrome: &chrome,
                    left,
                },
                now,
            ))
        })
    }

    fn dispatch(&self, intent: ChromeIntent) {
        let Some(ui) = self.me.upgrade() else { return };
        if intent == ChromeIntent::CheckMail {
            self.mail.check_soon();
            return;
        }
        if let ChromeIntent::AnswerFormReturn { question, proceed } = intent {
            self.pages.answer_form_return_for(question, proceed);
            return;
        }
        if self.pages.has_form_return() {
            self.pages.answer_form_return(false);
            self.pages
                .after_form_return_cancel(move || ui.dispatch(intent));
            return;
        }
        self.pages.cancel_continuation();
        if matches!(
            intent,
            ChromeIntent::Restore(_)
                | ChromeIntent::Back
                | ChromeIntent::Forward
                | ChromeIntent::SelectTree(_)
                | ChromeIntent::OpenCitation(_)
                | ChromeIntent::OpenSaved(_)
        ) {
            self.chrome.uncover();
        }
        let result = match intent {
            ChromeIntent::AnswerFormReturn { .. } | ChromeIntent::CheckMail => unreachable!(),
            ChromeIntent::CancelContinuation => Ok(()),
            ChromeIntent::OpenPath { first, second } => self.pages.open_path(first, second),
            ChromeIntent::Bookmark { url, title } => {
                let result = self.pages.bookmark(&url, &title);
                self.schedule_refresh();
                result
            }
            ChromeIntent::Decline(key) => {
                let result = self.pages.decline(&key);
                self.schedule_refresh();
                result
            }
            ChromeIntent::ShowTreesHelp(show) => {
                let result = self.pages.set_trees_help(show);
                self.schedule_refresh();
                if show {
                    self.chrome.show_trees_help();
                }
                result
            }
            ChromeIntent::SetTheme(mode) => {
                let result = self.pages.set_theme(mode);
                self.schedule_refresh();
                result
            }
            ChromeIntent::RemoveBookmark(id) => {
                let result = self.pages.remove_bookmark(id);
                self.schedule_refresh();
                result
            }
            ChromeIntent::PinTree { tree, pinned } => {
                let result = self.pages.set_pinned(tree, pinned);
                self.schedule_refresh();
                result
            }
            ChromeIntent::RenameTree(tree, name) => {
                let result = self.pages.rename_tree(tree, &name);
                self.schedule_refresh();
                result
            }
            ChromeIntent::SettleCurrent => {
                if let Some(tree) = self.chrome.current_tree() {
                    self.dispatch(ChromeIntent::SettleTree(tree));
                }
                Ok(())
            }
            ChromeIntent::Dismiss(head) => self.pages.dismiss(head),
            ChromeIntent::SettleBranch(head) => self.pages.settle_branch(head),
            ChromeIntent::MoveToTree(head) => self.pages.move_to_tree(head),
            ChromeIntent::UndoSettle => self.pages.undo_settle(),
            ChromeIntent::SettleTree(tree) => self.pages.settle_tree(tree),
            ChromeIntent::SelectTree(tree) => self.pages.go(Go::Tree(tree)),
            ChromeIntent::Restore(visit) => self.pages.go(Go::Visit(visit)),
            ChromeIntent::PageScale(action) => self.pages.page_scale(action),
            ChromeIntent::Reload => self.pages.reload(),
            ChromeIntent::Back => self.pages.go(Go::Parent),
            ChromeIntent::Forward => self.pages.go(Go::Forward),
            ChromeIntent::Stop => {
                self.pages.stop();
                Ok(())
            }
            ChromeIntent::BranchFromQueued(visit) => self.pages.branch_from_queued(visit),
            ChromeIntent::OpenCitation(child) => self.pages.go(Go::Citation(child)),
            ChromeIntent::Navigate { url, target } => self.pages.go(Go::Address { url, target }),
            ChromeIntent::ReadCapture(id) => ui.read_capture(id),
            ChromeIntent::ReadCaptures(visit) => ui.read_captures(visit),
            ChromeIntent::OpenSaved(id) => self.pages.go(Go::Saved(id)),
            ChromeIntent::SetCaptureEnabled(enabled) => {
                let result = self.pages.set_capture_enabled(enabled);
                self.schedule_refresh();
                result
            }
            ChromeIntent::RememberSelection => self.pages.remember_selection(),
            ChromeIntent::OpenFile(id) => ui.open_file(id, false),
            ChromeIntent::OpenFileFolder(id) => ui.open_file(id, true),
            ChromeIntent::FillLogin { offer, index } => ui.fill_login(offer, index),
            ChromeIntent::AnswerSave { offer, save } => ui.answer_save(offer, save),
            ChromeIntent::UnlockLogins => {
                ui.unlock_logins();
                Ok(())
            }
            ChromeIntent::Find { session, action } => {
                self.pages.find(FindTarget::Session(session), action);
                Ok(())
            }
        };
        self.report(result);
    }

    fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        self.pages.read(|b| b.search(query))
    }

    fn search_imported(&self, query: &str) -> Result<Vec<ImportedPage>> {
        self.pages.read(|b| b.search_imported(query, 24))
    }

    fn preview(&self, visit: VisitId) -> Result<VisitPreview> {
        self.pages.read(|b| b.preview(visit))
    }

    fn focus_page(&self) {
        self.pages.focus();
        self.chrome.raise();
    }
}

impl Ui {
    fn read_capture(&self, id: i64) -> Result<()> {
        let (capture, visit) = self.pages.read(|browser| -> Result<_> {
            let capture = browser.capture(id)?;
            let visit = browser.visit(capture.visit)?;
            Ok((capture, visit))
        })?;
        self.chrome.read(&capture, &visit);
        Ok(())
    }

    fn read_captures(&self, visit: VisitId) -> Result<()> {
        let captures = self.pages.read(|b| b.captures(visit))?;
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
        let download = self.pages.read(|b| b.download(id))?;
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
                key::equal,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::plus,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::plus,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::equal,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::KP_Equal,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::KP_Add,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::KP_Add,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::minus,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Decrease),
            ),
            (
                key::KP_Subtract,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Decrease),
            ),
            (
                key::_0,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
            (
                key::KP_0,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
            (
                key::_0,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
            (
                key::agrave,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
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
            (key::t, Modifiers::CONTROL_SHIFT, Shortcut::UndoSettle),
            (key::d, Modifiers::CONTROL, Shortcut::Bookmark),
            (key::w, Modifiers::CONTROL, Shortcut::SettleTree),
            (key::e, Modifiers::CONTROL, Shortcut::Forest),
            (key::e, Modifiers::CONTROL_SHIFT, Shortcut::ZoomTree),
            (key::Tab, Modifiers::CONTROL, Shortcut::NextWaiting),
            (key::Tab, Modifiers::CONTROL_SHIFT, Shortcut::PreviousTree),
            (
                key::ISO_Left_Tab,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PreviousTree,
            ),
            (key::r, Modifiers::CONTROL, Shortcut::Reload),
            (key::r, Modifiers::CONTROL_SHIFT, Shortcut::Reload),
            (key::F5, Modifiers::NONE, Shortcut::Reload),
            (key::s, Modifiers::CONTROL, Shortcut::RememberSelection),
            (key::l, Modifiers::CONTROL_SHIFT, Shortcut::FillLogin),
            (key::q, Modifiers::CONTROL, Shortcut::Quit),
            (key::Left, Modifiers::ALT, Shortcut::Back),
            (key::Right, Modifiers::ALT, Shortcut::Forward),
            (key::Up, Modifiers::ALT, Shortcut::PreviousSibling),
            (key::Down, Modifiers::ALT, Shortcut::NextSibling),
            (key::F1, Modifiers::NONE, Shortcut::TreesHelp),
            (key::F6, Modifiers::NONE, Shortcut::ToggleFocus),
            (key::f, Modifiers::CONTROL, Shortcut::Find),
            (key::g, Modifiers::CONTROL, Shortcut::FindNext),
            (key::g, Modifiers::CONTROL_SHIFT, Shortcut::FindPrevious),
            (key::F3, Modifiers::NONE, Shortcut::FindNext),
            (key::F3, Modifiers::SHIFT, Shortcut::FindPrevious),
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
            key::ampersand,
            key::eacute,
            key::quotedbl,
            key::apostrophe,
            key::parenleft,
            key::egrave,
            key::underscore,
            key::ccedilla,
            key::F5,
            key::F11,
        ] {
            assert_eq!(Shortcut::matching(Modifiers::CONTROL, key), None);
        }
    }
}
