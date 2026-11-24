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
/// It owns cookies, side fetches, downloads, favicons and the sandbox flag.
pub trait Engine {
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
