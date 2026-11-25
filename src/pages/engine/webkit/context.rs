use std::{
    cell::{OnceCell, RefCell},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

use crate::{
    address::web_url,
    browser::VisitId,
    chrome::{FaviconSource, Favicons},
    pages::engine::{
        CookieApplyError, DownloadEvent, DownloadPage, DownloadSink, Engine, EngineView, Fetched,
        LocalFuture, RuleStore, ViewConfig,
    },
};
use anyhow::{Context, Result};
use gtk::prelude::*;
use url::Url;
use webkit2gtk::{DownloadExt, FaviconDatabaseExt, URIRequestExt, WebContextExt, WebViewExt};

struct DownloadSource {
    view: glib::WeakRef<gtk::Widget>,
    page: DownloadPage,
}

/// The WebKitGTK profile: one `WebContext` with its website data in `<data>/profile`.
pub(crate) struct WebKitEngine {
    native: webkit2gtk::WebContext,
    favicons: Rc<Favicons>,
    sink: RefCell<Option<Weak<dyn DownloadSink>>>,
    sources: RefCell<Vec<DownloadSource>>,
    session: OnceCell<soup::Session>,
}

/// WebKit's favicon database, read for `Favicons`.
struct FaviconDatabase(webkit2gtk::FaviconDatabase);

impl FaviconSource for FaviconDatabase {
    fn load(&self, url: &str, done: Box<dyn FnOnce(Option<gtk::cairo::Surface>)>) {
        self.0
            .favicon(url, None::<&gio::Cancellable>, move |result| {
                done(result.ok())
            });
    }
}

impl WebKitEngine {
    pub(crate) fn new(data_dir: &Path) -> Result<Rc<Self>> {
        let profile = data_dir.join("profile");
        std::fs::create_dir_all(&profile)?;
        use webkit2gtk::{
            CookieManagerExt, CookiePersistentStorage, WebsiteDataManager, WebsiteDataManagerExt,
        };
        let data = WebsiteDataManager::builder()
            .base_data_directory(profile.to_string_lossy())
            .base_cache_directory(profile.to_string_lossy())
            .build();
        data.cookie_manager()
            .context("The cookie manager is unavailable.")?
            .set_persistent_storage(
                &profile.join("cookies").to_string_lossy(),
                CookiePersistentStorage::Text,
            );
        let context = webkit2gtk::WebContext::builder()
            .website_data_manager(&data)
            .build();
        context.set_automation_allowed(false);
        context.set_use_system_appearance_for_scrollbars(false);
        // WebKitGTK 2.54 requests icons only through the context's database.
        #[allow(deprecated)]
        context.set_favicon_database_directory(Some(&profile.join("favicons").to_string_lossy()));
        let database = context
            .favicon_database()
            .context("The favicon database is unavailable.")?;
        let favicons = Favicons::new(Box::new(FaviconDatabase(database.clone())));
        let weak = Rc::downgrade(&favicons);
        database.connect_favicon_changed(move |_, uri, _| {
            if let Some(favicons) = weak.upgrade() {
                favicons.changed(uri);
            }
        });
        let state = Rc::new(Self {
            native: context,
            favicons,
            sink: RefCell::new(None),
            sources: RefCell::new(Vec::new()),
            session: OnceCell::new(),
        });
        state.connect_downloads();
        Ok(state)
    }

    fn connect_downloads(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.native.connect_download_started(move |_, download| {
            let Some(context) = weak.upgrade() else {
                return;
            };
            let Some(sink) = context.sink.borrow().as_ref().and_then(Weak::upgrade) else {
                return;
            };
            let source = download
                .request()
                .and_then(|r| r.uri())
                .and_then(|uri| web_url(&uri).ok());
            let view = download.web_view().map(|view| view.upcast::<gtk::Widget>());
            let page = context
                .sources
                .borrow()
                .iter()
                .find(|entry| {
                    entry
                        .view
                        .upgrade()
                        .is_some_and(|v| Some(&v) == view.as_ref())
                })
                .map(|entry| (entry.page)(source.as_ref()));
            match page.transpose() {
                Ok(page) => save(download, sink, page.flatten()),
                Err(_) => download.cancel(),
            }
        });
    }

    fn add_source(&self, view: &gtk::Widget, page: DownloadPage) {
        let mut sources = self.sources.borrow_mut();
        sources.retain(|entry| entry.view.upgrade().is_some());
        sources.push(DownloadSource {
            view: view.downgrade(),
            page,
        });
    }
}

/// One download's progress through WebKit's signals, which end with `finished` either way.
enum Transfer {
    Deciding,
    Saving {
        staging: crate::download::Staging,
        name: String,
    },
    Failed {
        name: Option<String>,
        reason: String,
    },
    Over,
}

/// Saves `download` in the user's download folder, notes it on the page it came from and
/// reports it. WebKit's default would trust the server's file name.
fn save(download: &webkit2gtk::Download, sink: Rc<dyn DownloadSink>, page: Option<VisitId>) {
    let transfer = Rc::new(RefCell::new(Transfer::Deciding));
    let source = download
        .request()
        .and_then(|request| request.uri())
        .and_then(|uri| web_url(&uri).ok());
    {
        let transfer = transfer.clone();
        let sink = sink.clone();
        let source = source.clone();
        let emit = move |event| {
            sink.download(event, page, source.as_ref());
        };
        download.connect_decide_destination(move |download, suggested| {
            let name = crate::download::file_name(suggested);
            match crate::download::Staging::new(&download_directory()) {
                Ok(staging) => {
                    download.set_allow_overwrite(false);
                    download.set_destination(&staging.file().to_string_lossy());
                    emit(DownloadEvent::Started(name.clone()));
                    transfer.replace(Transfer::Saving { staging, name });
                }
                Err(error) => {
                    transfer.replace(Transfer::Over);
                    download.cancel();
                    emit(DownloadEvent::Failed {
                        name: Some(name),
                        reason: error.to_string(),
                    });
                }
            }
            true
        });
    }
    {
        let transfer = transfer.clone();
        download.connect_failed(move |_, error| {
            let reason = error.message().to_owned();
            let next = match transfer.replace(Transfer::Over) {
                Transfer::Deciding => Transfer::Failed { name: None, reason },
                Transfer::Saving { name, .. } => Transfer::Failed {
                    name: Some(name),
                    reason,
                },
                done => done,
            };
            transfer.replace(next);
        });
    }
    download.connect_finished(move |download| {
        let event = match transfer.replace(Transfer::Over) {
            Transfer::Saving { name, .. } if truncated(download) => DownloadEvent::Failed {
                name: Some(name),
                reason: "The connection closed before the whole file arrived.".into(),
            },
            Transfer::Saving { staging, name } => match staging.publish(&name) {
                Ok(path) => DownloadEvent::Saved(path),
                Err(error) => DownloadEvent::Failed {
                    name: Some(name),
                    reason: error.to_string(),
                },
            },
            Transfer::Failed { name, reason } => DownloadEvent::Failed { name, reason },
            Transfer::Deciding | Transfer::Over => return,
        };
        sink.download(event, page, source.as_ref());
    });
}

/// WebKit reports a transfer cut short after its headers as finished. A compressed body's
/// length counts encoded bytes, so only an unencoded one can be checked.
fn truncated(download: &webkit2gtk::Download) -> bool {
    use webkit2gtk::URIResponseExt;
    let Some(response) = download.response() else {
        return false;
    };
    let encoded = response
        .http_headers()
        .and_then(|headers| headers.one("Content-Encoding"))
        .is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity"));
    !encoded && download.received_data_length() < response.content_length()
}

fn download_directory() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads)
        .unwrap_or_else(|| glib::home_dir().join("Downloads"))
}

impl WebKitEngine {
    fn apply_cookie_batches(
        &self,
        batches: Vec<crate::cookies::Batch>,
        done: Box<dyn FnOnce(std::result::Result<usize, CookieApplyError>)>,
    ) {
        use webkit2gtk::WebsiteDataManagerExt;
        let manager = self
            .native
            .website_data_manager()
            .and_then(|d| d.cookie_manager());
        let Some(manager) = manager else {
            done(Err(CookieApplyError));
            return;
        };
        CookieDrain {
            manager,
            batches: batches.into(),
            current: None,
            count: 0,
            done,
        }
        .advance();
    }
}

struct CookieDrain {
    manager: webkit2gtk::CookieManager,
    batches: std::collections::VecDeque<crate::cookies::Batch>,
    current: Option<(
        crate::cookies::Batch,
        std::vec::IntoIter<crate::cookies::Cookie>,
    )>,
    count: usize,
    done: Box<dyn FnOnce(std::result::Result<usize, CookieApplyError>)>,
}
impl CookieDrain {
    fn advance(mut self) {
        use webkit2gtk::CookieManagerExt;
        loop {
            if self.current.is_none() {
                let Some(mut batch) = self.batches.pop_front() else {
                    (self.done)(Ok(self.count));
                    return;
                };
                let cookies = std::mem::take(&mut batch.cookies).into_iter();
                self.current = Some((batch, cookies));
            }
            let (_, cookies) = self.current.as_mut().expect("current batch");
            if let Some(cookie) = cookies.next() {
                // A cookie WebKit cannot represent is skipped, so it never blocks later imports.
                let Ok(mut native) = soup_cookie(&cookie) else {
                    continue;
                };
                self.manager.clone().add_cookie(
                    &mut native,
                    None::<&gio::Cancellable>,
                    move |result| {
                        if result.is_err() {
                            (self.done)(Err(CookieApplyError));
                        } else {
                            self.count += 1;
                            self.advance();
                        }
                    },
                );
                return;
            }
            let (batch, _) = self.current.take().expect("completed batch");
            if batch.accept().is_err() {
                (self.done)(Err(CookieApplyError));
                return;
            }
        }
    }
}

fn soup_cookie(
    cookie: &crate::cookies::Cookie,
) -> std::result::Result<soup::Cookie, CookieApplyError> {
    use crate::cookies::SameSite;
    let mut native = soup::Cookie::new(
        &cookie.name,
        cookie.value.expose(),
        &cookie.domain.dotted(),
        &cookie.path,
        -1,
    );
    if let Some(expires) = cookie.expires {
        let expires = glib::DateTime::from_unix_utc(expires).map_err(|_| CookieApplyError)?;
        native.set_expires(&expires);
    }
    native.set_secure(cookie.secure);
    native.set_http_only(cookie.http_only);
    native.set_same_site_policy(match cookie.same_site {
        SameSite::None => soup::SameSitePolicy::None,
        SameSite::Lax => soup::SameSitePolicy::Lax,
        SameSite::Strict => soup::SameSitePolicy::Strict,
    });
    Ok(native)
}

fn fetched(message: &soup::Message, body: Vec<u8>) -> Fetched {
    use glib::translate::IntoGlib;
    Fetched {
        status: message.status().into_glib() as u16,
        location: message
            .response_headers()
            .and_then(|h| h.one("Location"))
            .map(|s| s.to_string()),
        body,
    }
}

const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

type FetchDone = Box<dyn FnOnce(Result<Fetched>)>;

struct Fetch {
    done: RefCell<Option<FetchDone>>,
    timer: RefCell<Option<glib::SourceId>>,
    cancel: gio::Cancellable,
}

impl Fetch {
    fn finish(&self, result: Result<Fetched>) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        let done = self.done.borrow_mut().take();
        if let Some(done) = done {
            done(result);
        }
    }
}

impl WebKitEngine {
    fn fetch_session(&self) -> &soup::Session {
        use soup::prelude::SessionExt;
        use webkit2gtk::SettingsExt;

        self.session.get_or_init(|| {
            let session = soup::Session::new();
            let settings = self
                .sources
                .borrow()
                .iter()
                .filter_map(|s| s.view.upgrade())
                .filter_map(|v| v.downcast::<webkit2gtk::WebView>().ok())
                .find_map(|v| WebViewExt::settings(&v))
                .unwrap_or_default();
            if let Some(agent) = settings.user_agent() {
                session.set_user_agent(&agent);
            }
            session
        })
    }

    fn fetch_without_cookies(
        &self,
        url: &Url,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Fetched>> + 'static {
        use gio::prelude::InputStreamExt;
        use glib::prelude::StaticType;
        use soup::prelude::SessionExt;

        let message = soup::Message::new("GET", url.as_str());
        let session = self.fetch_session().clone();
        async move {
            let priority = glib::Priority::DEFAULT;
            let message = message?;
            message.set_flags(soup::MessageFlags::NO_REDIRECT);
            message.disable_feature(soup::CookieJar::static_type());
            message.disable_feature(soup::AuthManager::static_type());
            glib::future_with_timeout(FETCH_TIMEOUT, async {
                let stream = session.send_future(&message, priority).await?;
                let mut body = Vec::new();
                loop {
                    let chunk = stream
                        .read_bytes_future((limit + 1 - body.len()).min(8192), priority)
                        .await?;
                    if chunk.is_empty() {
                        return Ok(fetched(&message, body));
                    }
                    anyhow::ensure!(
                        body.len() + chunk.len() <= limit,
                        "The fetch body is too large."
                    );
                    body.extend_from_slice(&chunk);
                }
            })
            .await?
        }
    }

    /// Reads live cookies for this URL, including HttpOnly cookies. The callback runs once,
    /// after this call returns, within 30 seconds including the cookie lookup.
    fn fetch(
        &self,
        url: &Url,
        cancel: Option<&gio::Cancellable>,
        done: Box<dyn FnOnce(Result<Fetched>)>,
    ) {
        use gio::prelude::CancellableExt;
        use soup::prelude::SessionExt;
        use webkit2gtk::{CookieManagerExt, WebsiteDataManagerExt};

        let session = self.fetch_session().clone();
        let fetch = Rc::new(Fetch {
            done: RefCell::new(Some(done)),
            timer: RefCell::new(None),
            cancel: cancel.cloned().unwrap_or_default(),
        });
        let timeout = fetch.clone();
        *fetch.timer.borrow_mut() = Some(glib::timeout_add_local_once(FETCH_TIMEOUT, move || {
            timeout.timer.borrow_mut().take();
            timeout.cancel.cancel();
            timeout.finish(Err(anyhow::anyhow!("The fetch took too long to answer.")));
        }));
        let manager = self
            .native
            .website_data_manager()
            .and_then(|d| d.cookie_manager());
        let url = url.clone();
        glib::idle_add_local_once(move || {
            let Some(manager) = manager else {
                fetch.finish(Err(anyhow::anyhow!("The cookie manager is unavailable.")));
                return;
            };
            let cancel = fetch.cancel.clone();
            manager.cookies(url.clone().as_str(), Some(&cancel), move |cookies| {
                let result = (|| {
                    let header = cookies?
                        .iter_mut()
                        .filter_map(|c| c.to_cookie_header())
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join("; ");
                    let message = soup::Message::new("GET", url.as_str())?;
                    message.set_flags(soup::MessageFlags::NO_REDIRECT);
                    if !header.is_empty() {
                        message
                            .request_headers()
                            .context("Request headers are unavailable.")?
                            .append("Cookie", &header);
                    }
                    Ok(message)
                })();
                let message: soup::Message = match result {
                    Ok(message) => message,
                    Err(error) => {
                        fetch.finish(Err(error));
                        return;
                    }
                };
                if fetch.done.borrow().is_none() {
                    return;
                }
                let response = message.clone();
                let cancel = fetch.cancel.clone();
                session.send_and_read_async(
                    &message,
                    glib::Priority::DEFAULT,
                    Some(&cancel),
                    move |body| {
                        fetch.finish(
                            body.map(|body| fetched(&response, body.to_vec()))
                                .map_err(Into::into),
                        );
                    },
                );
            });
        });
    }
}

impl Engine for WebKitEngine {
    fn create_view(&self, config: ViewConfig) -> Result<Rc<dyn EngineView>> {
        Ok(super::view::WebKitView::new(&self.native, config)?)
    }

    fn favicons(&self) -> Rc<Favicons> {
        self.favicons.clone()
    }

    fn sandbox_enabled(&self) -> bool {
        self.native.is_sandbox_enabled()
    }

    fn apply_cookie_batches(
        &self,
        batches: Vec<crate::cookies::Batch>,
        done: Box<dyn FnOnce(std::result::Result<usize, CookieApplyError>)>,
    ) {
        WebKitEngine::apply_cookie_batches(self, batches, done);
    }

    fn fetch(
        &self,
        url: &Url,
        cancel: Option<&gio::Cancellable>,
        done: Box<dyn FnOnce(Result<Fetched>)>,
    ) {
        WebKitEngine::fetch(self, url, cancel, done);
    }

    fn fetch_without_cookies(&self, url: &Url, limit: usize) -> LocalFuture<Result<Fetched>> {
        Box::pin(WebKitEngine::fetch_without_cookies(self, url, limit))
    }

    fn report_downloads(&self, sink: Weak<dyn DownloadSink>) {
        *self.sink.borrow_mut() = Some(sink);
    }

    fn attribute_downloads(&self, view: &gtk::Widget, page: DownloadPage) {
        self.add_source(view, page);
    }

    fn refuse_downloads(&self, view: &gtk::Widget) {
        self.add_source(
            view,
            Box::new(|_| anyhow::bail!("An errand cannot download.")),
        );
    }

    fn open_rule_store(&self, directory: &Path) -> Rc<dyn RuleStore> {
        super::rules::open_store(directory)
    }
}
