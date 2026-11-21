//! Tabs, the tree strip, the palette, attached lists, menus, the new-tree page, the forest
//! and the saved-text reader, drawn in HTML by a transparent web view over the page.
//!
//! Rust owns the state and every decision: layouts, palette results, intents. The view paints
//! what `view` serializes and reports input over IPC. At rest the view covers only the top
//! 94 px. It grows down to whatever is open over the page (an attached list or a menu)
//! and covers the window only for the palette, the reader and the full-page surfaces, so the
//! page keeps its clicks everywhere else.

mod days;
mod favicon;
mod feedback;
mod find;
mod forest_pane;
pub use forest_pane::View as ForestView;
pub(crate) mod graph;
mod loading;
mod page_scale;
mod palette;
mod pinch;
mod pins;
mod sender_logos;
mod start;
mod tabs;
mod tree_card;
mod tree_view;
mod view;
mod waiting;

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::Mutex,
};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use url::Url;
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    http::{header::CONTENT_TYPE, Request, Response},
    Rect, WebView, WebViewBuilder,
};

pub use favicon::Favicons;
use palette::{Group, PaletteItem};
use sender_logos::SenderLogos;

use crate::{
    browser::{
        Capture, Download, ImportedPage, MoveTo, NavigationTarget, SearchHit, TabPlacement,
        ThemeMode, TreeId, Visit, VisitId, VisitPreview,
    },
    forest::ForestProjection,
};

/// The tab row and the resting strip.
pub const CHROME_HEIGHT: f64 = 94.;

const INDEX: &str = include_str!("assets/index.html");
const CSS: &str = include_str!("assets/chrome.css");
const SCRIPT: &str = include_str!("assets/chrome.js");
const INTER: &[u8] = include_bytes!("../../assets/fonts/InterVariable.ttf");

/// The query and rows last sent to the view, and their default row.
type ShownPalette = (String, Vec<(Group, PaletteItem)>, usize);

pub use forest_pane::CanopyLayout;
pub use view::{Grouping, MailView};

/// Everything the chrome shows, read from SQLite in one go.
pub struct ChromeFrame {
    pub canopy_layout: CanopyLayout,
    pub shown: crate::browser::Showings,
    pub mail: crate::mail::Mail,
    pub mail_view: MailView,
    pub bookmarks: Vec<crate::browser::Bookmark>,
    pub forest: ForestProjection,
    /// Waiting pages that already have a live view. Nothing fills it yet: background
    /// loading will, and the chrome then shows those pages as loaded with no other change.
    pub loaded: std::collections::HashSet<VisitId>,
    /// Current documents retained in hidden, previously shown pages.
    pub live: std::collections::HashSet<VisitId>,
    /// Pages the owner can hear now. The chrome keeps each one in reach and marks it.
    pub playing: std::collections::HashSet<VisitId>,
    pub capture_enabled: bool,
    pub trees_help: bool,
    pub theme: ThemeMode,
    pub declined: std::collections::HashSet<crate::suggest::SuggestionKey>,
    /// The files downloaded from the current tree's pages, oldest first.
    pub files: Vec<PageFile>,
    /// The page Back opens: the previous native history entry in this tree, else the parent.
    pub back: Option<VisitId>,
    /// The child of the current page that Forward opens: the one shown most recently.
    pub forward: Option<VisitId>,
    /// Where the page on screen is headed before it commits. The chrome shows that page,
    /// or a new tree's tab, as soon as the load is asked for.
    pub pending: Option<crate::browser::PendingTarget>,
}

fn parse_move_to(to: &Value) -> Option<MoveTo> {
    let id = |key: &str| to[key].as_str()?.parse::<VisitId>().ok();
    Some(match to["kind"].as_str()? {
        "page" => MoveTo::Page(id("id")?),
        "tree" => MoveTo::Tree(TreeId(id("id")?)),
        "own-tree" => MoveTo::OwnTree(match to["place"]["kind"].as_str()? {
            "end" => TabPlacement::End,
            "before" => TabPlacement::Before(TreeId(to["place"]["tree"].as_str()?.parse().ok()?)),
            _ => return None,
        }),
        _ => return None,
    })
}

/// A file downloaded from a page, and whether it is still where it was saved.
pub struct PageFile {
    pub download: Download,
    pub missing: bool,
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
    Mail(crate::mail::MailAsk),
    MailOpened,
    SetMailGrouping(Grouping),
    PageScale(crate::pages::PageScaleAction),
    AnswerFormReturn {
        question: u64,
        proceed: bool,
    },
    Stop,
    SelectTree(TreeId),
    PlaceTree {
        tree: TreeId,
        to: crate::browser::Move,
    },
    DeleteTrees(Vec<TreeId>),
    /// Delete a page and every page opened from it; a root deletes its tree.
    DeleteBranch(VisitId),
    UndoDelete(crate::browser::DeleteId),
    ForgetDeleted,
    Dismiss(VisitId),
    SettleBranch(VisitId),
    MoveBranch {
        head: VisitId,
        to: MoveTo,
    },
    UndoSettle,
    SettleCurrent,
    /// Name a tree; an empty name goes back to the automatic one.
    RenameTree(TreeId, String),
    OpenPath {
        first: Url,
        second: Url,
    },
    Bookmark {
        url: Url,
        title: String,
    },
    Decline(crate::suggest::SuggestionKey),
    SetTheme(ThemeMode),
    SetCanopyLayout(CanopyLayout),
    ShowTreesHelp(bool),
    /// Bookmark the page on screen, or remove that bookmark.
    ToggleBookmark,
    /// Open find on the page on screen.
    OpenFind,
    Quit,
    CancelContinuation,
    Restore(VisitId),
    Reload,
    /// What Alt+Left does: back to the parent page.
    Back,
    /// What Alt+Right does: the child shown most recently.
    Forward,
    /// Activate a queued page, then open a palette anchored on it once it commits.
    BranchFromQueued(VisitId),
    Navigate {
        url: Url,
        target: NavigationTarget,
    },
    OpenCitation(VisitId),
    ReadCapture(i64),
    ReadCaptures(VisitId),
    /// Load the live page a saved text came from.
    OpenSaved(i64),
    SetCaptureEnabled(bool),
    RememberSelection,
    /// Open a downloaded file with the app the desktop uses for it.
    OpenFile(i64),
    /// Open the folder a downloaded file was saved in.
    OpenFileFolder(i64),
    /// Fill one of the saved logins of offer number `offer` into the page.
    FillLogin {
        offer: u64,
        index: usize,
    },
    /// Unlock the password manager that holds the logins.
    UnlockLogins,
    SetPasswordProvider(PasswordChoice),
    /// Answer save offer number `offer`: keep the login, or not now.
    AnswerSave {
        offer: u64,
        save: bool,
    },
    RemoveBookmark(crate::browser::BookmarkId),
    Find {
        session: crate::pages::FindId,
        action: crate::pages::FindAction,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PasswordChoice {
    Vault,
    Bitwarden,
}

/// A login waiting for the user to say whether to keep it.
pub struct SaveOffer {
    pub id: u64,
    /// The site's host, as shown to the user.
    pub site: String,
    pub username: String,
    /// The account is already saved with another password.
    pub update: bool,
    /// Where the login would be kept, as in "Kept in …".
    pub keeper: String,
}

pub enum Toast {
    Info(String),
    Error(String),
}

/// The side of the browser the chrome talks to: commands, and read-only queries.
pub trait ChromeHost {
    fn dispatch(&self, intent: ChromeIntent);
    fn suggestions(&self, forest: &ForestProjection) -> Result<Vec<crate::suggest::Suggestion>>;
    fn search(&self, query: &str) -> Result<Vec<SearchHit>>;
    /// Pages from imported browser history.
    fn search_imported(&self, query: &str) -> Result<Vec<ImportedPage>>;
    fn preview(&self, visit: VisitId) -> Result<VisitPreview>;
    /// Gives the keyboard back to the page.
    fn focus_page(&self);
}

pub struct HtmlChrome {
    inner: Rc<Inner>,
}

const ESCAPE_SURFACES: [&str; 8] = [
    "formReturn",
    "reader",
    "palette",
    "popover",
    "list",
    "treeFilters",
    "overlay",
    "start",
];

struct Inner {
    view: WebView,
    container: gtk::Fixed,
    me: Weak<Inner>,
    error_view: RefCell<Option<feedback::ErrorView>>,
    system_dark: Cell<bool>,
    host: RefCell<Option<Weak<dyn ChromeHost>>>,
    _favicons: Rc<Favicons>,
    _logos: Option<Rc<SenderLogos>>,
    ready: Cell<bool>,
    /// Calls made before the view loaded, replayed once it reports ready.
    queue: RefCell<Vec<String>>,
    size: Cell<(f64, f64)>,
    /// How far down the view reaches, or `None` to cover the window.
    cover: Cell<Option<f64>>,
    toast_rects: RefCell<Vec<[i32; 4]>>,
    errors: RefCell<feedback::Errors>,
    waiting: RefCell<Option<(VisitId, std::time::Instant, usize)>>,
    /// Something is open that Escape closes.
    escape_surface: Cell<Option<&'static str>>,
    /// The find session the bar shows. Escape closes it even before the view reports.
    find: Cell<Option<crate::pages::FindId>>,
    frame: RefCell<Option<ChromeFrame>>,
    tab_room: Cell<f64>,
    suggested: RefCell<Vec<crate::suggest::Suggestion>>,
    start: Cell<bool>,
    forest: RefCell<forest_pane::ForestPane>,
    palette_scope: Cell<Option<VisitId>>,
    /// The last palette results, so Enter runs exactly the row the view showed.
    palette: RefCell<Option<ShownPalette>>,
    workbench: RefCell<tree_view::Workbench>,
    /// A page covers the window; the chrome and its toasts stay hidden.
    fullscreen: Cell<bool>,
    pinch: gtk::GestureZoom,
}

impl HtmlChrome {
    /// Adds the chrome view to `container`, above the page already in it.
    pub fn mount(container: &gtk::Fixed, favicons: Rc<Favicons>) -> Result<Self> {
        Self::mount_inner(container, favicons, None)
    }

    pub(crate) fn mount_with_mail(
        container: &gtk::Fixed,
        pages: &Rc<crate::pages::Pages>,
        origin: Option<crate::mail::Origin>,
        data: &std::path::Path,
    ) -> Result<Self> {
        let logos = origin.map(|origin| SenderLogos::new(origin, pages, data));
        Self::mount_inner(container, pages.favicons(), logos)
    }

    fn mount_inner(
        container: &gtk::Fixed,
        favicons: Rc<Favicons>,
        logos: Option<Rc<SenderLogos>>,
    ) -> Result<Self> {
        use wry::WebViewBuilderExtUnix;

        // Wry requires a Send + Sync handler, but WebKitGTK calls it on the GTK thread.
        let icons = Mutex::new(glib::thread_guard::ThreadGuard::new((
            Rc::downgrade(&favicons),
            logos.as_ref().map(Rc::downgrade),
        )));
        let slot: Rc<RefCell<Weak<Inner>>> = Rc::default();
        let messages = slot.clone();
        let view = WebViewBuilder::new()
            .with_transparent(true)
            .with_bounds(rect(0., 0., 1., CHROME_HEIGHT))
            .with_custom_protocol("canopy".into(), move |_, request| {
                let (favicons, logos) = icons.lock().unwrap().get_ref().clone();
                serve(&request, &favicons, logos.as_ref())
            })
            .with_ipc_handler(move |request: Request<String>| {
                let weak = messages.borrow().clone();
                let body = request.into_body();
                // Out of WebKit's callback, so handling may call back into the view.
                glib::idle_add_local_once(move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.message(&body);
                    }
                });
            })
            .with_url("canopy://localhost/index.html")
            .build_gtk(container)
            .context("Could not create the chrome view.")?;
        {
            use webkit2gtk::{HitTestResultExt, WebViewExt};
            use wry::WebViewExtUnix;
            view.webview().connect_context_menu(|_, _, _, hit| {
                !(hit.context_is_editable() || hit.context_is_selection())
            });
        }
        use wry::WebViewExtUnix;
        let pinch = gtk::GestureZoom::new(&view.webview());
        let inner = Rc::new_cyclic(|me| Inner {
            view,
            container: container.clone(),
            me: me.clone(),
            error_view: RefCell::new(None),
            system_dark: Cell::new(true),
            host: RefCell::new(None),
            _favicons: favicons.clone(),
            _logos: logos.clone(),
            ready: Cell::new(false),
            queue: RefCell::new(Vec::new()),
            size: Cell::new((1., 1.)),
            cover: Cell::new(Some(CHROME_HEIGHT)),
            toast_rects: RefCell::new(Vec::new()),
            errors: RefCell::new(feedback::Errors::default()),
            waiting: RefCell::new(None),
            escape_surface: Cell::new(None),
            find: Cell::new(None),
            frame: RefCell::new(None),
            tab_room: Cell::new(0.),
            suggested: RefCell::new(Vec::new()),
            start: Cell::new(false),
            forest: RefCell::new(forest_pane::ForestPane::default()),
            palette_scope: Cell::new(None),
            palette: RefCell::new(None),
            workbench: RefCell::default(),
            fullscreen: Cell::new(false),
            pinch,
        });
        {
            use gtk::prelude::*;
            use wry::WebViewExtUnix;
            let weak = Rc::downgrade(&inner);
            inner
                .view
                .webview()
                .connect_query_tooltip(move |view, _, _, _, _| {
                    if weak
                        .upgrade()
                        .is_some_and(|inner| inner.escape_surface.get().is_some())
                    {
                        // WebKit caches title text on pointer motion, before a menu may open.
                        view.stop_signal_emission_by_name("query-tooltip");
                    }
                    false
                });
        }
        pinch::connect(&inner);
        *slot.borrow_mut() = Rc::downgrade(&inner);
        let weak = Rc::downgrade(&inner);
        favicons.connect_changed(move |url, revision| {
            if let Some(inner) = weak.upgrade() {
                inner.call("icons", &[json!(url), json!(revision)]);
            }
        });
        if let Some(logos) = logos {
            let weak = Rc::downgrade(&inner);
            logos.connect_changed(move |sites| {
                if let Some(inner) = weak.upgrade() {
                    inner.call("icons", &[Value::Null, Value::Null, json!(sites)]);
                }
            });
        }
        Ok(Self { inner })
    }

    /// Connects the browser the chrome sends its intents and queries to.
    pub fn attach(&self, host: Weak<dyn ChromeHost>) {
        *self.inner.host.borrow_mut() = Some(host);
    }

    pub fn present(&self, mut frame: ChromeFrame) {
        let inner = &self.inner;
        let committed = frame.forest.current();
        // A settled tree's page waits for its tree to reopen at commit; drawn earlier, it
        // would be current with no tab to stand under.
        if let Some(crate::browser::PendingTarget::Visit(visit)) = &frame.pending {
            let tree = frame.forest.tree_of(*visit);
            if frame.forest.tabs().iter().any(|t| Some(t.id) == tree) {
                frame.forest.show_current(*visit);
            }
        }
        let mut value = view::frame(
            &frame,
            &inner.suggested.borrow(),
            inner.size.get(),
            &inner.workbench.borrow(),
            view::Surfaces {
                start: inner.start.get(),
                palette: inner.palette_scope.get(),
            },
        );
        value["tabFit"] = tabs::layout(&frame.forest, inner.tab_room.get(), inner.start.get());
        // The page the window really shows, while `current` may already name the one opening.
        value["committed"] = json!(committed.map(|id| id.to_string()));
        let reply = inner.forest.borrow_mut().present(&frame);
        inner.call("present", &[value]);
        inner.forest_reply(reply);
        *inner.frame.borrow_mut() = Some(frame);
        inner.present_error();
    }

    /// Follows the window: the page sits below `CHROME_HEIGHT`, the view on top.
    pub fn resize(&self, width: f64, height: f64) {
        let inner = &self.inner;
        let previous = inner.size.replace((width, height));
        inner.apply_bounds();
        inner.call("viewport", &[json!(height)]);
        inner.present_error();
        if previous.0 != width {
            if let Some(frame) = inner.frame.borrow().as_ref() {
                inner.call(
                    "strip",
                    &[view::strip(
                        frame,
                        width,
                        &waiting::Waiting::of(&frame.forest),
                    )],
                );
            }
        }
    }

    pub fn open_palette(&self, scope: PaletteScope, seed: PaletteSeed) {
        let inner = &self.inner;
        let text = match seed {
            PaletteSeed::Empty => String::new(),
            PaletteSeed::CurrentAddress => inner
                .frame
                .borrow()
                .as_ref()
                .and_then(|frame| frame.forest.current().and_then(|id| frame.forest.visit(id)))
                .map(|visit| visit.url.to_string())
                .unwrap_or_default(),
        };
        let scope = match scope {
            PaletteScope::NewTree => Value::Null,
            PaletteScope::From(id) => json!(id.to_string()),
        };
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        let from_chrome = inner.view.webview().has_focus();
        inner.call(
            "openPalette",
            &[json!({ "scope": scope, "text": text, "fromChrome": from_chrome })],
        );
        let _ = inner.view.focus();
    }

    pub fn tab(&self, index: usize) -> Option<TreeId> {
        self.inner.call("tabSelected", &[]);
        self.inner
            .frame
            .borrow()
            .as_ref()?
            .forest
            .tabs()
            .get(index)
            .map(|t| t.id)
    }

    pub fn current_tree(&self) -> Option<TreeId> {
        let frame = self.inner.frame.borrow();
        let forest = &frame.as_ref()?.forest;
        forest.current().and_then(|id| forest.tree_of(id))
    }

    pub fn form_return(&self, open: Option<u64>) {
        self.inner.call("formReturn", &[json!(open)]);
        if open.is_some() {
            let _ = self.inner.view.focus();
        }
    }

    pub fn forest_open(&self) -> bool {
        self.inner.forest.borrow().is_open()
    }
    /// The forest shows a delete's Undo while it is open; otherwise a toast does.
    pub fn deleted(&self, id: Option<crate::browser::DeleteId>, text: String) {
        if self.forest_open() {
            self.inner.forest.borrow_mut().deleted(id, text);
        } else if let Some(id) = id {
            self.inner.call(
                "toast",
                &[json!("deleted"), json!(text), json!(id.to_string())],
            );
        }
    }
    pub fn rerank_suggestions(&self, forest: &crate::forest::ForestProjection) {
        if let Some(host) = self.inner.host() {
            *self.inner.suggested.borrow_mut() = host.suggestions(forest).unwrap_or_default();
            self.inner.palette.borrow_mut().take();
        }
    }
    pub fn settle_current(&self) {
        match self.inner.escape_surface.get() {
            Some("list") | None => {
                if let Some(host) = self.inner.host() {
                    host.dispatch(ChromeIntent::SettleCurrent);
                    host.focus_page();
                }
            }
            Some(surface) => self.inner.call("escape", &[json!(surface)]),
        }
    }

    pub fn uncover(&self) {
        let _ = self.evaluate("uncover()");
    }

    pub fn new_tree(&self) {
        self.inner.call("newTree", &[]);
        let _ = self.inner.view.focus();
    }

    /// Opens the Ctrl+Tab list. `recent` reads the current tree's read pages, most
    /// recently shown first, from the forest the chrome shows.
    pub fn open_switcher(
        &self,
        recent: impl FnOnce(&ForestProjection, usize) -> Result<Vec<VisitId>>,
    ) -> Result<()> {
        let rows = {
            let frame = self.inner.frame.borrow();
            let Some(frame) = frame.as_ref() else {
                return Ok(());
            };
            view::switcher_value(frame, &recent(&frame.forest, view::SWITCHER_READ)?)
        };
        self.inner.call("switcher", &[json!("open"), rows]);
        Ok(())
    }

    /// Moves the Ctrl+Tab selection one row down, or up with `back`, wrapping at the ends.
    pub fn step_switcher(&self, back: bool) {
        self.inner.call(
            "switcher",
            &[json!("step"), json!(if back { -1 } else { 1 })],
        );
    }

    /// Closes the Ctrl+Tab list, opening the selected page when `open` is set.
    pub fn end_switcher(&self, open: bool) {
        self.inner
            .call("switcher", &[json!(if open { "commit" } else { "cancel" })]);
    }

    pub fn toggle_tree(&self) {
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        let from_chrome = self.inner.view.webview().has_focus();
        self.inner.call("toggleTree", &[json!(from_chrome)]);
        let _ = self.inner.view.focus();
    }

    pub fn toggle_forest(&self, view: ForestView) {
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        let from_chrome = self.inner.view.webview().has_focus();
        self.inner
            .call("toggleForest", &[json!(view.name()), json!(from_chrome)]);
    }

    /// Offers these saved accounts for the sign-in fields on the page. No account takes
    /// the offer away. `offer` comes back with the choice, so a choice made on an offer
    /// that has since been replaced can be told apart.
    pub fn offer_logins(&self, offer: u64, usernames: &[&str]) {
        self.inner
            .call("logins", &[json!({ "offer": offer, "names": usernames })]);
    }

    /// Offers to save the login the user just signed in with, or to update its saved
    /// password. `None` takes the offer away.
    pub fn offer_save(&self, offer: Option<&SaveOffer>) {
        let value = offer.map_or(Value::Null, |offer| {
            json!({ "offer": offer.id, "site": offer.site, "username": offer.username, "update": offer.update, "keeper": offer.keeper })
        });
        self.inner.call("saveLogin", &[value]);
    }

    /// Says that `keeper` has to be unlocked before it can offer the page its logins, or,
    /// with `None`, that it no longer has to.
    pub fn logins_locked(&self, keeper: Option<&str>) {
        self.inner.call("loginsLocked", &[json!(keeper)]);
    }

    pub(crate) fn password_settings(&self, active: Option<&str>, next: &str, bitwarden: bool) {
        self.inner.call(
            "passwordSettings",
            &[json!({ "active": active, "next": next, "bitwarden": bitwarden })],
        );
    }

    /// Fills the only offered login, or opens the list when there are several.
    pub fn fill_login(&self) {
        self.inner.call("fillLogin", &[]);
    }

    pub fn queued(&self, child: VisitId) {
        self.inner
            .call("waiting", &[Value::Null, json!(child.to_string())]);
    }

    pub fn waiting(&self, parent: VisitId, label: &str) {
        if self
            .inner
            .frame
            .borrow()
            .as_ref()
            .and_then(|f| f.forest.current())
            != Some(parent)
        {
            return;
        }
        let now = std::time::Instant::now();
        let mut waiting = self.inner.waiting.borrow_mut();
        let count = waiting
            .as_ref()
            .filter(|(id, at, _)| *id == parent && now.duration_since(*at).as_millis() < 2600)
            .map_or(1, |(_, _, count)| count + 1);
        *waiting = Some((parent, now, count));
        let text = if count == 1 {
            format!("Waiting · {label}")
        } else {
            format!("{count} waiting from this page")
        };
        drop(waiting);
        self.inner.call("waiting", &[json!(text)]);
    }

    pub fn settled(&self, text: &str) {
        self.inner.call("toast", &[json!("settled"), json!(text)]);
    }

    pub fn toast(&self, toast: Toast) {
        let (kind, text) = match toast {
            Toast::Info(text) => ("info", text),
            Toast::Error(text) => ("error", text),
        };
        if kind == "error" {
            self.inner.report_text(&text);
        } else {
            self.inner.call("toast", &[json!(kind), json!(text)]);
        }
    }

    pub fn show_trees_help(&self) {
        self.inner.call("showTreesHelp", &[]);
    }

    /// F6 moves keyboard focus between the page and the current node.
    pub fn toggle_focus(&self) {
        self.inner.call("toggleFocus", &[]);
    }

    /// Closes the topmost chrome surface. Returns false when there was none, so the page
    /// can have the key.
    pub fn escape(&self) -> bool {
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        if let Some(surface) = self.inner.escape_surface.get() {
            if surface == "overlay" {
                if let Some(frame) = self.inner.frame.borrow().as_ref() {
                    let reply = self
                        .inner
                        .forest
                        .borrow_mut()
                        .input(forest_pane::Input::Escape, frame);
                    if !matches!(reply, forest_pane::Reply::Unchanged) {
                        self.inner.forest_reply(reply);
                        return true;
                    }
                }
            }
            self.inner.call("escape", &[json!(surface)]);
            return true;
        }
        if let Some(session) = self.inner.find.get() {
            // Closed now, not after a round trip through the view, so a quick Ctrl+F opens anew.
            if let Some(host) = self.inner.host() {
                host.dispatch(ChromeIntent::Find {
                    session,
                    action: crate::pages::FindAction::Close,
                });
            }
            return true;
        }
        let focused = self.inner.view.webview().has_focus()
            || self
                .inner
                .error_view
                .borrow()
                .as_ref()
                .is_some_and(|v| v.view.webview().has_focus());
        if focused && self.inner.errors.borrow().showing() {
            self.inner.errors.borrow_mut().act("dismiss");
            self.inner.errors_changed();
            return true;
        }
        false
    }

    /// Shows one saved text in the reader.
    pub fn read(&self, capture: &Capture, visit: &Visit, forest: &ForestProjection) {
        self.inner
            .call("reader", &[view::capture(capture, visit, forest)]);
        let _ = self.inner.view.focus();
    }

    /// Lets the user pick one of several saved texts for a page.
    pub fn choose(&self, captures: &[Capture]) {
        self.inner.call("readerList", &[view::captures(captures)]);
        let _ = self.inner.view.focus();
    }

    /// Whether `system` resolves to dark, from the desktop's preference.
    pub fn set_system_dark(&self, dark: bool) {
        self.inner.system_dark.set(dark);
        self.inner.call("systemDark", &[json!(dark)]);
        self.inner.present_error();
    }

    /// Hides the chrome while a page is fullscreen, and brings it back after.
    pub fn set_fullscreen(&self, on: bool) {
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        let inner = &self.inner;
        if inner.fullscreen.replace(on) == on {
            return;
        }
        inner.view.webview().set_visible(!on);
        if on {
            if let Some(error) = inner.error_view.borrow().as_ref() {
                error.bounds(inner.size.get(), false);
            }
        } else {
            inner.apply_bounds();
        }
    }

    /// Puts the chrome back above the page, after the page moved or took focus.
    pub fn raise(&self) {
        self.inner.raise();
    }

    /// Runs `script` in the chrome view. For tests.
    pub fn evaluate(&self, script: &str) -> Result<()> {
        self.inner.view.evaluate_script(script)?;
        Ok(())
    }

    /// The chrome's WebKit view.
    pub fn native(&self) -> webkit2gtk::WebView {
        use wry::WebViewExtUnix;
        self.inner.view.webview()
    }

    /// The error toast lives in its own small native view. For tests.
    pub fn error_native(&self) -> Option<webkit2gtk::WebView> {
        use wry::WebViewExtUnix;
        self.inner
            .error_view
            .borrow()
            .as_ref()
            .map(|v| v.view.webview())
    }

    pub fn is_ready(&self) -> bool {
        self.inner.ready.get()
    }
}

impl Inner {
    fn call(&self, function: &str, arguments: &[Value]) {
        let arguments: Vec<String> = arguments.iter().map(Value::to_string).collect();
        let script = format!("canopy.{function}({})", arguments.join(","));
        if self.ready.get() {
            let _ = self.view.evaluate_script(&script);
        } else {
            self.queue.borrow_mut().push(script);
        }
    }

    fn apply_bounds(&self) {
        let (width, height) = self.size.get();
        let height = if !self.toast_rects.borrow().is_empty() {
            height
        } else {
            self.cover.get().map_or(height, |cover| cover.min(height))
        };
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        self.view
            .webview()
            .set_size_request(width.round() as i32, height.round() as i32);
        let _ = self.view.set_bounds(rect(0., 0., width, height));
        // wry recognizes GtkFixed by exact type name, so its bounds setter skips
        // allocation for our Fixed subclass. Keep chrome's allocation with its request.
        self.view.webview().size_allocate(&gtk::Allocation::new(
            0,
            0,
            width.round() as i32,
            height.round() as i32,
        ));
        if let Some(window) = self.view.webview().window() {
            let region = self.cover.get().map(|cover| {
                let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(
                    0,
                    0,
                    width.ceil() as i32,
                    cover.ceil() as i32,
                ));
                for r in self.toast_rects.borrow().iter() {
                    let _ = region
                        .union_rectangle(&gtk::cairo::RectangleInt::new(r[0], r[1], r[2], r[3]));
                }
                region
            });
            let region = region.unwrap_or_else(|| {
                gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(
                    0,
                    0,
                    width.ceil() as i32,
                    height.ceil() as i32,
                ))
            });
            window.input_shape_combine_region(&region, 0, 0);
        }
        self.raise();
    }

    /// Keeps the chrome's window above the page's. WebKit can restack the page's window
    /// (on focus, on reallocation), and input then reaches the page through the chrome.
    fn raise(&self) {
        use wry::WebViewExtUnix;
        if let Some(window) = gtk::prelude::WidgetExt::window(&self.view.webview()) {
            window.raise();
        }
        if let Some(error) = self.error_view.borrow().as_ref() {
            let showing = self.errors.borrow().showing() && !self.fullscreen.get();
            error.bounds(self.size.get(), showing);
        }
    }

    fn host(&self) -> Option<Rc<dyn ChromeHost>> {
        self.host.borrow().as_ref()?.upgrade()
    }

    fn report_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.errors.borrow_mut().push(text);
        self.errors_changed();
    }

    fn present_error(&self) {
        if let Some(error) = self.error_view.borrow().as_ref().filter(|v| v.ready.get()) {
            let mode = self
                .frame
                .borrow()
                .as_ref()
                .map_or(ThemeMode::Dark, |f| f.theme);
            let dark =
                mode == ThemeMode::Dark || (mode == ThemeMode::System && self.system_dark.get());
            let _ = error.view.evaluate_script(&format!(
                "canopy.present({}, {}, {})",
                self.errors.borrow().value()["toast"],
                self.size.get().0,
                dark
            ));
        }
    }

    fn errors_changed(&self) {
        if self.errors.borrow().showing() && self.error_view.borrow().is_none() {
            match feedback::ErrorView::new(&self.container, self.me.clone()) {
                Ok(view) => *self.error_view.borrow_mut() = Some(view),
                Err(error) => eprintln!("Could not create the error view: {error}"),
            }
        }
        self.call("errors", &[self.errors.borrow().value()]);
        self.present_error();
        if let Some(error) = self.error_view.borrow().as_ref() {
            error.bounds(self.size.get(), self.errors.borrow().showing());
        }
    }

    fn report(&self, error: anyhow::Error) {
        self.report_text(&error.to_string());
    }

    fn message(self: &Rc<Self>, body: &str) {
        if let Err(error) = self.handle(body) {
            self.report(error);
        }
    }

    fn forest_reply(&self, reply: forest_pane::Reply) {
        match reply {
            forest_pane::Reply::Unchanged => {}
            forest_pane::Reply::Act(intent) => {
                if let Some(host) = self.host() {
                    host.dispatch(intent);
                }
            }
            forest_pane::Reply::Go(intent) => {
                self.call("navigated", &[]);
                if let Some(host) = self.host() {
                    host.dispatch(intent);
                    host.focus_page();
                }
            }
            forest_pane::Reply::Return => {
                self.call("navigated", &[]);
                if let Some(host) = self.host() {
                    host.focus_page();
                }
            }
            forest_pane::Reply::Close => self.call("escape", &[json!("overlay")]),
            forest_pane::Reply::Show(value) => self.call("forest", &[value]),
        }
    }

    fn handle(self: &Rc<Self>, body: &str) -> Result<()> {
        let message: Value = serde_json::from_str(body)?;
        match message["t"].as_str().unwrap_or_default() {
            "ready" => {
                self.ready.set(true);
                self.call("help", &[start::help()]);
                let queued = std::mem::take(&mut *self.queue.borrow_mut());
                for script in queued {
                    self.view.evaluate_script(&script)?;
                }
            }
            "branch-move-choices" => {
                if let (Some(request), Some(head)) = (
                    message["request"].as_u64(),
                    message["head"]
                        .as_str()
                        .and_then(|s| s.parse::<VisitId>().ok()),
                ) {
                    let reply = self.frame.borrow().as_ref().map(|frame| {
                        let choices = frame.forest.branch_move_choices(head);
                        let own_tree = choices.own_tree.map(|own| match own {
                            crate::browser::OwnTreeMove::Detach => json!({"kind":"detach"}),
                            crate::browser::OwnTreeMove::Reorder(tree) => json!({"kind":"reorder","tree":tree.root().to_string()}),
                        });
                        json!({"request":request,"head":head.to_string(),
                            "parents":choices.parents.iter().map(ToString::to_string).collect::<Vec<_>>(),
                            "trees":choices.trees.iter().map(|t| json!({"id":t.tree.root().to_string(),"title":t.title})).collect::<Vec<_>>(),
                            "ownTree":own_tree,"warning":choices.warning})
                    });
                    if let Some(reply) = reply {
                        self.call("branchMoveChoices", &[reply]);
                    }
                }
            }
            "paletteScope" => {
                self.palette_scope
                    .set(message["id"].as_str().map(str::parse).transpose()?);
            }
            "tree" => {
                let tree = TreeId(message["tree"].as_str().context("Missing tree")?.parse()?);
                let act = tree_card::Act::parse(message["act"].as_str().unwrap_or_default())
                    .context("Unknown tree action")?;
                let intent = {
                    let frame = self.frame.borrow();
                    frame
                        .as_ref()
                        .and_then(|f| f.forest.tree(tree))
                        .and_then(|t| tree_card::intent(t, act))
                };
                if let (Some(intent), Some(host)) = (intent, self.host()) {
                    host.dispatch(intent);
                }
            }
            "forest" => {
                let input = forest_pane::Input::parse(&message)?;
                let reply = {
                    let frame = self.frame.borrow();
                    frame
                        .as_ref()
                        .map(|frame| self.forest.borrow_mut().input(input, frame))
                };
                if let Some(reply) = reply {
                    self.forest_reply(reply);
                }
            }
            "suggestions" => {
                if self.start.get() {
                    let frame = self.frame.borrow();
                    if let Some(frame) = frame.as_ref() {
                        *self.suggested.borrow_mut() = self
                            .host()
                            .context("The browser closed.")?
                            .suggestions(&frame.forest)?;
                        self.call(
                            "start",
                            &[json!({"start": start::value(frame, &self.suggested.borrow())})],
                        );
                    }
                }
            }
            "surface" => {
                let start = message["start"].as_bool().unwrap_or(false);
                if self.start.replace(start) != start {
                    if start {
                        self.freeze()?;
                    }
                    if let Some(frame) = self.frame.borrow().as_ref() {
                        self.call(
                            "tabsFit",
                            &[tabs::layout(&frame.forest, self.tab_room.get(), start)],
                        );
                    }
                }
            }
            "tab-room" => {
                if let Some(room) = message["room"]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.)
                {
                    self.tab_room.set(room);
                    if let Some(frame) = self.frame.borrow().as_ref() {
                        self.call(
                            "tabsFit",
                            &[tabs::layout(&frame.forest, room, self.start.get())],
                        );
                    }
                }
            }
            "error-ready" => {
                if let Some(error) = self.error_view.borrow().as_ref() {
                    error.ready.set(true);
                }
                self.present_error();
            }
            "error-size" => {
                if let Some(error) = self.error_view.borrow().as_ref() {
                    error.size.set((
                        message["width"].as_i64().unwrap_or(0) as i32,
                        message["height"].as_i64().unwrap_or(0) as i32,
                    ));
                    *error.buttons.borrow_mut() =
                        serde_json::from_value(message["buttons"].clone())?;
                    error.bounds(self.size.get(), self.errors.borrow().showing());
                }
            }
            "errors" => {
                let action = message["action"].as_str().unwrap_or_default();
                self.errors
                    .borrow_mut()
                    .act(if action == "open" { "read" } else { action });
                self.errors_changed();
                if action == "open" {
                    self.call("openErrors", &[]);
                    let _ = self.view.focus();
                }
            }
            "layer" => {
                self.escape_surface.set(
                    ESCAPE_SURFACES
                        .into_iter()
                        .find(|surface| message["surfaces"][surface].as_bool() == Some(true)),
                );
                let cover = message["height"].as_f64().map(|h| h.max(CHROME_HEIGHT));
                let toast_rects =
                    serde_json::from_value::<Vec<[i32; 4]>>(message["toastRects"].clone())?;
                let changed = *self.toast_rects.borrow() != toast_rects;
                *self.toast_rects.borrow_mut() = toast_rects;
                if self.cover.replace(cover) != cover || changed {
                    self.apply_bounds();
                }
            }
            "workbench" => {
                if let Some(frame) = self.frame.borrow().as_ref() {
                    let mut state = self.workbench.borrow_mut();
                    if let Some(query) = message["query"].as_str() {
                        state.query = query.to_owned();
                    }
                    if let Some(waiting) = message["waiting"].as_bool() {
                        state.waiting_only = waiting;
                    }
                    if let Some(id) = message["id"].as_str().and_then(|id| id.parse().ok()) {
                        if frame.forest.visit(id).is_some() {
                            let folded = message["folded"]
                                .as_bool()
                                .unwrap_or(!state.folded(&frame.forest, id));
                            let settled_at =
                                frame.forest.visit(id).and_then(|v| v.branch_settled_at);
                            state.folds.insert((id, settled_at), folded);
                        }
                    }
                    self.call(
                        "workbench",
                        &[view::tree_view_value(
                            &frame.forest,
                            &state,
                            self.size.get(),
                        )],
                    );
                }
            }
            "copy" => {
                if let Some(text) = message["text"].as_str() {
                    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(text);
                }
            }
            "focus" => {
                if message["target"] == "page" {
                    if let Some(host) = self.host() {
                        host.focus_page();
                    }
                } else {
                    self.view.focus()?;
                }
            }
            "form-return" => {
                if let Some(host) = self.host() {
                    if let Some(question) = message["question"].as_u64() {
                        host.dispatch(ChromeIntent::AnswerFormReturn {
                            question,
                            proceed: message["proceed"].as_bool().unwrap_or(false),
                        });
                    }
                }
            }
            "intent" => {
                let intent = self.intent(&message);
                if intent.is_none()
                    && matches!(message["kind"].as_str(), Some("open-mail" | "open-inbox"))
                {
                    return Ok(());
                }
                let intent = intent.context("The chrome sent an unknown action.")?;
                if let Some(host) = self.host() {
                    host.dispatch(intent);
                }
            }
            "palette" => {
                let query = message["q"].as_str().unwrap_or_default();
                let scope = self
                    .palette_scope
                    .get()
                    .map_or(PaletteScope::NewTree, PaletteScope::From);
                let rows = {
                    let (items, first) = self.palette_items(query, scope);
                    let frame = self.frame.borrow();
                    let forest = &frame
                        .as_ref()
                        .context("The chrome has no forest yet.")?
                        .forest;
                    let rows =
                        json!({ "rows": view::palette_rows(&items, forest), "first": first });
                    *self.palette.borrow_mut() = Some((query.to_owned(), items, first));
                    rows
                };
                self.call("paletteResults", &[message["seq"].clone(), rows]);
            }
            "run" => {
                let query = message["q"].as_str().unwrap_or_default();
                let shown = self
                    .palette
                    .borrow()
                    .as_ref()
                    .filter(|(q, _, _)| q == query)
                    .cloned();
                let scope = match message["scope"].as_str() {
                    Some(id) => PaletteScope::From(id.parse()?),
                    None => PaletteScope::NewTree,
                };
                let (items, index) = match shown {
                    Some((_, items, first)) => (
                        items,
                        message["item"].as_u64().map_or(first, |i| i as usize),
                    ),
                    None => self.palette_items(query, scope),
                };
                let new_tree = message["newTree"].as_bool().unwrap_or(false);
                let intent = {
                    let frame = self.frame.borrow();
                    let forest = &frame
                        .as_ref()
                        .context("The chrome has no forest yet.")?
                        .forest;
                    items
                        .get(index)
                        .and_then(|(_, item)| palette::intent(item, scope, new_tree, forest))
                };
                if let (Some(intent), Some(host)) = (intent, self.host()) {
                    host.dispatch(intent);
                }
            }
            "preview" => {
                let id: VisitId = message["id"].as_str().unwrap_or_default().parse()?;
                let host = self.host().context("The browser closed.")?;
                let preview = host.preview(id)?;
                let value = {
                    let frame = self.frame.borrow();
                    let forest = &frame
                        .as_ref()
                        .context("The chrome has no forest yet.")?
                        .forest;
                    view::preview(forest, id, &preview)
                };
                self.call("previewResult", &[message["seq"].clone(), value]);
            }
            other => anyhow::bail!("The chrome sent an unknown message: {other}"),
        }
        Ok(())
    }

    /// Fresh results for `query`, independent of the rows the view was last sent.
    fn palette_items(
        &self,
        query: &str,
        scope: PaletteScope,
    ) -> (Vec<(Group, PaletteItem)>, usize) {
        let (hits, imported) = match self.host() {
            Some(host) if !query.trim().is_empty() => (
                host.search(query).unwrap_or_default(),
                host.search_imported(query).unwrap_or_default(),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        let frame = self.frame.borrow();
        let Some(frame) = frame.as_ref() else {
            return (Vec::new(), 0);
        };
        let (mut items, first) = palette::results(
            query,
            hits,
            imported,
            &frame.bookmarks,
            &frame.forest,
            scope,
        );
        if query.trim().is_empty() {
            let suggestions = if self.start.get() {
                self.suggested.borrow().clone()
            } else {
                self.host()
                    .and_then(|h| h.suggestions(&frame.forest).ok())
                    .unwrap_or_default()
            };
            items.extend(
                suggestions
                    .into_iter()
                    .filter(|s| {
                        crate::suggest::admits(&frame.forest, &frame.bookmarks, &frame.declined, s)
                    })
                    .map(|s| (Group::Suggested, PaletteItem::Suggestion(s))),
            );
        }
        (items, first)
    }

    fn freeze(&self) -> Result<()> {
        let frame = self.frame.borrow();
        let Some(frame) = frame.as_ref() else {
            return Ok(());
        };
        self.suggested.borrow_mut().clear();
        self.call(
            "start",
            &[
                view::frame(
                    frame,
                    &self.suggested.borrow(),
                    self.size.get(),
                    &self.workbench.borrow(),
                    view::Surfaces {
                        start: true,
                        palette: self.palette_scope.get(),
                    },
                ),
                json!(true),
            ],
        );
        Ok(())
    }

    fn chip(&self, message: &Value) -> Option<crate::suggest::Suggestion> {
        let key: crate::suggest::SuggestionKey = message["key"].as_str()?.parse().ok()?;
        self.suggested
            .borrow()
            .iter()
            .find(|s| s.key() == key)
            .cloned()
    }

    fn intent(&self, message: &Value) -> Option<ChromeIntent> {
        let id = |key: &str| message[key].as_str()?.parse::<VisitId>().ok();
        let frame = self.frame.borrow();
        let frame = frame.as_ref()?;
        let index = || message["index"].as_u64().map(|i| i as usize);
        Some(match message["kind"].as_str()? {
            "mail-opened" => ChromeIntent::MailOpened,
            "mail-grouping" => {
                ChromeIntent::SetMailGrouping(Grouping::parse(message["grouping"].as_str()?)?)
            }
            "check-mail" => ChromeIntent::Mail(crate::mail::MailAsk::Check),
            "mark-mail-read" => ChromeIntent::Mail(crate::mail::MailAsk::MarkRead {
                address: message["account"].as_str()?.to_owned(),
                id: message["id"].as_str()?.to_owned(),
            }),
            "open-mail" => ChromeIntent::Navigate {
                url: frame
                    .mail
                    .link_for(message["account"].as_str()?, message["id"].as_str()?)?
                    .clone(),
                target: NavigationTarget::Root,
            },
            "open-inbox" => ChromeIntent::Navigate {
                url: frame.mail.inbox(message["account"].as_str()?)?.clone(),
                target: NavigationTarget::Root,
            },
            "stop" => ChromeIntent::Stop,
            "dismiss" => ChromeIntent::Dismiss(id("id")?),
            "settle-branch" => ChromeIntent::SettleBranch(id("id")?),
            "delete-branch" => ChromeIntent::DeleteBranch(id("id")?),
            "undo-delete" => ChromeIntent::UndoDelete(message["id"].as_str()?.parse().ok()?),
            "move-to-tree" => ChromeIntent::MoveBranch {
                head: id("id")?,
                to: MoveTo::OwnTree(TabPlacement::End),
            },
            "move-branch" => ChromeIntent::MoveBranch {
                head: id("head")?,
                to: parse_move_to(&message["to"])?,
            },
            "undo-settle" => ChromeIntent::UndoSettle,

            "place-tree" => ChromeIntent::PlaceTree {
                tree: TreeId(id("id")?),
                to: match message["place"].as_str()? {
                    "tabs" => crate::browser::Move::Tabs(if message["before"].is_null() {
                        crate::browser::Slot::End
                    } else {
                        let before = TreeId(id("before")?);
                        if !frame.forest.tree(before)?.place.is_tab() {
                            return None;
                        }
                        crate::browser::Slot::Before(before)
                    }),
                    _ => return None,
                },
            },
            "settle-current" => ChromeIntent::SettleCurrent,
            "rename-tree" => {
                ChromeIntent::RenameTree(TreeId(id("id")?), message["name"].as_str()?.to_owned())
            }
            "restore" => ChromeIntent::Restore(id("id")?),
            "open-page-in-tree" => ChromeIntent::Navigate {
                url: frame.forest.visit(id("id")?)?.url.clone(),
                target: NavigationTarget::Root,
            },
            "page-scale" => page_scale::intent(message)?,
            "reload" => ChromeIntent::Reload,
            "back" => ChromeIntent::Back,
            "forward" => ChromeIntent::Forward,
            "branch-queued" => ChromeIntent::BranchFromQueued(id("id")?),
            "open-citation" => ChromeIntent::OpenCitation(id("id")?),
            "read-captures" => ChromeIntent::ReadCaptures(id("id")?),
            "read-capture" => ChromeIntent::ReadCapture(message["capture"].as_i64()?),
            "open-saved" => ChromeIntent::OpenSaved(message["capture"].as_i64()?),
            "theme" => ChromeIntent::SetTheme(match message["mode"].as_str()? {
                "light" => ThemeMode::Light,
                "dark" => ThemeMode::Dark,
                _ => ThemeMode::System,
            }),
            "trees-help" => ChromeIntent::ShowTreesHelp(message["show"].as_bool()?),
            "toggle-bookmark" => ChromeIntent::ToggleBookmark,
            "find-open" => ChromeIntent::OpenFind,
            "quit" => ChromeIntent::Quit,
            "capture" => ChromeIntent::SetCaptureEnabled(message["on"].as_bool()?),
            "remember" => ChromeIntent::RememberSelection,
            "open-file" => ChromeIntent::OpenFile(message["file"].as_i64()?),
            "open-file-folder" => ChromeIntent::OpenFileFolder(message["file"].as_i64()?),
            "unlock-logins" => ChromeIntent::UnlockLogins,
            "password-vault" => ChromeIntent::SetPasswordProvider(PasswordChoice::Vault),
            "password-bitwarden" => ChromeIntent::SetPasswordProvider(PasswordChoice::Bitwarden),
            "save-login" => ChromeIntent::AnswerSave {
                offer: message["offer"].as_u64()?,
                save: message["save"].as_bool()?,
            },
            "fill-login" => ChromeIntent::FillLogin {
                offer: message["offer"].as_u64()?,
                index: index()?,
            },
            "open-bookmark" => ChromeIntent::Navigate {
                url: frame
                    .bookmarks
                    .iter()
                    .find(|b| Some(b.id.get()) == message["id"].as_i64())?
                    .url
                    .clone(),
                target: NavigationTarget::Root,
            },
            "remove-bookmark" => ChromeIntent::RemoveBookmark(
                frame
                    .bookmarks
                    .iter()
                    .find(|b| Some(b.id.get()) == message["id"].as_i64())?
                    .id,
            ),
            "cancel" => ChromeIntent::CancelContinuation,
            "open-suggestion" => start::open(&self.chip(message)?),
            "keep-suggestion" => start::keep(&self.chip(message)?),
            "decline-suggestion" => ChromeIntent::Decline(self.chip(message)?.key()),
            "find" => find::intent(message)?,
            _ => return None,
        })
    }
}

/// The chrome's own files, its font, and page icons, on the `canopy://` scheme. Only the
/// chrome view registers it, so pages cannot read it.
const FOREST_ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "/forest.js",
        "text/javascript",
        include_bytes!("assets/forest.js"),
    ),
    (
        "/forest.css",
        "text/css",
        include_bytes!("assets/forest.css"),
    ),
    (
        "/forest-canopy.js",
        "text/javascript",
        include_bytes!("assets/forest-canopy.js"),
    ),
    (
        "/forest-canopy.css",
        "text/css",
        include_bytes!("assets/forest-canopy.css"),
    ),
    (
        "/forest-history.js",
        "text/javascript",
        include_bytes!("assets/forest-history.js"),
    ),
    (
        "/forest-history.css",
        "text/css",
        include_bytes!("assets/forest-history.css"),
    ),
];

fn serve(
    request: &Request<Vec<u8>>,
    favicons: &Weak<Favicons>,
    logos: Option<&Weak<SenderLogos>>,
) -> Response<Cow<'static, [u8]>> {
    let respond = |kind: &str, body: Cow<'static, [u8]>| {
        Response::builder()
            .header(CONTENT_TYPE, kind)
            .body(body)
            .expect("a static response builds")
    };
    match request.uri().path() {
        "/" | "/index.html" => respond("text/html", Cow::Borrowed(INDEX.as_bytes())),
        "/error.html" => respond(
            "text/html",
            Cow::Borrowed(include_bytes!("assets/error.html")),
        ),
        "/chrome.css" => respond("text/css", Cow::Borrowed(CSS.as_bytes())),
        "/chrome.js" => respond("text/javascript", Cow::Borrowed(SCRIPT.as_bytes())),
        path if FOREST_ASSETS.iter().any(|(name, _, _)| *name == path) => {
            let (_, kind, bytes) = FOREST_ASSETS
                .iter()
                .find(|(name, _, _)| *name == request.uri().path())
                .unwrap();
            respond(kind, Cow::Borrowed(bytes))
        }
        "/inter.ttf" => respond("font/ttf", Cow::Borrowed(INTER)),
        "/icon" => {
            let query = request.uri().query().unwrap_or_default();
            let mut target = None;
            let mut opened = false;
            let mut sender = false;
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                match &*key {
                    "u" => target = Some(value),
                    "o" => opened = value == "1",
                    "sender" => sender = value == "1",
                    _ => {}
                }
            }
            let png = target.and_then(|target| {
                if sender {
                    logos?.upgrade()?.png(&target)
                } else {
                    let url = Url::parse(&target).ok()?;
                    favicons.upgrade()?.png(&url, opened)
                }
            });
            let mut response = match png {
                Some(png) => respond("image/png", Cow::Owned(png.to_vec())),
                None => Response::builder()
                    .status(404)
                    .body(Cow::Borrowed(&[][..]))
                    .expect("a static response builds"),
            };
            if sender {
                use wry::http::header::{HeaderValue, CACHE_CONTROL};
                response
                    .headers_mut()
                    .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            }
            response
        }
        _ => Response::builder()
            .status(404)
            .body(Cow::Borrowed(&[][..]))
            .expect("a static response builds"),
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width.max(1.), height.max(1.)).into(),
    }
}

#[cfg(test)]
mod forest_asset_tests {
    #[test]
    fn chrome_markup_calls_the_map_canopy() {
        assert!(INDEX.contains("title=\"Canopy · Ctrl+E\""));
        assert!(INDEX.contains("aria-label=\"Canopy\""));
        assert!(INDEX.contains("data-view=\"canopy\">Canopy"));
        assert!(!INDEX.contains("forest-trees"));
        assert!(!INDEX.contains(">Forest<"));
        assert!(!SCRIPT.contains("<span>Forest</span>"));
    }

    use super::*;
    #[test]
    fn split_assets_are_served_and_ready_waits_for_all_scripts() {
        for (name, _, bytes) in FOREST_ASSETS {
            assert!(INDEX.contains(&format!("\"{}\"", &name[1..])));
            assert!(!bytes.is_empty());
        }
        assert!(SCRIPT
            .contains("document.addEventListener('DOMContentLoaded', () => post({ t: 'ready' }))"));
        assert!(
            SCRIPT.find("  forest(value)").unwrap()
                < SCRIPT.find("for (const kind of ['present'").unwrap()
        );
    }
}
