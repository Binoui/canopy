//! The web engine behind Canopy's pages, seen through engine-neutral types.
//!
//! WebKitGTK is the only engine for now; it lives in `webkit`. No type from `webkit2gtk`,
//! `javascriptcore` or `soup` appears in a signature here.

use std::{any::Any, future::Future, path::PathBuf, pin::Pin, rc::Rc, rc::Weak};

use anyhow::Result;
use url::Url;

use crate::{browser::VisitId, chrome::Favicons};

pub(crate) mod webkit;

/// Finds the page a download belongs to from the address it came from. An error refuses
/// the download.
pub type DownloadPage = Box<dyn Fn(Option<&Url>) -> Result<Option<VisitId>>>;

/// Where the engine reports downloads.
pub trait DownloadSink {
    /// One step of a download. `page` and `source` say where it came from, as found when it
    /// started.
    fn download(&self, event: DownloadEvent, page: Option<VisitId>, source: Option<&Url>);
}

/// The shared side of a web engine: one profile per Canopy window, made from the data folder.
/// It owns cookies, side fetches, downloads, favicons and the sandbox flag, and it makes the
/// views that show pages.
pub trait Engine {
    /// A new view showing nothing yet, set up as `config` says.
    fn create_view(&self, config: ViewConfig) -> Result<Rc<dyn EngineView>>;

    /// The page icons this engine knows.
    fn favicons(&self) -> Rc<Favicons>;

    /// Whether web content runs confined.
    fn sandbox_enabled(&self) -> bool;

    /// Adds imported cookies to the profile, batch by batch. `done` runs once.
    fn apply_cookie_batches(
        &self,
        batches: Vec<crate::cookies::Batch>,
        done: Box<dyn FnOnce(std::result::Result<usize, CookieApplyError>)>,
    );

    /// Fetches `url` outside any page with the profile's cookies, HttpOnly ones included, and
    /// without following redirects. `done` runs once, after this call returns.
    fn fetch(
        &self,
        url: &Url,
        cancel: Option<&gio::Cancellable>,
        done: Box<dyn FnOnce(Result<Fetched>)>,
    );

    /// Fetches `url` with no cookies, no authentication and no redirects, reading at most
    /// `limit` bytes of body.
    fn fetch_without_cookies(&self, url: &Url, limit: usize) -> LocalFuture<Result<Fetched>>;

    /// Sends every download to `sink` while it lives.
    fn report_downloads(&self, sink: Weak<dyn DownloadSink>);

    /// Downloads started in `view` belong to the page `page` finds.
    fn attribute_downloads(&self, view: &gtk::Widget, page: DownloadPage);

    /// Downloads started in `view` are cancelled.
    fn refuse_downloads(&self, view: &gtk::Widget);

    /// For the temporary way back to the WebKit backend; see `PageContext::temporary_webkit`.
    fn as_any(&self) -> &dyn Any;
}

pub type LocalFuture<T> = Pin<Box<dyn Future<Output = T>>>;

/// One view showing web pages: its loads, history, address and title, scripts, find, zoom,
/// sound, pictures, saved session and its GTK widget.
///
/// Getters answer at once from the engine's current state, as Canopy's load bookkeeping
/// needs. Calls that finish later take a callback that runs on the main loop.
pub trait EngineView {
    /// The widget that shows the page. Canopy places, shows, hides and focuses it.
    fn widget(&self) -> &gtk::Widget;

    fn load_uri(&self, uri: &str);
    /// Shows `html` as a page of its own, with no address.
    fn load_html(&self, html: &str);
    fn stop_loading(&self);

    fn uri(&self) -> Option<String>;
    fn title(&self) -> Option<String>;
    fn is_loading(&self) -> bool;
    /// How far the current load has come, from 0 to 1.
    fn estimated_progress(&self) -> f64;

    /// The view's back/forward list as it stands now.
    fn history(&self) -> History;
    /// The entry `offset` steps from the current one: negative is back, 0 is current.
    fn history_nth(&self, offset: i32) -> Option<EntryId>;
    /// Goes to a listed entry. An entry the list no longer holds is ignored.
    fn go_to(&self, entry: EntryId);

    /// The view's history and page state, to give back to `restore_session_state` in a new
    /// view. Only this engine understands the bytes.
    fn session_state(&self) -> Option<glib::Bytes>;
    fn restore_session_state(&self, state: &glib::Bytes);

    /// Runs `script` in `world`. `done`, when given, gets the script's value as text.
    fn evaluate(
        &self,
        script: &str,
        world: World,
        cancel: Option<&gio::Cancellable>,
        done: Option<ScriptDone>,
    );
    /// Runs `body` as an async function in `world`. `done` learns whether it resolved.
    fn call_async(
        &self,
        body: &str,
        world: World,
        cancel: Option<&gio::Cancellable>,
        done: Box<dyn FnOnce(std::result::Result<(), glib::Error>)>,
    );
    /// Types `text` where the page's focus is, as a paste would.
    fn insert_text(&self, text: &str);

    /// Looks for `text` in the page, ignoring case, wrapping at the end, every match counted.
    fn find(&self, text: &str);
    fn find_next(&self);
    fn find_previous(&self);
    fn find_finish(&self);

    /// The page zoom, 1.0 for 100 %. It scales the whole page, not only its text.
    fn zoom(&self) -> f64;
    fn set_zoom(&self, level: f64);

    fn is_playing_audio(&self) -> bool;
    fn is_muted(&self) -> bool;
    fn set_muted(&self, muted: bool);

    /// A picture of the visible part of the page.
    fn snapshot(
        &self,
        cancel: Option<&gio::Cancellable>,
        done: Box<dyn FnOnce(std::result::Result<gtk::cairo::Surface, glib::Error>)>,
    );

    /// Where the view sends its events from now on, or `None` to stop sending them. Events
    /// arrive synchronously, while the engine is still inside the step they report, in the
    /// order the engine emits them.
    fn set_observer(&self, observer: Option<Observer>);

    /// For test accessors and the temporary way back to the WebKit view.
    fn as_any(&self) -> &dyn Any;
}

/// Hears a view's events. Returning `true` says Canopy handled the event: the engine then
/// skips its own default for it, as a WebKit signal handler returning `true` does. Events
/// without a default ignore the answer unless they say otherwise.
pub type Observer = Rc<dyn Fn(ViewEvent) -> bool>;

/// Something a view reports. One engine signal is one event, in the engine's order.
pub enum ViewEvent {
    /// A stage of the main frame's load.
    Load(LoadEvent),
    /// The main frame's load failed. `true` keeps the engine's error page away.
    LoadFailed(LoadFailure),
    /// The main frame's load failed on the server's certificate. `true` keeps the engine's
    /// error page away.
    LoadFailedWithTlsErrors,
    /// The view's address changed.
    UriChanged,
    TitleChanged,
    /// The page's icon changed. Carries the address and the icon as they were then.
    FaviconChanged {
        uri: Option<String>,
        favicon: Option<gtk::cairo::Surface>,
    },
    /// `is_loading` changed.
    LoadingChanged,
    /// `estimated_progress` changed.
    ProgressChanged,
    /// `is_playing_audio` changed.
    PlayingAudioChanged,
    /// The back/forward list changed: an entry was added, entries were removed, or both.
    /// `current` is the list's current entry after the change.
    HistoryChanged {
        added: Option<HistoryItem>,
        removed: Vec<EntryId>,
        current: Option<EntryId>,
    },
    /// The engine asks whether to go ahead with a navigation, a new window or a response.
    /// `true` says Canopy answered, or will answer later through the decision.
    Decide(PolicyDecision, PolicyDecisionType),
    /// The main frame's main resource started loading with this request. `true` asks for
    /// that resource's later requests (redirects) as `MainRequestSent`.
    MainResourceStarted(Request),
    /// The main frame's main resource sent a new request, after a redirect.
    MainRequestSent(Request),
    /// A script posted a message to a handler named in `ViewConfig::message_handlers`.
    /// `body` is the message's value as text.
    ScriptMessage {
        name: String,
        body: Option<String>,
    },
    /// The page opens an alert, confirm, prompt or leave-page question. `true` keeps the
    /// engine's own dialog away.
    ScriptDialog(Box<dyn ScriptDialog>),
    /// The page asks for a permission. `true` keeps the engine's own question away.
    PermissionRequest(Box<dyn Responder>),
    /// The page asks for files. `true` keeps the engine's own chooser away.
    FileChooser(Box<dyn Responder>),
    /// The page asks to print. `true` keeps the engine's print dialog away.
    Print,
    /// The page asks for fullscreen. `true` refuses it.
    EnterFullscreen,
    /// The page leaves fullscreen. `true` keeps the engine from restoring the window.
    LeaveFullscreen,
    /// The process drawing the page ended.
    WebProcessTerminated,
    /// The server asks for a sign-in. `true` keeps the engine's own dialog away.
    Authenticate(Box<dyn Responder>),
    /// The page asks to close its window.
    Close,
    /// Find found `count` matches.
    FoundText(u32),
    /// Find found nothing.
    FailedToFindText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadEvent {
    Started,
    Redirected,
    Committed,
    Finished,
}

pub struct LoadFailure {
    pub kind: LoadFailureKind,
    /// The engine's own words for the failure.
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadFailureKind {
    /// The load was stopped, by Canopy or by a newer load.
    Cancelled,
    /// A policy decision ended the load, as when a response becomes a download.
    InterruptedByPolicyChange,
    Other,
}

/// An answer to a request the page made: a permission, a file choice or a sign-in.
pub trait Responder {
    /// Refuses or cancels the request.
    fn refuse(&self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptDialogType {
    Alert,
    Confirm,
    Prompt,
    BeforeUnloadConfirm,
}

pub trait ScriptDialog {
    fn dialog_type(&self) -> ScriptDialogType;
    /// Answers a confirm or leave-page question.
    fn confirm_set_confirmed(&self, confirmed: bool);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyDecisionType {
    NavigationAction,
    NewWindowAction,
    Response,
}

/// How a policy decision is answered. The engine waits until one of these is called, so a
/// decision may be kept and answered later.
pub trait Decide {
    fn use_(&self);
    fn ignore(&self);
    /// Saves the response as a file instead of showing it.
    fn download(&self);
}

/// A navigation or response the engine waits on, with what Canopy reads to decide. Clones
/// share one answer.
#[derive(Clone)]
pub struct PolicyDecision {
    answer: Rc<dyn Decide>,
    navigation_action: Option<NavigationAction>,
    response: Option<ResponseDetails>,
}

impl PolicyDecision {
    pub fn new(
        answer: Rc<dyn Decide>,
        navigation_action: Option<NavigationAction>,
        response: Option<ResponseDetails>,
    ) -> Self {
        Self {
            answer,
            navigation_action,
            response,
        }
    }

    pub fn use_(&self) {
        self.answer.use_();
    }

    pub fn ignore(&self) {
        self.answer.ignore();
    }

    pub fn download(&self) {
        self.answer.download();
    }

    /// What a navigation or new-window decision asks for.
    pub fn navigation_action(&self) -> Option<&NavigationAction> {
        self.navigation_action.as_ref()
    }

    /// What a response decision received.
    pub fn response(&self) -> Option<&ResponseDetails> {
        self.response.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationType {
    LinkClicked,
    FormSubmitted,
    BackForward,
    Reload,
    FormResubmitted,
    Other,
}

/// A navigation as the engine saw it start.
#[derive(Clone, Debug)]
pub struct NavigationAction {
    navigation_type: NavigationType,
    /// The target frame's name, when the navigation names one.
    frame_name: Option<String>,
    modifiers: gdk::ModifierType,
    /// The mouse button that started it, 0 for none.
    mouse_button: u32,
    user_gesture: bool,
    request: Option<Request>,
}

impl NavigationAction {
    pub fn navigation_type(&self) -> NavigationType {
        self.navigation_type
    }
    pub fn modifiers(&self) -> gdk::ModifierType {
        self.modifiers
    }
    pub fn frame_name(&self) -> Option<&str> {
        self.frame_name.as_deref()
    }
    pub fn mouse_button(&self) -> u32 {
        self.mouse_button
    }
    pub fn is_user_gesture(&self) -> bool {
        self.user_gesture
    }
    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    uri: Option<String>,
    http_method: Option<String>,
}

impl Request {
    pub fn uri(&self) -> Option<String> {
        self.uri.clone()
    }
    pub fn http_method(&self) -> Option<String> {
        self.http_method.clone()
    }
}

/// A response the engine received, before it shows it.
#[derive(Clone, Debug)]
pub struct ResponseDetails {
    main_frame_main_resource: bool,
    request: Option<Request>,
    response: Option<Response>,
    /// Whether the engine can show this MIME type itself.
    mime_type_supported: bool,
}

impl ResponseDetails {
    pub fn is_main_frame_main_resource(&self) -> bool {
        self.main_frame_main_resource
    }
    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }
    pub fn response(&self) -> Option<&Response> {
        self.response.as_ref()
    }
    pub fn is_mime_type_supported(&self) -> bool {
        self.mime_type_supported
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    status_code: u32,
    /// The disposition type of the Content-Disposition header, such as `attachment`.
    content_disposition: Option<String>,
}

impl Response {
    pub fn status_code(&self) -> u32 {
        self.status_code
    }
    pub fn content_disposition(&self) -> Option<&str> {
        self.content_disposition.as_deref()
    }
}

/// Gets a script's value as text, or the error it threw.
pub type ScriptDone = Box<dyn FnOnce(std::result::Result<String, glib::Error>)>;

/// Where a script runs. Canopy's own scripts run in isolated worlds, which share the page's
/// document but not its JavaScript, so the page cannot see or change them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum World {
    /// The page's own JavaScript world.
    Page,
    /// Reading memory, scroll, find and sign-in helpers.
    ReadingMemory,
    /// Waits for the incoming page to paint before the outgoing one goes away.
    Hold,
    /// Reads the shown page's text for `canopy session`.
    Session,
}

/// How a new view is set up.
pub struct ViewConfig {
    /// Scripts the view adds to every document, in this order.
    pub scripts: Vec<UserScript>,
    /// Names scripts may post messages to, each in its world.
    pub message_handlers: Vec<MessageHandler>,
    /// WebGL and Web Audio.
    pub media: bool,
    /// Media may play without a click.
    pub autoplay: bool,
    /// Pages left stay in memory for a quick return.
    pub page_cache: bool,
    /// The web inspector.
    pub developer_extras: bool,
}

pub struct UserScript {
    pub source: &'static str,
    pub world: World,
    pub frames: Frames,
    pub time: ScriptTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frames {
    Top,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptTime {
    /// Before the document's own scripts.
    DocumentStart,
}

pub struct MessageHandler {
    pub name: &'static str,
    pub world: World,
}

/// One entry of a view's back/forward list. The same entry keeps the same id while the list
/// holds it; ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntryId(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryItem {
    pub id: EntryId,
    pub uri: Option<String>,
}

/// A view's back/forward list at one moment.
#[derive(Clone, Debug, Default)]
pub struct History {
    /// Nearest first, as the engine lists them.
    pub back: Vec<HistoryItem>,
    pub current: Option<HistoryItem>,
    /// Nearest first.
    pub forward: Vec<HistoryItem>,
    /// How many entries the engine says the list holds.
    pub length: u32,
}

impl History {
    /// Every entry the list holds: back, forward, then current.
    pub fn live(&self) -> Vec<EntryId> {
        self.back
            .iter()
            .chain(&self.forward)
            .chain(&self.current)
            .map(|item| item.id)
            .collect()
    }

    pub fn current_id(&self) -> Option<EntryId> {
        self.current.as_ref().map(|item| item.id)
    }

    pub fn find(&self, id: EntryId) -> Option<&HistoryItem> {
        self.back
            .iter()
            .chain(&self.forward)
            .chain(&self.current)
            .find(|item| item.id == id)
    }
}

pub enum DownloadEvent {
    /// Carries the file name the download asks for. The saved name can differ.
    Started(String),
    Saved(PathBuf),
    Failed {
        name: Option<String>,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct CookieApplyError;
impl std::fmt::Display for CookieApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Could not add imported cookies.")
    }
}
impl std::error::Error for CookieApplyError {}

/// An HTTP answer kept outside the engine's page cache. Redirects are returned unfollowed.
pub struct Fetched {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}
