use std::{
    cell::{OnceCell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{address::web_url, browser::VisitId, chrome::Favicons};
use anyhow::{Context, Result};
use gtk::prelude::*;
use url::Url;
use webkit2gtk::{DownloadExt, URIRequestExt, WebContextExt, WebViewExt};

use super::PageEvent;
type Observer = RefCell<Option<Rc<dyn Fn(PageEvent)>>>;
type RecordDownload = dyn Fn(VisitId, &Path, u64, Option<&Url>) -> Result<()>;
type PageLookup = Box<dyn Fn(Option<&Url>) -> Result<Option<VisitId>>>;

struct DownloadSource {
    view: glib::WeakRef<webkit2gtk::WebView>,
    page: PageLookup,
}

pub struct PageContext {
    pub(crate) native: webkit2gtk::WebContext,
    pub favicons: Rc<Favicons>,
    record: RefCell<Option<Rc<RecordDownload>>>,
    observer: Observer,
    sources: RefCell<Vec<DownloadSource>>,
    session: OnceCell<soup::Session>,
}

impl PageContext {
    pub fn new(profile: &Path) -> Result<Rc<Self>> {
        std::fs::create_dir_all(profile)?;
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
        let state = Rc::new(Self {
            native: context,
            favicons: Favicons::new(&database),
            record: RefCell::new(None),
            observer: RefCell::new(None),
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
            let source = download
                .request()
                .and_then(|r| r.uri())
                .and_then(|uri| web_url(&uri).ok());
            let view = download.web_view();
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
                Ok(page) => save(download, context, page.flatten()),
                Err(_) => download.cancel(),
            }
        });
    }

    pub(crate) fn register_view(
        &self,
        view: &webkit2gtk::WebView,
        page: impl Fn(Option<&Url>) -> Result<Option<VisitId>> + 'static,
    ) -> glib::SignalHandlerId {
        let mut sources = self.sources.borrow_mut();
        sources.retain(|entry| entry.view.upgrade().is_some());
        sources.push(DownloadSource {
            view: view.downgrade(),
            page: Box::new(page),
        });
        drop(sources);
        let weak = Rc::downgrade(&self.favicons);
        view.connect_favicon_notify(move |page| {
            let Some(favicons) = weak.upgrade() else {
                return;
            };
            if let (Some(uri), Some(surface)) = (page.uri(), page.favicon()) {
                favicons.store(uri.to_string(), Some(&surface));
            }
        })
    }

    pub fn observe(&self, callback: impl Fn(PageEvent) + 'static) {
        *self.observer.borrow_mut() = Some(Rc::new(callback));
    }

    fn emit(&self, event: PageEvent) {
        let observer = self.observer.borrow().clone();
        if let Some(observer) = observer {
            observer(event);
        }
    }

    pub(super) fn record_downloads(
        &self,
        record: impl Fn(VisitId, &Path, u64, Option<&Url>) -> Result<()> + 'static,
    ) {
        *self.record.borrow_mut() = Some(Rc::new(record));
    }

    fn remember_download(&self, page: VisitId, path: &Path, source: Option<&Url>) -> Result<()> {
        let size = std::fs::metadata(path)?.len();
        let record = self
            .record
            .borrow()
            .clone()
            .context("Download writer is unavailable.")?;
        record(page, path, size, source)
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
fn save(download: &webkit2gtk::Download, state: Rc<PageContext>, page: Option<VisitId>) {
    let transfer = Rc::new(RefCell::new(Transfer::Deciding));
    let source = download
        .request()
        .and_then(|request| request.uri())
        .and_then(|uri| web_url(&uri).ok());
    {
        let transfer = transfer.clone();
        let state = state.clone();
        let emit = move |event| {
            state.emit(PageEvent::Download(event));
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
        if let (DownloadEvent::Saved(path), Some(page)) = (&event, page) {
            match state.remember_download(page, path, source.as_ref()) {
                Ok(()) => state.emit(PageEvent::Changed),
                Err(error) => state.emit(PageEvent::Error(format!(
                    "The file was saved, but could not be added to its page. {error}"
                ))),
            }
        }
        state.emit(PageEvent::Download(event));
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

#[derive(Clone, Copy, Debug)]
pub struct CookieApplyError;
impl std::fmt::Display for CookieApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Could not add imported cookies.")
    }
}
impl std::error::Error for CookieApplyError {}

impl PageContext {
    pub fn apply_cookie_batches(
        &self,
        batches: Vec<crate::cookies::Batch>,
        done: impl FnOnce(std::result::Result<usize, CookieApplyError>) + 'static,
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
            done: Box::new(done),
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

/// An HTTP answer kept outside WebKit's page cache. Redirects are returned unfollowed.
pub struct Fetched {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

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

impl PageContext {
    /// Reads live cookies for this URL, including HttpOnly cookies. The callback runs once,
    /// after this call returns, within 30 seconds including the cookie lookup.
    pub fn fetch(
        &self,
        url: &Url,
        cancel: Option<&gio::Cancellable>,
        done: impl FnOnce(Result<Fetched>) + 'static,
    ) {
        use gio::prelude::CancellableExt;
        use glib::translate::IntoGlib;
        use soup::prelude::SessionExt;
        use webkit2gtk::{CookieManagerExt, SettingsExt, WebsiteDataManagerExt};

        let session = self
            .session
            .get_or_init(|| {
                let session = soup::Session::new();
                let settings = self
                    .sources
                    .borrow()
                    .iter()
                    .filter_map(|s| s.view.upgrade())
                    .find_map(|v| WebViewExt::settings(&v))
                    .unwrap_or_default();
                if let Some(agent) = settings.user_agent() {
                    session.set_user_agent(&agent);
                }
                session
            })
            .clone();
        let fetch = Rc::new(Fetch {
            done: RefCell::new(Some(Box::new(done))),
            timer: RefCell::new(None),
            cancel: cancel.cloned().unwrap_or_default(),
        });
        let timeout = fetch.clone();
        *fetch.timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_secs(30),
            move || {
                timeout.timer.borrow_mut().take();
                timeout.cancel.cancel();
                timeout.finish(Err(anyhow::anyhow!("The fetch took too long to answer.")));
            },
        ));
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
                            body.map(|body| Fetched {
                                status: response.status().into_glib() as u16,
                                location: response
                                    .response_headers()
                                    .and_then(|h| h.one("Location"))
                                    .map(|s| s.to_string()),
                                body: body.to_vec(),
                            })
                            .map_err(Into::into),
                        );
                    },
                );
            });
        });
    }
}
