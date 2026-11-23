mod appearance;
mod canopy_layout;
mod logins;
mod mail_popup;
mod save_watch;

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

use anyhow::Result;
use gtk::prelude::*;
use url::Url;

use crate::{
    browser::{
        Browser, Download, ImportedPage, SearchHit, Snapshot, ThemeMode, VisitId, VisitPreview,
    },
    chrome::{
        ChromeFrame, ChromeHost, ChromeIntent, HtmlChrome, MailView, PageFile, PaletteScope,
        PaletteSeed, Toast,
    },
    forest::ForestProjection,
    logins::{Login, LoginProvider, SavedLogin},
    pages::{DownloadEvent, FindAction, FindTarget, Go, PageEvent, Pages},
    shortcuts::{KeyCombo, Shortcut, ShortcutSettings},
};

pub struct Ui {
    pub window: gtk::Window,
    pub pages: Rc<Pages>,
    pub chrome: HtmlChrome,
    pub mail: Rc<crate::mail::MailWatch>,
    /// The desktop's light or dark preference, which `Theme: System` follows.
    desktop: Rc<appearance::Desktop>,
    canopy_layout: canopy_layout::Setting,
    mail_popup: mail_popup::Setting,
    refresh_scheduled: Cell<bool>,
    shortcuts: ShortcutSettings,
    /// The pending target the chrome last drew, so a load that starts or ends redraws it.
    shown_pending: RefCell<Option<crate::browser::PendingTarget>>,
    me: Weak<Ui>,
    inbox: crate::cookies::Inbox,
    cookie_monitor: gio::FileMonitor,
    draining_cookies: Cell<bool>,
    cookies_pending: Cell<bool>,
    data_dir: PathBuf,
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
        let pages = Pages::mount(&fixed, browser, data_dir)?;
        let mail_origin = crate::mail::Origin::configured();
        let shortcuts = ShortcutSettings::defaults();
        let chrome =
            HtmlChrome::mount_with_mail(&fixed, &pages, mail_origin.clone(), data_dir, &shortcuts)?;
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
            let mail = crate::mail::MailWatch::start(pages.clone(), mail_origin, {
                let me = me.clone();
                move |signed_out| {
                    if let Some(ui) = me.upgrade() {
                        ui.mail_checked(&signed_out);
                    }
                }
            });
            let desktop = appearance::Desktop::watch({
                let me = me.clone();
                move || {
                    if let Some(ui) = me.upgrade() {
                        ui.desktop_appearance_changed();
                    }
                }
            });
            Ui {
                mail,
                desktop,
                canopy_layout: canopy_layout::Setting::load(data_dir),
                mail_popup: mail_popup::Setting::load(data_dir),
                window,
                pages,
                chrome,
                refresh_scheduled: Cell::new(false),
                shortcuts,
                shown_pending: RefCell::new(None),
                me: me.clone(),
                inbox,
                cookie_monitor,
                draining_cookies: Cell::new(false),
                cookies_pending: Cell::new(false),
                data_dir: data_dir.to_owned(),
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
                PageEvent::BranchMoved => {
                    ui.chrome.toast(Toast::Info("Moved branch".into()));
                    ui.schedule_refresh();
                }
                PageEvent::Changed | PageEvent::Loading(false) => ui.schedule_refresh(),
                PageEvent::Loading(true) | PageEvent::BlockingStoreReady => {}
                PageEvent::LoadState(value) => {
                    if ui.pages.pending_target() != *ui.shown_pending.borrow() {
                        ui.schedule_refresh();
                    }
                    ui.chrome.loading(value);
                }
                PageEvent::Queued(child) => {
                    ui.refresh();
                    ui.chrome.queued(child);
                }
                PageEvent::Waiting { parent, label } => ui.chrome.waiting(parent, &label),
                PageEvent::Saved(message) => ui.chrome.toast(Toast::Info(message)),
                PageEvent::Deleted { id, text } => {
                    if let Ok(forest) = ui
                        .pages
                        .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))
                    {
                        ui.chrome.rerank_suggestions(&forest);
                    }
                    ui.chrome.deleted(id, text);
                    ui.refresh();
                    ui.schedule_refresh();
                }
                PageEvent::Settled(message) => ui.chrome.settled(&message),
                PageEvent::Error(message) | PageEvent::BlockingError(message) => {
                    ui.chrome.toast(Toast::Error(message));
                }
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
                PageEvent::Fullscreen(on) => ui.chrome.set_fullscreen(on),
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_key_press_event(move |_, event| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |ui| ui.shortcut(event))
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_key_release_event(move |_, event| {
            use gdk::keys::constants as key;
            if let Some(ui) = weak.upgrade() {
                if matches!(event.keyval(), key::Control_L | key::Control_R) {
                    ui.chrome.finish_switcher(true);
                }
            }
            // Page scale also listens for Control's release on this window.
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_focus_out_event(move |_, _| {
            if let Some(ui) = weak.upgrade() {
                ui.chrome.finish_switcher(false);
            }
            glib::Propagation::Proceed
        });
        if let Some(settings) = gtk::Settings::default() {
            // Not `gtk-application-prefer-dark-theme`: Canopy sets that one for its pages.
            let weak = Rc::downgrade(&ui);
            settings.connect_notify_local(Some("gtk-theme-name"), move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    ui.desktop_appearance_changed();
                }
            });
        }
        let weak = Rc::downgrade(&ui);
        ui.cookie_monitor.connect_changed(move |_, _, _, _| {
            if let Some(ui) = weak.upgrade() {
                ui.drain_cookies();
            }
        });
        ui.chrome.set_system_dark(ui.desktop.prefers_dark());
        ui.show_password_settings();
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

    /// Answers `canopy session`: the open trees as the window shows them, and with `text`
    /// the shown page's visible text, which is read from the page and so arrives later.
    pub fn answer_session(&self, text: bool, reply: crate::instance::Reply) {
        let session = self.pages.read(|browser| {
            let forest =
                ForestProjection::from_snapshot(browser.snapshot()?, browser.tree_states()?)?;
            Ok::<_, anyhow::Error>(crate::session::describe(&crate::session::Seen {
                forest: &forest,
                live: &self.pages.hidden_visits(),
                loaded: &self.pages.loaded_visits(),
                pending: self.pages.pending_target().as_ref(),
                loading: self.pages.selected_loading(),
            }))
        });
        let mut session = match session {
            Ok(session) if text && !session["shown"].is_null() => session,
            other => return reply.send(other),
        };
        self.pages.selected_text(200_000, move |text| {
            match text {
                Ok(text) => session["text"] = text.into(),
                Err(error) => session["text_error"] = format!("{error:#}").into(),
            }
            reply.send(Ok(session));
        });
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
        let back = self.pages.back_target();
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
            let mail_popup = self.mail_popup.get();
            Ok(ChromeFrame {
                shown: browser.showings()?,
                canopy_layout: self.canopy_layout.get(),
                mail: self.mail.mail(),
                mail_view: MailView::new(mail_popup.look.new_after(), mail_popup.grouping),
                loaded: self.pages.loaded_visits(),
                live: self.pages.hidden_visits(),
                playing: self.pages.playing_visits(),
                forest,
                capture_enabled: browser.capture_enabled()?,
                blocking: self.pages.blocking_view(),
                trees_help: browser.trees_help()?,
                theme: browser.theme_mode()?,
                declined: browser.declined()?,
                bookmarks: browser.bookmarks()?,
                files,
                back,
                forward,
                pending: self.pages.pending_target(),
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

    #[cfg(feature = "gui-smoke")]
    pub fn set_mail_now(&self, now_ms: i64) {
        self.mail_popup.set_now_ms(now_ms);
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
                self.shown_pending.replace(frame.pending.clone());
                if let Some(visit) = frame.forest.current().and_then(|id| frame.forest.visit(id)) {
                    self.mail.notice_page(&visit.url, &visit.title);
                }
                let dark = match frame.theme {
                    ThemeMode::Dark => true,
                    ThemeMode::Light => false,
                    ThemeMode::System => self.desktop.prefers_dark(),
                };
                appearance::show_pages(dark);
                self.chrome.present(frame);
            }
            Err(error) => self.report_error(&error.to_string()),
        }
    }

    fn desktop_appearance_changed(&self) {
        self.chrome.set_system_dark(self.desktop.prefers_dark());
        self.schedule_refresh();
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
        // The fullscreen page gets every key; the chrome its shortcuts drive is hidden.
        if self.pages.is_fullscreen() {
            return glib::Propagation::Proceed;
        }
        let combo = KeyCombo::from_event(event);
        let shortcut = self
            .shortcuts
            .bindings
            .resolve(combo)
            .filter(|key| *key != Shortcut::PastePlainText || self.pages.has_focus());
        if self.chrome.switcher_active() {
            use gdk::keys::constants as key;
            let value = event.keyval();
            let stepped = match (shortcut, value) {
                (Some(Shortcut::Switch), _) => self.chrome.step_switcher(false, true),
                (Some(Shortcut::PreviousTree), _) => self.chrome.step_switcher(true, true),
                (_, key::Left) => self.chrome.step_switcher(true, false),
                (_, key::Right) => self.chrome.step_switcher(false, false),
                (_, key::Delete) => self.chrome.dismiss_switcher_selection(),
                _ => false,
            };
            if stepped {
                return glib::Propagation::Stop;
            }
            if !is_modifier_key(&value) {
                self.chrome.finish_switcher(false);
                if value == key::Escape {
                    return glib::Propagation::Stop;
                }
            }
        }
        let Some(shortcut) = shortcut else {
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
        if matches!(shortcut, Shortcut::PreviousSibling | Shortcut::NextSibling)
            && self.chrome.forest_open()
        {
            return glib::Propagation::Proceed;
        }
        self.pages.cancel_path_continuation();
        match shortcut {
            Shortcut::Palette(seed) => self.open_palette(seed),
            Shortcut::SettleTree => self.chrome.settle_current(),
            Shortcut::UndoSettle => self.report(self.pages.undo_settle()),
            Shortcut::Bookmark => self.toggle_bookmark(),
            Shortcut::NewTree => self.chrome.new_tree(),
            Shortcut::Forest(view) => self.chrome.toggle_forest(view),
            Shortcut::ZoomTree => self.chrome.toggle_tree(),
            Shortcut::MoveTree { next } => {
                self.report(self.pages.move_current_tree(next));
                self.schedule_refresh();
            }
            Shortcut::PreviousTree => match self.pages.read(|b| b.previous_tree()) {
                Ok(Some(tree)) => self.dispatch(ChromeIntent::SelectTree(tree)),
                Ok(None) => {}
                Err(error) => self.report(Err(error)),
            },
            Shortcut::Switch => self.report(self.chrome.arm_switcher(
                &self.pages,
                &self.window,
                combo.modifiers(),
            )),
            Shortcut::PageScale(action) => self.report(self.pages.page_scale(action)),
            Shortcut::Reload => {
                self.chrome.uncover();
                self.report(self.pages.reload());
            }
            Shortcut::RememberSelection => self.report(self.pages.remember_selection()),
            Shortcut::FillLogin => self.chrome.fill_login(),
            Shortcut::PastePlainText => self.pages.paste_plain_text(),
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

/// Modifier keys on their own. `EventKey::is_modifier` is not set on every backend.
fn is_modifier_key(keyval: &gdk::keys::Key) -> bool {
    use gdk::keys::constants as key;
    [
        key::Shift_L,
        key::Shift_R,
        key::Control_L,
        key::Control_R,
        key::Alt_L,
        key::Alt_R,
        key::Meta_L,
        key::Meta_R,
        key::Super_L,
        key::Super_R,
        key::Hyper_L,
        key::Hyper_R,
        key::ISO_Level3_Shift,
        key::Caps_Lock,
        key::Num_Lock,
    ]
    .contains(keyval)
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
        match intent {
            ChromeIntent::Mail(ask) => {
                self.mail.ask(ask);
                return;
            }
            ChromeIntent::MailOpened => {
                if let Err(error) = self.mail_popup.open() {
                    self.report_error(&error.to_string());
                }
                self.mail.check_soon();
                self.schedule_refresh();
                return;
            }
            ChromeIntent::SetMailGrouping(grouping) => {
                if let Err(error) = self.mail_popup.set_grouping(grouping) {
                    self.report_error(&error.to_string());
                }
                self.schedule_refresh();
                return;
            }
            _ => {}
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
        // The chrome waits for the next frame to learn whether a page it asked for opens a
        // new tree, so every navigation answers with one.
        let navigates = matches!(
            intent,
            ChromeIntent::Navigate { .. }
                | ChromeIntent::OpenPath { .. }
                | ChromeIntent::Restore(_)
                | ChromeIntent::SelectTree(_)
                | ChromeIntent::OpenCitation(_)
                | ChromeIntent::OpenSaved(_)
                | ChromeIntent::BranchFromQueued(_)
        );
        if navigates {
            self.schedule_refresh();
        }
        let result = match intent {
            ChromeIntent::AnswerFormReturn { .. }
            | ChromeIntent::Mail(_)
            | ChromeIntent::MailOpened
            | ChromeIntent::SetMailGrouping(_) => unreachable!(),
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
            ChromeIntent::ToggleBookmark => {
                self.toggle_bookmark();
                Ok(())
            }
            ChromeIntent::OpenFind => {
                self.pages.find(FindTarget::Selected, FindAction::Open);
                Ok(())
            }
            ChromeIntent::Quit => {
                self.window.close();
                Ok(())
            }
            ChromeIntent::ShowTreesHelp(show) => {
                let result = self.pages.set_trees_help(show);
                self.schedule_refresh();
                if show {
                    self.chrome.show_trees_help();
                }
                result
            }
            ChromeIntent::SetCanopyLayout(layout) => {
                let result = self.canopy_layout.save(layout);
                self.schedule_refresh();
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
            ChromeIntent::PlaceTree { tree, to } => {
                let result = self.pages.place_tree(tree, to);
                self.schedule_refresh();
                result
            }
            ChromeIntent::DeleteTrees(trees) => self.pages.delete_trees(&trees),
            ChromeIntent::DeleteBranch(head) => self.pages.delete_branch(head),
            ChromeIntent::UndoDelete(id) => self.pages.undo_delete(id),
            ChromeIntent::ForgetDeleted => {
                self.pages.forget_deleted();
                Ok(())
            }
            ChromeIntent::RenameTree(tree, name) => {
                let result = self.pages.rename_tree(tree, &name);
                self.schedule_refresh();
                result
            }
            ChromeIntent::SettleCurrent => {
                if let Some(tree) = self.chrome.current_tree() {
                    self.dispatch(ChromeIntent::PlaceTree {
                        tree,
                        to: crate::browser::Move::History,
                    });
                }
                Ok(())
            }
            ChromeIntent::Dismiss(head) => self
                .pages
                .dismiss_with(head, self.chrome.dismiss_completion(head)),
            ChromeIntent::SettleBranch(head) => self.pages.settle_branch(head),
            ChromeIntent::MoveBranch { head, to } => self.pages.move_branch(head, to),
            ChromeIntent::UndoSettle => self.pages.undo_settle(),
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
            ChromeIntent::SetBlocking(on) => self.pages.set_blocking(on),
            ChromeIntent::SetSiteBlocking { document, on } => {
                let result = self.pages.set_site_blocking(document, on);
                self.schedule_refresh();
                result
            }
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
            ChromeIntent::SetPasswordProvider(choice) => ui.choose_password_provider(choice),
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
        let (capture, visit, forest) = self.pages.read(|browser| -> Result<_> {
            let capture = browser.capture(id)?;
            let visit = browser.visit(capture.visit)?;
            Ok((
                capture,
                visit,
                ForestProjection::from_snapshot(browser.snapshot()?, browser.tree_states()?)?,
            ))
        })?;
        self.chrome.read(&capture, &visit, &forest);
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
