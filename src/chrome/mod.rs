//! The chrome: tabs, the tree strip, the palette, cards, menus, the new-tree page, the forest
//! view and the saved-text reader, drawn in HTML by a transparent web view over the page.
//!
//! Rust owns the state and every decision: layouts, palette results, intents. The view paints
//! what `view` serializes and reports input over IPC. At rest the view covers only the top
//! 94 px. It grows down to whatever is open over the page (the open strip, a card, a menu)
//! and covers the window only for the palette, the reader and the full-page surfaces, so the
//! page keeps its clicks everywhere else.

mod favicon;
mod forest_layout;
pub(crate) mod graph;
mod palette;
mod pins;
mod start;
mod tabs;
mod tree_view;
mod view;
mod waiting;

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
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

use crate::{
    browser::{
        Capture, Download, ImportedPage, NavigationTarget, SearchHit, ThemeMode, TreeId, Visit,
        VisitId, VisitPreview,
    },
    forest::ForestProjection,
};

/// The tab row and the resting strip.
pub const CHROME_HEIGHT: f64 = 94.;

const INDEX: &str = include_str!("assets/index.html");
const CSS: &str = include_str!("assets/chrome.css");
const SCRIPT: &str = include_str!("assets/chrome.js");
const INTER: &[u8] = include_bytes!("../../assets/fonts/InterVariable.ttf");

thread_local! {
    /// The icon cache, for the protocol handler, which wry wants `Send` but calls on the
    /// main thread.
    static FAVICONS: RefCell<Weak<Favicons>> = const { RefCell::new(Weak::new()) };
}

/// The query and rows last sent to the view, and their default row.
type ShownPalette = (String, Vec<(Group, PaletteItem)>, usize);

/// Everything the chrome shows, read from SQLite in one go.
pub struct ChromeFrame {
    pub bookmarks: Vec<crate::browser::Bookmark>,
    pub forest: ForestProjection,
    /// Waiting pages that already have a live view. Nothing fills it yet: background
    /// loading will, and the chrome then shows those pages as loaded with no other change.
    pub loaded: std::collections::HashSet<VisitId>,
    /// Current documents retained in hidden, previously shown pages.
    pub live: std::collections::HashSet<VisitId>,
    pub capture_enabled: bool,
    pub theme: ThemeMode,
    pub declined: std::collections::HashSet<crate::suggest::SuggestionKey>,
    /// The files downloaded from the current tree's pages, oldest first.
    pub files: Vec<PageFile>,
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
    AnswerFormReturn {
        question: u64,
        proceed: bool,
    },
    SelectTree(TreeId),
    SettleTree(TreeId),
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
    CancelContinuation,
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
    /// Answer save offer number `offer`: keep the login, or not now.
    AnswerSave {
        offer: u64,
        save: bool,
    },
    RemoveBookmark(crate::browser::BookmarkId),
    PinTree {
        tree: TreeId,
        pinned: bool,
    },
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

struct Inner {
    view: WebView,
    host: RefCell<Option<Weak<dyn ChromeHost>>>,
    _favicons: Rc<Favicons>,
    ready: Cell<bool>,
    /// Calls made before the view loaded, replayed once it reports ready.
    queue: RefCell<Vec<String>>,
    size: Cell<(f64, f64)>,
    /// How far down the view reaches, or `None` to cover the window.
    cover: Cell<Option<f64>>,
    /// Something is open that Escape closes.
    modal: Cell<bool>,
    frame: RefCell<Option<ChromeFrame>>,
    suggested: RefCell<Vec<crate::suggest::Suggestion>>,
    start: Cell<bool>,
    /// The last palette results, so Enter runs exactly the row the view showed.
    palette: RefCell<Option<ShownPalette>>,
}

impl HtmlChrome {
    /// Adds the chrome view to `container`, above the page already in it.
    pub fn mount(container: &gtk::Fixed, favicons: Rc<Favicons>) -> Result<Self> {
        use wry::WebViewBuilderExtUnix;

        FAVICONS.with(|slot| *slot.borrow_mut() = Rc::downgrade(&favicons));
        let slot: Rc<RefCell<Weak<Inner>>> = Rc::default();
        let messages = slot.clone();
        let view = WebViewBuilder::new()
            .with_transparent(true)
            .with_bounds(rect(0., 0., 1., CHROME_HEIGHT))
            .with_custom_protocol("canopy".into(), |_, request| serve(&request))
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
        let inner = Rc::new(Inner {
            view,
            host: RefCell::new(None),
            _favicons: favicons.clone(),
            ready: Cell::new(false),
            queue: RefCell::new(Vec::new()),
            size: Cell::new((1., 1.)),
            cover: Cell::new(Some(CHROME_HEIGHT)),
            modal: Cell::new(false),
            frame: RefCell::new(None),
            suggested: RefCell::new(Vec::new()),
            start: Cell::new(false),
            palette: RefCell::new(None),
        });
        *slot.borrow_mut() = Rc::downgrade(&inner);
        let weak = Rc::downgrade(&inner);
        favicons.connect_changed(move |revision| {
            if let Some(inner) = weak.upgrade() {
                inner.call("icons", &[json!(revision)]);
            }
        });
        Ok(Self { inner })
    }

    /// Connects the browser the chrome sends its intents and queries to.
    pub fn attach(&self, host: Weak<dyn ChromeHost>) {
        *self.inner.host.borrow_mut() = Some(host);
    }

    pub fn present(&self, frame: ChromeFrame) {
        let inner = &self.inner;
        let (width, _) = inner.size.get();
        inner.call(
            "present",
            &[view::frame(&frame, &inner.suggested.borrow(), width)],
        );
        *inner.frame.borrow_mut() = Some(frame);
    }

    /// Follows the window: the page sits below `CHROME_HEIGHT`, the view on top.
    pub fn resize(&self, width: f64, height: f64) {
        let inner = &self.inner;
        let previous = inner.size.replace((width, height));
        inner.apply_bounds();
        inner.call("viewport", &[json!(height)]);
        if previous.0 != width {
            if let Some(frame) = inner.frame.borrow().as_ref() {
                inner.call("strip", &[view::strip(&frame.forest, width)]);
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
        inner.call("openPalette", &[json!({ "scope": scope, "text": text })]);
        let _ = inner.view.focus();
    }

    pub fn tab(&self, index: usize) -> Option<TreeId> {
        self.inner
            .frame
            .borrow()
            .as_ref()?
            .forest
            .open()
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

    pub fn settle_current(&self) {
        self.inner.call("settleCurrent", &[]);
    }

    pub fn new_tree(&self) {
        self.inner.call("newTree", &[]);
        let _ = self.inner.view.focus();
    }

    pub fn next_waiting(&self) {
        self.inner.call("nextWaiting", &[]);
    }

    pub fn toggle_tree(&self) {
        self.inner.call("toggleTree", &[]);
    }

    pub fn toggle_forest(&self) {
        self.inner.call("toggleForest", &[]);
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

    /// Fills the only offered login, or opens the list when there are several.
    pub fn fill_login(&self) {
        self.inner.call("fillLogin", &[]);
    }

    pub fn toast(&self, toast: Toast) {
        let (kind, text) = match toast {
            Toast::Info(text) => ("info", text),
            Toast::Error(text) => ("error", text),
        };
        self.inner.call("toast", &[json!(kind), json!(text)]);
    }

    /// F6 moves keyboard focus between the page and the current node.
    pub fn toggle_focus(&self) {
        self.inner.call("toggleFocus", &[]);
    }

    /// Closes the topmost chrome surface. Returns false when there was none, so the page
    /// can have the key.
    pub fn escape(&self) -> bool {
        if !self.inner.modal.get() {
            return false;
        }
        self.inner.call("escape", &[]);
        true
    }

    /// Shows one saved text in the reader.
    pub fn read(&self, capture: &Capture, visit: &Visit) {
        self.inner.call("reader", &[view::capture(capture, visit)]);
        let _ = self.inner.view.focus();
    }

    /// Lets the user pick one of several saved texts for a page.
    pub fn choose(&self, captures: &[Capture]) {
        self.inner.call("readerList", &[view::captures(captures)]);
        let _ = self.inner.view.focus();
    }

    /// Whether `system` resolves to dark, from the desktop's preference.
    pub fn set_system_dark(&self, dark: bool) {
        self.inner.call("systemDark", &[json!(dark)]);
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
        let height = self.cover.get().map_or(height, |cover| cover.min(height));
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
        self.raise();
    }

    /// Keeps the chrome's window above the page's. WebKit can restack the page's window
    /// (on focus, on reallocation), and input then reaches the page through the chrome.
    fn raise(&self) {
        use wry::WebViewExtUnix;
        if let Some(window) = gtk::prelude::WidgetExt::window(&self.view.webview()) {
            window.raise();
        }
    }

    fn host(&self) -> Option<Rc<dyn ChromeHost>> {
        self.host.borrow().as_ref()?.upgrade()
    }

    fn report(&self, error: anyhow::Error) {
        self.call("toast", &[json!("error"), json!(error.to_string())]);
    }

    fn message(self: &Rc<Self>, body: &str) {
        if let Err(error) = self.handle(body) {
            self.report(error);
        }
    }

    fn handle(self: &Rc<Self>, body: &str) -> Result<()> {
        let message: Value = serde_json::from_str(body)?;
        match message["t"].as_str().unwrap_or_default() {
            "ready" => {
                self.ready.set(true);
                let queued = std::mem::take(&mut *self.queue.borrow_mut());
                for script in queued {
                    self.view.evaluate_script(&script)?;
                }
            }
            "surface" => {
                let start = message["start"].as_bool().unwrap_or(false);
                if self.start.replace(start) != start && start {
                    self.freeze()?;
                }
            }
            "tab-room" => {
                if let Some(room) = message["room"]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.)
                {
                    if let Some(frame) = self.frame.borrow().as_ref() {
                        self.call("tabsFit", &[tabs::layout(&frame.forest, room)]);
                    }
                }
            }
            "layer" => {
                self.modal.set(message["modal"].as_bool().unwrap_or(false));
                let cover = message["height"].as_f64().map(|h| h.max(CHROME_HEIGHT));
                if self.cover.replace(cover) != cover {
                    self.apply_bounds();
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
                let intent = self
                    .intent(&message)
                    .context("The chrome sent an unknown action.")?;
                if let Some(host) = self.host() {
                    host.dispatch(intent);
                }
            }
            "palette" => {
                let query = message["q"].as_str().unwrap_or_default();
                let rows = {
                    let (items, first) = self.palette_items(query);
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
                let (items, index) = match shown {
                    Some((_, items, first)) => (
                        items,
                        message["item"].as_u64().map_or(first, |i| i as usize),
                    ),
                    None => self.palette_items(query),
                };
                let scope = match message["scope"].as_str() {
                    Some(id) => PaletteScope::From(id.parse()?),
                    None => PaletteScope::NewTree,
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
    fn palette_items(&self, query: &str) -> (Vec<(Group, PaletteItem)>, usize) {
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
        let (mut items, first) =
            palette::results(query, hits, imported, &frame.bookmarks, &frame.forest);
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
        *self.suggested.borrow_mut() = self
            .host()
            .context("The browser closed.")?
            .suggestions(&frame.forest)?;
        self.call("start", &[start::value(frame, &self.suggested.borrow())]);
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
            "settle-tree" => ChromeIntent::SettleTree(TreeId(id("id")?)),
            "settle-current" => ChromeIntent::SettleCurrent,
            "select-tree" => ChromeIntent::SelectTree(TreeId(id("id")?)),
            "rename-tree" => {
                ChromeIntent::RenameTree(TreeId(id("id")?), message["name"].as_str()?.to_owned())
            }
            "restore" => ChromeIntent::Restore(id("id")?),
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
            "capture" => ChromeIntent::SetCaptureEnabled(message["on"].as_bool()?),
            "remember" => ChromeIntent::RememberSelection,
            "open-file" => ChromeIntent::OpenFile(message["file"].as_i64()?),
            "open-file-folder" => ChromeIntent::OpenFileFolder(message["file"].as_i64()?),
            "unlock-logins" => ChromeIntent::UnlockLogins,
            "save-login" => ChromeIntent::AnswerSave {
                offer: message["offer"].as_u64()?,
                save: message["save"].as_bool()?,
            },
            "fill-login" => ChromeIntent::FillLogin {
                offer: message["offer"].as_u64()?,
                index: index()?,
            },
            "pin-tree" => ChromeIntent::PinTree {
                tree: TreeId(id("id")?),
                pinned: message["pinned"].as_bool()?,
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
            _ => return None,
        })
    }
}

/// The chrome's own files, its font, and page icons, on the `canopy://` scheme. Only the
/// chrome view registers it, so pages cannot read it.
fn serve(request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let respond = |kind: &str, body: Cow<'static, [u8]>| {
        Response::builder()
            .header(CONTENT_TYPE, kind)
            .body(body)
            .expect("a static response builds")
    };
    match request.uri().path() {
        "/" | "/index.html" => respond("text/html", Cow::Borrowed(INDEX.as_bytes())),
        "/chrome.css" => respond("text/css", Cow::Borrowed(CSS.as_bytes())),
        "/chrome.js" => respond("text/javascript", Cow::Borrowed(SCRIPT.as_bytes())),
        "/inter.ttf" => respond("font/ttf", Cow::Borrowed(INTER)),
        "/icon" => {
            let query = request.uri().query().unwrap_or_default();
            let mut url = None;
            let mut opened = false;
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                match &*key {
                    "u" => url = Url::parse(&value).ok(),
                    "o" => opened = value == "1",
                    _ => {}
                }
            }
            let png = url
                .and_then(|url| FAVICONS.with(|slot| slot.borrow().upgrade()?.png(&url, opened)));
            match png {
                Some(png) => respond("image/png", Cow::Owned(png.to_vec())),
                None => Response::builder()
                    .status(404)
                    .body(Cow::Borrowed(&[][..]))
                    .expect("a static response builds"),
            }
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
