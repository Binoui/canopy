use std::{
    cell::RefCell,
    path::Path,
    rc::{Rc, Weak},
};

use anyhow::{Context, Result};
use url::Url;

use super::{
    blocking::Blocking,
    engine::{webkit::WebKitEngine, DownloadEvent, DownloadSink, Engine, Fetched},
    PageEvent,
};
use crate::{browser::VisitId, chrome::Favicons};

type Observer = RefCell<Option<Rc<dyn Fn(PageEvent)>>>;
type RecordDownload = dyn Fn(VisitId, &Path, u64, Option<&Url>) -> Result<()>;

/// What every page of one window shares: the engine's profile, content blocking, and where
/// page-wide events go.
pub struct PageContext {
    engine: Rc<dyn Engine>,
    pub(super) blocking: Rc<Blocking>,
    record: RefCell<Option<Rc<RecordDownload>>>,
    observer: Observer,
}

impl PageContext {
    pub fn new(data_dir: &Path) -> Result<Rc<Self>> {
        let engine: Rc<dyn Engine> = WebKitEngine::new(data_dir)?;
        let state = Rc::new_cyclic(|weak: &Weak<Self>| Self {
            engine,
            blocking: Rc::new(Blocking::new(data_dir, weak.clone())),
            record: RefCell::new(None),
            observer: RefCell::new(None),
        });
        let sink: Weak<dyn DownloadSink> = Rc::downgrade(&state) as Weak<Self>;
        state.engine.report_downloads(sink);
        Ok(state)
    }

    pub(super) fn engine(&self) -> &Rc<dyn Engine> {
        &self.engine
    }

    /// TEMPORARY: the WebKit backend itself, for errand views, which still create WebKit
    /// views directly. Goes away once errands are made through the engine (engine layer PR 4).
    pub(super) fn temporary_webkit(&self) -> &WebKitEngine {
        self.engine
            .as_any()
            .downcast_ref()
            .expect("WebKit is the only engine")
    }

    pub fn favicons(&self) -> Rc<Favicons> {
        self.engine.favicons()
    }

    pub fn observe(&self, callback: impl Fn(PageEvent) + 'static) {
        *self.observer.borrow_mut() = Some(Rc::new(callback));
    }

    pub(super) fn emit(&self, event: PageEvent) {
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

    pub(super) fn fetch_without_cookies(
        &self,
        url: &Url,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Fetched>> + 'static {
        self.engine.fetch_without_cookies(url, limit)
    }
}

impl DownloadSink for PageContext {
    fn download(&self, event: DownloadEvent, page: Option<VisitId>, source: Option<&Url>) {
        if let (DownloadEvent::Saved(path), Some(page)) = (&event, page) {
            match self.remember_download(page, path, source) {
                Ok(()) => self.emit(PageEvent::Changed),
                Err(error) => self.emit(PageEvent::Error(format!(
                    "The file was saved, but could not be added to its page. {error}"
                ))),
            }
        }
        self.emit(PageEvent::Download(event));
    }
}
