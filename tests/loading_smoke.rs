mod common;
#[path = "common/http.rs"]
mod http;
use anyhow::{Context, Result};
use canopy::{
    browser::{NavigationTarget, VisitId},
    pages::Go,
    ui::Ui,
};
use common::*;
use gtk::prelude::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use url::Url;
use webkit2gtk::WebViewExt;

struct Site {
    origin: String,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
    released: Arc<Mutex<BTreeSet<String>>>,
    cancelled: Arc<Mutex<BTreeSet<String>>>,
}
impl Site {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let released = Arc::new(Mutex::new(BTreeSet::new()));
        let cancelled = Arc::new(Mutex::new(BTreeSet::new()));
        let closed = cancelled.clone();
        let seen = hits.clone();
        let gates = released.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let closed = closed.clone();
                let seen = seen.clone();
                let gates = gates.clone();
                thread::spawn(move || {
                    let Ok(request) = http::request(&mut stream) else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&request);
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    *seen.lock().unwrap().entry(path.clone()).or_default() += 1;
                    if path.starts_with("/hold") || path.starts_with("/resource") {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(20)))
                            .unwrap();
                        let end = Instant::now() + Duration::from_secs(30);
                        while !gates.lock().unwrap().contains(&path) && Instant::now() < end {
                            match stream.read(&mut [0; 1]) {
                                Ok(0) => {
                                    closed.lock().unwrap().insert(path.clone());
                                    return;
                                }
                                Err(e)
                                    if !matches!(
                                        e.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                    ) =>
                                {
                                    closed.lock().unwrap().insert(path.clone());
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }
                    if path == "/fail" {
                        return;
                    }
                    let body = format!("<!doctype html><title>{path}</title><h1>{path}</h1><input id='note'><a href='/hold-link'>Slow link</a>{}", if path == "/busy" { "<img src='/resource-busy'>" } else { "" });
                    let headers = if path == "/attachment" {
                        "Content-Disposition: attachment; filename=loading.txt\r\nContent-Type: text/plain"
                    } else {
                        "Content-Type: text/html"
                    };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\n{headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        Ok(Self {
            origin,
            hits,
            released,
            cancelled,
        })
    }
    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("{}{path}", self.origin)).unwrap()
    }
    fn hit(&self, path: &str) -> bool {
        self.hits.lock().unwrap().contains_key(path)
    }
    fn cancelled(&self, path: &str) -> bool {
        self.cancelled.lock().unwrap().contains(path)
    }
    fn count(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
    fn release(&self, path: &str) {
        self.released.lock().unwrap().insert(path.into());
    }
}

fn arrived(ui: &Ui, path: &str) -> Result<VisitId> {
    wait("committed fixture page", || {
        ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == path))
    })?;
    Ok(ui.snapshot()?.current.unwrap())
}
fn status(ui: &Ui, value: &str) -> bool {
    chrome(ui, "canopy.state().loading?.state").is_ok_and(|v| v == value)
}
fn idle(ui: &Ui) -> Result<()> {
    wait("idle native and chrome", || {
        !ui.pages.selected_page().native.is_loading()
            && status(ui, "idle")
            && !visible(ui, "#load-progress")
    })
}
fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let directory = tempfile::tempdir()?;
    std::env::set_var("HOME", directory.path());
    std::env::set_var("XDG_CONFIG_HOME", directory.path().join("config"));
    std::env::set_var("XDG_DATA_HOME", directory.path().join("data"));
    start(directory.path())?;
    let site = Site::new()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    idle(&ui)?;
    ui.pages.go(Go::Address {
        url: site.url("/source"),
        target: NavigationTarget::Root,
    })?;
    let source = arrived(&ui, "/source")?;
    idle(&ui)?;
    pump(1100);
    eval(
        &ui,
        "document.querySelector('#note').value = 'keep me'; JSON.stringify(true)",
    )?;
    let selected = ui.pages.selected_page().native;

    ui.pages.go(Go::Address {
        url: site.url("/hold-before-start"),
        target: NavigationTarget::Root,
    })?;
    let queued = ui.pages.foreground_loading_view().unwrap();
    assert!(!queued.is_loading());
    assert!(ui.pages.stop());
    assert!(!ui.pages.stop());
    idle(&ui)?;
    pump(300);
    assert!(!site.hit("/hold-before-start"));
    assert_eq!(ui.snapshot()?.current, Some(source));
    println!("PASS Stop before Started clears the queued attempt without needing Finished.");

    ui.pages.go(Go::Address {
        url: site.url("/hold-candidate"),
        target: NavigationTarget::Root,
    })?;
    wait("candidate request and progress", || {
        site.hit("/hold-candidate") && status(&ui, "busy") && visible(&ui, "#load-progress")
    })?;
    let candidate = ui.pages.foreground_loading_view().unwrap();
    assert!(candidate.is_loading());
    assert!(!selected.is_loading());
    assert_eq!(ui.pages.selected_page().native, selected);
    assert_eq!(
        chrome(&ui, "canopy.state().loading.page")?,
        serde_json::json!(source.to_string())
    );
    assert!(chrome(&ui, "canopy.state().loading.progress")?
        .as_u64()
        .is_some());
    key("ctrl+k")?;
    key("Escape")?;
    assert!(status(&ui, "busy"), "first Escape only closes the palette");
    key("Escape")?;
    idle(&ui)?;
    wait("candidate transport closed", || {
        site.cancelled("/hold-candidate")
    })?;
    assert_eq!(
        eval(&ui, "JSON.stringify(document.querySelector('#note').value)")?,
        "keep me"
    );
    site.release("/hold-candidate");
    pump(200);
    assert_eq!(ui.snapshot()?.current, Some(source));
    println!(
        "PASS Off-screen progress decorates the outgoing page; Escape priority preserves its form."
    );

    eval(
        &ui,
        "document.querySelector('a').click(); JSON.stringify(true)",
    )?;
    wait("native navigation started", || {
        selected.is_loading() && status(&ui, "busy")
    })?;
    key("Escape")?;
    idle(&ui)?;
    assert_eq!(ui.snapshot()?.current, Some(source));
    assert_eq!(
        eval(&ui, "JSON.stringify(document.querySelector('#note').value)")?,
        "keep me"
    );
    site.release("/hold-link");
    println!("PASS Native page navigation stops and leaves the committed document usable.");

    ui.pages.go(Go::Address {
        url: site.url("/hold-old"),
        target: NavigationTarget::Root,
    })?;
    wait("old candidate started", || {
        site.hit("/hold-old") && status(&ui, "busy")
    })?;
    ui.pages.go(Go::Address {
        url: site.url("/hold-new"),
        target: NavigationTarget::Root,
    })?;
    wait("replacement candidate started", || {
        site.hit("/hold-new") && status(&ui, "busy")
    })?;
    site.release("/hold-old");
    pump(200);
    assert!(status(&ui, "busy"));
    assert_eq!(ui.snapshot()?.current, Some(source));
    assert!(ui.pages.stop());
    idle(&ui)?;
    site.release("/hold-new");
    println!(
        "PASS Replacing and dropping candidates cannot clear the replacement's loading state."
    );

    ui.pages.go(Go::Address {
        url: site.url("/source#part"),
        target: NavigationTarget::ChildOf(source),
    })?;
    idle(&ui)?;
    println!("PASS Same-document navigation clears loading without a native Finished.");

    ui.pages.go(Go::Address {
        url: site.url("/busy"),
        target: NavigationTarget::ChildOf(source),
    })?;
    let busy_visit = arrived(&ui, "/busy")?;
    let shown = format!("canopy.state().loading.page === '{busy_visit}' && document.querySelector('#strip-nodes .pill')?.dataset.page === '{busy_visit}'");
    wait("committed page and loading resource are shown", || {
        site.hit("/resource-busy")
            && ui.pages.selected_page().native.is_loading()
            && status(&ui, "busy")
            && chrome(&ui, &shown).is_ok_and(|value| value == true)
    })
    .with_context(|| {
        format!(
            "native URI {:?}, loading {}, expected {busy_visit}, chrome {:?}",
            ui.pages.selected_page().native.uri(),
            ui.pages.selected_page().native.is_loading(),
            chrome(&ui, "({loading:canopy.state().loading,pills:[...document.querySelectorAll('#strip-nodes .pill')].map(e=>e.dataset.page),buttons:canopy.state().buttons})")
        )
    })?;
    open_list(&ui)?;
    assert_eq!(text(&ui, "#strip-list [data-key='action-reload']"), "Stop");
    chrome(&ui, "(() => { window.loadAction = document.querySelector('[data-key=action-reload]'); loadAction.focus(); return true; })()")?;
    hover_chrome(&ui, "#strip-list [data-key='action-reload']")?;
    xdo(&["mousedown", "1"])?;
    pump(80);
    site.release("/resource-busy");
    wait("native finishes under pressed Stop", || {
        !ui.pages.selected_page().native.is_loading() && status(&ui, "idle")
    })?;
    assert_eq!(text(&ui, "#strip-list [data-key='action-reload']"), "Stop");
    assert_eq!(
        chrome(
            &ui,
            "loadAction === document.querySelector('[data-key=action-reload]')"
        )?,
        true
    );
    xdo(&["mouseup", "1"])?;
    pump(400);
    assert!(!ui.pages.selected_page().native.is_loading());
    assert_eq!(site.count("/busy"), 1, "releasing Stop must not reload");
    open_list(&ui)?;
    assert_eq!(
        text(&ui, "#strip-list [data-key='action-reload']"),
        "Reload"
    );
    println!("PASS In-place commits retain progress; a pressed Stop keeps its DOM key and meaning through completion.");
    leave_strip(&ui)?;

    ui.pages.go(Go::Address {
        url: site.url("/hold-button"),
        target: NavigationTarget::Root,
    })?;
    wait("button candidate", || {
        site.hit("/hold-button") && status(&ui, "busy")
    })?;
    open_list(&ui)?;
    click_chrome(&ui, "#strip-list [data-act='stop']")?;
    idle(&ui)?;
    wait("Stop returns the keyboard to the page", || {
        ui.pages.selected_page().native.has_focus()
    })?;
    site.release("/hold-button");
    println!("PASS The strip Stop button cancels the foreground candidate and returns the keyboard to the page.");

    let queued_count = ui.snapshot()?.visits.len();
    eval(&ui, "document.body.insertAdjacentHTML('beforeend', '<a id=queue href=/hold-background style=position:fixed;top:100px;left:20px>Queue</a>'); JSON.stringify(true)")?;
    let (x, y) = page_point(&ui, 40, 94 + 110)?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    wait("waiting visit", || {
        ui.snapshot().is_ok_and(|s| s.visits.len() > queued_count)
    })?;
    let q = ui
        .snapshot()?
        .visits
        .iter()
        .find(|v| v.url.path() == "/hold-background")
        .unwrap()
        .id;
    wait("background request", || {
        site.hit("/hold-background") && ui.pages.background_view(q).is_some()
    })?;
    let background = ui.pages.background_view(q).unwrap();
    assert!(background.is_loading());
    assert!(status(&ui, "idle"));
    ui.pages.go(Go::Visit(q))?;
    wait("adopted candidate busy", || status(&ui, "busy"))?;
    assert!(ui.pages.stop());
    idle(&ui)?;
    wait("adopted network load cancelled", || {
        site.cancelled("/hold-background")
    })?;
    assert!(ui.pages.background_view(q).is_none());
    site.release("/hold-background");
    println!("PASS Background work stays quiet; Stop cancels an adopted preload rather than returning it to the pool.");

    ui.pages.go(Go::Address {
        url: site.url("/fail"),
        target: NavigationTarget::Root,
    })?;
    wait("failure requested", || site.hit("/fail"))?;
    idle(&ui)?;
    println!("PASS Network failure leaves the loading bar idle.");
    ui.pages.go(Go::Address {
        url: site.url("/attachment"),
        target: NavigationTarget::Root,
    })?;
    wait("download handed over", || {
        ui.downloads().is_ok_and(|d| !d.is_empty())
    })?;
    idle(&ui)?;
    println!("PASS Download handover leaves the loading bar idle.");
    ui.pages.go(Go::Visit(source))?;
    arrived(&ui, "/source")?;
    idle(&ui)?;
    assert_eq!(
        eval(&ui, "JSON.stringify(document.querySelector('#note').value)")?,
        "keep me"
    );
    ui.pages.go(Go::Address {
        url: site.url("/hold-settle"),
        target: NavigationTarget::ChildOf(source),
    })?;
    wait("foreground load before settling", || {
        site.hit("/hold-settle") && status(&ui, "busy")
    })?;
    let tree = ui
        .pages
        .read(|b| b.tree_states())?
        .into_iter()
        .find(|t| t.root.root() == source)
        .unwrap()
        .root;
    ui.pages.settle_tree(tree)?;
    wait("settle cancels foreground activity", || {
        status(&ui, "idle") && !visible(&ui, "#load-progress")
    })?;
    site.release("/hold-settle");
    println!("PASS Direct hidden return publishes idle; settling cancels foreground activity.");
    ui.window.close();
    Ok(())
}
