mod lists;

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use url::Url;

use crate::{
    address::{web_url, WebOrigin},
    browser::Document,
};

use super::{
    context::PageContext,
    engine::{Engine, EngineView, LoadEvent, RuleSet, RuleStore, ViewRules},
    PageEvent,
};
use lists::{Manifest, Record, Source};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const START_WAIT: Duration = Duration::from_secs(1);
const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct Blocking {
    context: Weak<PageContext>,
    engine: Rc<dyn Engine>,
    folder: PathBuf,
    settings_file: PathBuf,
    settings: RefCell<Settings>,
    sources: Vec<Source>,
    store: OnceCell<Rc<dyn RuleStore>>,
    activity: RefCell<Activity>,
    start_gate: RefCell<Option<glib::SourceId>>,
    compiled: RefCell<BTreeMap<String, Installed>>,
    pages: RefCell<Vec<Followed>>,
    daily: RefCell<Option<glib::SourceId>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Settings {
    on: bool,
    unblocked: BTreeSet<WebOrigin>,
}

impl Settings {
    fn load(file: &Path) -> Self {
        let value: Value = fs::read(file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let unblocked = value["unblocked"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|site| {
                let raw = site.as_str()?;
                let origin = WebOrigin::of(&web_url(raw).ok()?)?;
                (origin.as_str() == raw).then_some(origin)
            })
            .collect();
        Self {
            on: value["on"].as_bool().unwrap_or(true),
            unblocked,
        }
    }

    fn save(&self, file: &Path) -> Result<()> {
        let unblocked: Vec<_> = self.unblocked.iter().map(WebOrigin::as_str).collect();
        let bytes = serde_json::to_vec(&json!({"on": self.on, "unblocked": unblocked}))?;
        crate::atomic_file::write(file, bytes)
            .context("Could not save content blocking preferences.")
    }
}

#[derive(Clone, Debug)]
pub struct BlockingView {
    pub on: bool,
    pub detail: Option<&'static str>,
    pub site: Option<SiteView>,
}

#[derive(Clone, Debug)]
pub struct SiteView {
    pub(crate) origin: WebOrigin,
    pub(crate) document: Document,
    pub(crate) on: bool,
}

impl Default for BlockingView {
    fn default() -> Self {
        Self {
            on: true,
            detail: None,
            site: None,
        }
    }
}

enum Activity {
    Dormant,
    Opening,
    Idle,
    Refreshing(glib::JoinHandle<()>),
}

struct Installed {
    record: Record,
    rules: RuleSet,
}

struct Followed {
    rules: Box<dyn ViewRules>,
    top: TopSite,
    holds: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Site {
    Web(WebOrigin),
    #[default]
    Other,
}

impl Site {
    fn of(url: Option<&Url>) -> Self {
        url.and_then(WebOrigin::of).map_or(Self::Other, Self::Web)
    }
}

#[derive(Default)]
struct TopSite {
    committed: Site,
    loading: Option<Site>,
}

enum Top {
    Loading(Site),
    Committed(Site),
    Ended,
}

impl TopSite {
    fn report(&mut self, top: Top) {
        match top {
            Top::Loading(site) => self.loading = Some(site),
            Top::Committed(site) => {
                self.committed = site;
                self.loading = None;
            }
            Top::Ended => self.loading = None,
        }
    }

    fn effective(&self) -> &Site {
        self.loading.as_ref().unwrap_or(&self.committed)
    }
}

impl Blocking {
    pub(super) fn new(data_dir: &Path, context: Weak<PageContext>, engine: Rc<dyn Engine>) -> Self {
        let settings_file = data_dir.join("content-blocking");
        Self {
            context,
            engine,
            folder: data_dir.join("blocklists"),
            settings: RefCell::new(Settings::load(&settings_file)),
            settings_file,
            sources: lists::configured(),
            store: OnceCell::new(),
            activity: RefCell::new(Activity::Dormant),
            start_gate: RefCell::default(),
            compiled: RefCell::default(),
            pages: RefCell::default(),
            daily: RefCell::default(),
        }
    }

    pub(super) fn start(self: &Rc<Self>) {
        if !self.settings.borrow().on || self.sources.is_empty() {
            return;
        }
        let weak = Rc::downgrade(self);
        *self.daily.borrow_mut() = Some(glib::timeout_add_local(DAY, move || {
            let Some(blocking) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            blocking.check(SystemTime::now());
            glib::ControlFlow::Continue
        }));
        if !matches!(*self.activity.borrow(), Activity::Dormant) {
            self.check(SystemTime::now());
            return;
        }
        let store = self
            .store
            .get_or_init(|| self.engine.open_rule_store(&self.folder.join("compiled")))
            .clone();
        let manifest = Manifest::load(&self.folder);
        let records: Vec<_> = self
            .sources
            .iter()
            .filter_map(|source| Some((source.id.clone(), manifest.0.get(&source.id)?.clone())))
            .collect();
        *self.activity.borrow_mut() = Activity::Opening;
        if !records.is_empty() {
            let weak = Rc::downgrade(self);
            *self.start_gate.borrow_mut() = Some(glib::timeout_add_local(START_WAIT, move || {
                if let Some(blocking) = weak.upgrade() {
                    blocking.release_startup();
                }
                glib::ControlFlow::Break
            }));
        }
        let weak = Rc::downgrade(self);
        let sources_dir = self.folder.join("sources");
        glib::MainContext::default().spawn_local(async move {
            let started = Instant::now();
            let mut loaded = BTreeMap::new();
            for (id, record) in records {
                match store.load(record.identifier.clone()).await {
                    Ok(rules) => {
                        loaded.insert(id, Installed { record, rules });
                    }
                    Err(error) => eprintln!("Canopy: could not open stored list {id}: {error}"),
                }
            }
            let elapsed = started.elapsed();
            let Some(blocking) = weak.upgrade() else {
                return;
            };
            eprintln!(
                "Canopy: content blocking opened {} stored lists in {:.3} ms",
                loaded.len(),
                ms(elapsed)
            );
            *blocking.compiled.borrow_mut() = loaded;
            blocking.refit();
            blocking.release_startup();
            drop(blocking);

            let named: BTreeSet<_> = manifest
                .0
                .into_values()
                .map(|record| record.identifier)
                .collect();
            if let Some(identifiers) = store.identifiers().await {
                for identifier in identifiers {
                    if !named.contains(&identifier) {
                        store.remove(identifier).await;
                    }
                }
            }
            let _ = gio::spawn_blocking(move || {
                for entry in fs::read_dir(sources_dir).into_iter().flatten().flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "txt")
                        && path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .is_some_and(|id| !named.contains(id))
                    {
                        let _ = fs::remove_file(path);
                    }
                }
            })
            .await;

            if let Some(blocking) = weak.upgrade() {
                *blocking.activity.borrow_mut() = Activity::Idle;
                blocking.check(SystemTime::now());
            }
        });
    }

    pub(super) fn view(&self, document: Option<Document>) -> BlockingView {
        let settings = self.settings.borrow();
        let site = document.filter(|_| settings.on).and_then(|document| {
            let origin = WebOrigin::of(&document.page_url)?;
            let on = !settings.unblocked.contains(&origin);
            Some(SiteView {
                origin,
                document,
                on,
            })
        });
        BlockingView {
            on: settings.on,
            detail: detail(&self.activity.borrow(), self.compiled.borrow().is_empty()),
            site,
        }
    }

    pub(super) fn set_on(self: &Rc<Self>, on: bool) -> Result<()> {
        {
            let mut current = self.settings.borrow_mut();
            if current.on == on {
                return Ok(());
            }
            let next = Settings {
                on,
                ..current.clone()
            };
            next.save(&self.settings_file)?;
            *current = next;
        }
        self.refit();
        if on {
            self.start();
        } else {
            if let Some(daily) = self.daily.borrow_mut().take() {
                daily.remove();
            }
            let mut activity = self.activity.borrow_mut();
            if let Activity::Refreshing(task) = &*activity {
                task.abort();
                *activity = Activity::Idle;
            }
            drop(activity);
            self.release_startup();
        }
        self.emit(PageEvent::Changed);
        Ok(())
    }

    pub(super) fn set_site(&self, origin: WebOrigin, on: bool) -> Result<bool> {
        {
            let mut current = self.settings.borrow_mut();
            if !current.on {
                return Ok(false);
            }
            let mut next = current.clone();
            let changed = if on {
                next.unblocked.remove(&origin)
            } else {
                next.unblocked.insert(origin)
            };
            if !changed {
                return Ok(false);
            }
            next.save(&self.settings_file)?;
            *current = next;
        }
        self.refit();
        Ok(true)
    }

    pub(super) fn startup_pending(&self) -> bool {
        self.start_gate.borrow().is_some()
    }

    fn release_startup(&self) {
        let timer = self.start_gate.borrow_mut().take();
        if let Some(timer) = timer {
            timer.remove();
            self.emit(PageEvent::BlockingStoreReady);
        }
    }

    /// Fits blocking to `view`'s top site as it loads. The returned hook must hear every load
    /// stage of the view before anything else hears it, and keeps fitting while it does.
    pub(super) fn follow(
        self: &Rc<Self>,
        view: &Rc<dyn EngineView>,
    ) -> Option<impl Fn(LoadEvent) + 'static> {
        if self.sources.is_empty() {
            return None;
        }
        let mut page = Followed {
            rules: view.rules(),
            top: TopSite::default(),
            holds: Vec::new(),
        };
        self.fit(&mut page);
        let mut pages = self.pages.borrow_mut();
        pages.retain(|page| page.rules.alive());
        pages.push(page);
        drop(pages);
        let weak = Rc::downgrade(self);
        let view = Rc::downgrade(view);
        Some(move |event| {
            let (Some(blocking), Some(view)) = (weak.upgrade(), view.upgrade()) else {
                return;
            };
            let report = match event {
                LoadEvent::Started | LoadEvent::Redirected => {
                    // A warm restore can start while WebKit still reports an empty URI.
                    let Some(uri) = view.uri().filter(|uri| !uri.is_empty()) else {
                        return;
                    };
                    Top::Loading(Site::of(web_url(&uri).ok().as_ref()))
                }
                LoadEvent::Committed => Top::Committed(Site::of(
                    view.uri().and_then(|uri| web_url(&uri).ok()).as_ref(),
                )),
                LoadEvent::Finished => Top::Ended,
            };
            blocking.report_top(&*view, report);
        })
    }

    pub(super) fn loading(&self, view: &dyn EngineView, url: Option<&Url>) {
        if !self.sources.is_empty() {
            self.report_top(view, Top::Loading(Site::of(url)));
        }
    }

    pub(super) fn hold_navigation(&self, view: &dyn EngineView) {
        self.report_top(view, Top::Ended);
    }

    fn report_top(&self, view: &dyn EngineView, top: Top) {
        if let Some(page) = self
            .pages
            .borrow_mut()
            .iter_mut()
            .find(|page| page.rules.is_for(view))
        {
            page.top.report(top);
            self.fit(page);
        }
    }

    fn fit(&self, page: &mut Followed) {
        let compiled = self.compiled.borrow();
        let enabled = wanted(
            &self.settings.borrow(),
            !compiled.is_empty(),
            page.top.effective(),
        );
        refit_page(enabled.then_some(&*compiled), page);
    }

    fn refit(&self) {
        self.pages.borrow_mut().retain_mut(|page| {
            if !page.rules.alive() {
                return false;
            }
            self.fit(page);
            true
        });
    }

    pub(super) fn check(self: &Rc<Self>, now: SystemTime) {
        if !self.settings.borrow().on || !matches!(*self.activity.borrow(), Activity::Idle) {
            return;
        }
        let manifest = Manifest::load(&self.folder);
        let due: Vec<_> = {
            let compiled = self.compiled.borrow();
            self.sources
                .iter()
                .filter(|source| {
                    let current = manifest.0.get(&source.id).filter(|record| {
                        compiled
                            .get(&source.id)
                            .is_some_and(|installed| &installed.record == *record)
                    });
                    lists::due(current, now)
                })
                .cloned()
                .collect()
        };
        if due.is_empty() {
            return;
        }
        let weak = Rc::downgrade(self);
        let context = self.context.clone();
        let store = self
            .store
            .get()
            .expect("opened content filter store")
            .clone();
        let folder = self.folder.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let pauses = PauseMeter::start();
            let started = Instant::now();
            let mut results = Vec::with_capacity(due.len());
            for source in due {
                let result = refresh(&context, &folder, &store, &source, &pauses).await;
                results.push((source, result));
            }
            if let Some(blocking) = weak.upgrade() {
                blocking.publish(manifest, results, pauses, started);
            }
        });
        *self.activity.borrow_mut() = Activity::Refreshing(task);
        self.emit(PageEvent::Changed);
    }

    fn publish(
        &self,
        mut manifest: Manifest,
        results: Vec<(Source, Result<Refreshed>)>,
        pauses: PauseMeter,
        started: Instant,
    ) {
        *self.activity.borrow_mut() = Activity::Idle;
        let mut good = Vec::new();
        let mut failures = Vec::new();
        let mut retired = Vec::new();
        for (source, result) in results {
            match result {
                Ok(done) => {
                    if let Some(old) = manifest
                        .0
                        .insert(source.id.clone(), done.installed.record.clone())
                    {
                        retired.push(old.identifier);
                    }
                    good.push((source, done));
                }
                Err(error) => failures
                    .push(format!("{}: {error:#}", source.name()).replace(['\n', '\r'], " ")),
            }
        }
        if !good.is_empty() {
            if let Err(error) = manifest.save(&self.folder) {
                failures.push(format!("Could not save the list catalog: {error:#}"));
                retired = good
                    .iter()
                    .map(|(_, done)| done.installed.record.identifier.clone())
                    .collect();
            } else {
                for (source, mut done) in good {
                    done.source_file.disable_cleanup(true);
                    let report = &done.report;
                    eprintln!(
                        "Canopy: content blocking {}: {} rules, fetch {:.1} ms, convert {:.1} ms, save source {:.1} ms, compile {:.1} ms (WebKit call {:.1} ms), total {:.1} ms",
                        source.name(), report.rules, ms(report.fetch), ms(report.convert), ms(report.save_source),
                        ms(report.compile), ms(report.compile_call), ms(report.total),
                    );
                    self.compiled.borrow_mut().insert(source.id, done.installed);
                }
                self.refit();
            }
        }
        if !retired.is_empty() {
            let sources = self.folder.join("sources");
            let store = self
                .store
                .get()
                .expect("opened content filter store")
                .clone();
            glib::MainContext::default().spawn_local(async move {
                for identifier in retired {
                    let _ = fs::remove_file(sources.join(format!("{identifier}.txt")));
                    store.remove(identifier).await;
                }
            });
        }
        eprintln!(
            "Canopy: content blocking round {:.1} ms, longest other GTK pause {:.1} ms, {}",
            ms(started.elapsed()),
            ms(pauses.stop()),
            peak_rss()
        );
        self.emit(PageEvent::Changed);
        if !failures.is_empty() {
            self.emit(PageEvent::BlockingError(round_message(
                &failures,
                !self.compiled.borrow().is_empty(),
            )));
        }
    }

    fn emit(&self, event: PageEvent) {
        if let Some(context) = self.context.upgrade() {
            context.emit(event);
        }
    }
}

impl Drop for Blocking {
    fn drop(&mut self) {
        if let Activity::Refreshing(task) = self.activity.get_mut() {
            task.abort();
        }
        if let Some(daily) = self.daily.get_mut().take() {
            daily.remove();
        }
        if let Some(timer) = self.start_gate.get_mut().take() {
            timer.remove();
        }
    }
}

fn wanted(settings: &Settings, available: bool, site: &Site) -> bool {
    settings.on
        && available
        && !matches!(site, Site::Web(origin) if settings.unblocked.contains(origin))
}

fn refit_page(compiled: Option<&BTreeMap<String, Installed>>, page: &mut Followed) {
    let filters = compiled.into_iter().flat_map(BTreeMap::values);
    let wanted = filters.clone().map(|item| &item.record.identifier);
    if page.holds.iter().eq(wanted.clone()) {
        return;
    }
    for item in filters {
        if !page.holds.contains(&item.record.identifier) {
            page.rules.add(&item.rules);
        }
    }
    for identifier in &page.holds {
        if !wanted.clone().any(|next| next == identifier) {
            page.rules.remove(identifier);
        }
    }
    page.holds = wanted.cloned().collect();
}

fn detail(activity: &Activity, empty: bool) -> Option<&'static str> {
    (empty && matches!(activity, Activity::Refreshing(_))).then_some("Preparing lists…")
}

struct Refreshed {
    installed: Installed,
    source_file: tempfile::TempPath,
    report: Report,
}

struct Report {
    rules: usize,
    fetch: Duration,
    convert: Duration,
    save_source: Duration,
    compile: Duration,
    compile_call: Duration,
    total: Duration,
}

async fn refresh(
    context: &Weak<PageContext>,
    folder: &Path,
    store: &Rc<dyn RuleStore>,
    source: &Source,
    pauses: &PauseMeter,
) -> Result<Refreshed> {
    let started = Instant::now();
    let mut url = source.url.clone();
    let mut redirects = 0;
    let body = loop {
        let fetch = context
            .upgrade()
            .context("The window closed.")?
            .fetch_without_cookies(&url, MAX_LIST_BYTES);
        let answer = fetch.await.context("Could not fetch the list")?;
        match lists::next_hop(&url, answer.status, answer.location.as_deref(), redirects)
            .context("Could not fetch the list")?
        {
            Some(next) => {
                redirects += 1;
                url = next;
            }
            None => break answer.body,
        }
    };
    let fetch = started.elapsed();
    let identifier = source.next_identifier();
    let folder = folder.to_owned();
    let source_path = folder.join("sources").join(format!("{identifier}.txt"));
    let (converted, source_file, convert, save_source) = gio::spawn_blocking(move || {
        let write_started = Instant::now();
        fs::create_dir_all(folder.join("compiled"))?;
        fs::create_dir_all(folder.join("sources"))?;
        let source_file = tempfile::TempPath::try_from_path(source_path)?;
        fs::write(&source_file, &body)?;
        let save_source = write_started.elapsed();
        let convert_started = Instant::now();
        let converted = lists::convert(body)?;
        Ok::<_, anyhow::Error>((
            converted,
            source_file,
            convert_started.elapsed(),
            save_source,
        ))
    })
    .await
    .map_err(|_| anyhow::anyhow!("The converter stopped."))?
    .context("Could not prepare the list")?;
    let compile_started = Instant::now();
    let sampled = pauses.sample.clone();
    let rules = store
        .compile(
            identifier.clone(),
            glib::Bytes::from_owned(converted.json),
            Box::new(move |call| {
                sampled.compile_call.set(call);
                sampled.excluded.set(sampled.excluded.get() + call);
            }),
        )
        .await
        .context("Could not compile the list")?;
    Ok(Refreshed {
        installed: Installed {
            record: Record::new(identifier, SystemTime::now()),
            rules,
        },
        source_file,
        report: Report {
            rules: converted.rules,
            fetch,
            convert,
            save_source,
            compile: compile_started.elapsed(),
            compile_call: pauses.sample.compile_call.get(),
            total: started.elapsed(),
        },
    })
}

fn round_message(failures: &[String], has_filters: bool) -> String {
    let status = if has_filters {
        "Available lists stay in use."
    } else {
        "Pages load without blocking."
    };
    format!(
        "Could not update blocklists. {status} {}",
        failures.join(" · ")
    )
}

fn peak_rss() -> String {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")
                    .map(|value| format!("process peak RSS {}", value.trim()))
            })
        })
        .unwrap_or_else(|| "process peak RSS unavailable".into())
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

struct PauseMeter {
    timer: Option<glib::SourceId>,
    sample: Rc<PauseSample>,
}

struct PauseSample {
    last: Cell<Instant>,
    excluded: Cell<Duration>,
    longest: Cell<Duration>,
    compile_call: Cell<Duration>,
}

impl PauseSample {
    fn tick(&self) {
        let gap = self.last.replace(Instant::now()).elapsed();
        let other = gap.saturating_sub(self.excluded.take());
        self.longest.set(self.longest.get().max(other));
    }
}

impl PauseMeter {
    fn start() -> Self {
        let sample = Rc::new(PauseSample {
            last: Cell::new(Instant::now()),
            excluded: Cell::default(),
            longest: Cell::default(),
            compile_call: Cell::default(),
        });
        let ticks = sample.clone();
        let timer = glib::timeout_add_local(Duration::from_millis(5), move || {
            ticks.tick();
            glib::ControlFlow::Continue
        });
        Self {
            timer: Some(timer),
            sample,
        }
    }

    fn stop(self) -> Duration {
        self.sample.tick();
        self.sample.longest.get()
    }
}

impl Drop for PauseMeter {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_missing_or_unreadable_mean_on() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("content-blocking");
        assert!(Settings::load(&file).on);
        fs::create_dir(&file)?;
        assert!(Settings::load(&file).on);
        fs::remove_dir(&file)?;
        for text in ["false", "not JSON", r#"{"on":"invalid"}"#] {
            fs::write(&file, text)?;
            assert!(Settings::load(&file).on);
        }
        Ok(())
    }

    #[test]
    fn settings_round_trip_and_failed_save() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("content-blocking");
        let origin = WebOrigin::of(&web_url("https://example.org:8080")?).unwrap();
        let off = Settings {
            on: false,
            unblocked: BTreeSet::from([origin]),
        };
        off.save(&file)?;
        assert_eq!(Settings::load(&file), off);
        fs::create_dir(file.with_extension("tmp"))?;
        assert!(Settings {
            on: true,
            unblocked: BTreeSet::new()
        }
        .save(&file)
        .is_err());
        assert_eq!(Settings::load(&file), off);
        Ok(())
    }

    #[test]
    fn settings_drop_non_canonical_sites() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("content-blocking");
        fs::write(
            &file,
            r#"{"on":true,"unblocked":["https://example.org","https://example.org:443","https://example.org/path","file:///tmp",7]}"#,
        )?;
        let settings = Settings::load(&file);
        assert_eq!(settings.unblocked.len(), 1);
        assert_eq!(
            settings.unblocked.first().unwrap().as_str(),
            "https://example.org"
        );
        Ok(())
    }

    #[test]
    fn top_site_falls_back_to_the_committed_site() -> Result<()> {
        let a = web_url("http://a.test/page")?;
        let b = web_url("http://b.test/elsewhere")?;
        let mut top = TopSite::default();
        top.report(Top::Loading(Site::of(Some(&a))));
        top.report(Top::Committed(Site::of(Some(&a))));
        top.report(Top::Loading(Site::of(Some(&b))));
        assert_eq!(top.effective(), &Site::of(Some(&b)));
        top.report(Top::Ended);
        assert_eq!(top.effective(), &Site::of(Some(&a)));
        Ok(())
    }

    #[test]
    fn only_an_exact_top_origin_is_excepted() -> Result<()> {
        let excepted = web_url("https://example.org")?;
        let mut settings = Settings {
            on: true,
            unblocked: BTreeSet::from([WebOrigin::of(&excepted).unwrap()]),
        };
        assert!(!wanted(&settings, true, &Site::of(Some(&excepted))));
        for address in [
            "http://example.org",
            "https://www.example.org",
            "https://example.org:8080",
        ] {
            assert!(wanted(&settings, true, &Site::of(Some(&web_url(address)?))));
        }
        assert!(!wanted(&settings, false, &Site::Other));
        settings.on = false;
        assert!(!wanted(&settings, true, &Site::Other));
        Ok(())
    }

    #[test]
    fn one_line_per_round() {
        let errors = [
            "easylist: HTTP 500".to_owned(),
            "easyprivacy: HTTP 503".to_owned(),
        ];
        assert_eq!(round_message(&errors, true), "Could not update blocklists. Available lists stay in use. easylist: HTTP 500 · easyprivacy: HTTP 503");
    }
}
