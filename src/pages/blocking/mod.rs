mod lists;
mod webkit;

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};
use glib::prelude::ObjectExt;
use webkit2gtk::UserContentManager;

use super::{native::PageContext, PageEvent};
use lists::{Manifest, Record, Source};
use webkit::{Filter, FilterStore};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct Blocking {
    context: Weak<PageContext>,
    folder: PathBuf,
    sources: Vec<Source>,
    store: OnceCell<FilterStore>,
    activity: RefCell<Activity>,
    compiled: RefCell<BTreeMap<String, Installed>>,
    pages: RefCell<Vec<Followed>>,
    daily: RefCell<Option<glib::SourceId>>,
}

enum Activity {
    Dormant,
    Opening,
    Idle,
    Refreshing(glib::JoinHandle<()>),
}

struct Installed {
    record: Record,
    filter: Filter,
}

struct Followed {
    manager: glib::WeakRef<UserContentManager>,
    holds: Vec<String>,
}

impl Blocking {
    pub(super) fn new(data_dir: &Path, context: Weak<PageContext>) -> Self {
        Self {
            context,
            folder: data_dir.join("blocklists"),
            sources: lists::configured(),
            store: OnceCell::new(),
            activity: RefCell::new(Activity::Dormant),
            compiled: RefCell::default(),
            pages: RefCell::default(),
            daily: RefCell::default(),
        }
    }

    pub(super) fn start(self: &Rc<Self>) {
        if self.sources.is_empty() {
            return;
        }
        let store = self
            .store
            .get_or_init(|| FilterStore::new(&self.folder.join("compiled")))
            .clone();
        let manifest = Manifest::load(&self.folder);
        let records: Vec<_> = self
            .sources
            .iter()
            .filter_map(|source| Some((source.id.clone(), manifest.0.get(&source.id)?.clone())))
            .collect();
        *self.activity.borrow_mut() = Activity::Opening;
        let weak = Rc::downgrade(self);
        let sources_dir = self.folder.join("sources");
        glib::MainContext::default().spawn_local(async move {
            let started = Instant::now();
            let mut loaded = BTreeMap::new();
            for (id, record) in records {
                match store.load(record.identifier.clone()).await {
                    Ok(filter) => {
                        loaded.insert(id, Installed { record, filter });
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
        let weak = Rc::downgrade(self);
        *self.daily.borrow_mut() = Some(glib::timeout_add_local(DAY, move || {
            let Some(blocking) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            blocking.check(SystemTime::now());
            glib::ControlFlow::Continue
        }));
    }

    pub(super) fn follow(&self, manager: &UserContentManager) {
        if self.sources.is_empty() {
            return;
        }
        let mut page = Followed {
            manager: manager.downgrade(),
            holds: Vec::new(),
        };
        refit_page(&self.compiled.borrow(), &mut page, manager);
        let mut pages = self.pages.borrow_mut();
        pages.retain(|page| page.manager.upgrade().is_some());
        pages.push(page);
    }

    fn refit(&self) {
        let compiled = self.compiled.borrow();
        self.pages.borrow_mut().retain_mut(|page| {
            let Some(manager) = page.manager.upgrade() else {
                return false;
            };
            refit_page(&compiled, page, &manager);
            true
        });
    }

    pub(super) fn check(self: &Rc<Self>, now: SystemTime) {
        if !matches!(*self.activity.borrow(), Activity::Idle) {
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
    }
}

fn refit_page(
    compiled: &BTreeMap<String, Installed>,
    page: &mut Followed,
    manager: &UserContentManager,
) {
    let wanted = compiled.values().map(|item| &item.record.identifier);
    if page.holds.iter().eq(wanted.clone()) {
        return;
    }
    for item in compiled.values() {
        if !page.holds.contains(&item.record.identifier) {
            item.filter.attach(manager);
        }
    }
    for identifier in &page.holds {
        if !wanted.clone().any(|next| next == identifier) {
            webkit::detach(manager, identifier);
        }
    }
    page.holds = wanted.cloned().collect();
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
    store: &FilterStore,
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
    let filter = store
        .save(
            identifier.clone(),
            glib::Bytes::from_owned(converted.json),
            move |call| {
                sampled.compile_call.set(call);
                sampled.excluded.set(sampled.excluded.get() + call);
            },
        )
        .await
        .context("Could not compile the list")?;
    Ok(Refreshed {
        installed: Installed {
            record: Record::new(identifier, SystemTime::now()),
            filter,
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
    fn one_line_per_round() {
        let errors = [
            "easylist: HTTP 500".to_owned(),
            "easyprivacy: HTTP 503".to_owned(),
        ];
        assert_eq!(round_message(&errors, true), "Could not update blocklists. Available lists stay in use. easylist: HTTP 500 · easyprivacy: HTTP 503");
    }
}
