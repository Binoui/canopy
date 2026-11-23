mod common;
#[path = "common/http.rs"]
mod http;

use std::{
    cell::{Cell, RefCell},
    fs,
    io::Write,
    net::{TcpListener, TcpStream},
    path::Path,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use canopy::{
    address::web_url,
    browser::{NavigationTarget, VisitId},
    pages::Go,
    ui::Ui,
};
use common::*;
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use url::Url;
use webkit2gtk::{SettingsExt, WebView, WebViewExt};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, PartialEq, Eq)]
enum List {
    Held,
    Initial,
    Updated,
    Unavailable,
}

struct Request {
    path: String,
    credentials: bool,
}

struct Server {
    port: u16,
    lists: Mutex<[List; 2]>,
    release: Condvar,
    requests: Mutex<Vec<Request>>,
}

impl Server {
    fn start() -> Result<Arc<Self>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let server = Arc::new(Self {
            port: listener.local_addr()?.port(),
            lists: Mutex::new([List::Held, List::Initial]),
            release: Condvar::new(),
            requests: Mutex::new(Vec::new()),
        });
        let a = server.url("127.0.0.1", "list-a.txt");
        let b = server.url("127.0.0.1", "list-b.txt");
        std::env::set_var("CANOPY_BLOCKLISTS", format!("{a} {b}"));
        let serve = server.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let serve = serve.clone();
                thread::spawn(move || {
                    let _ = serve.reply(stream);
                });
            }
        });
        Ok(server)
    }

    fn set(&self, a: List, b: List) {
        *self.lists.lock().unwrap() = [a, b];
        self.release.notify_all();
    }

    fn reply(&self, mut stream: TcpStream) -> Result<()> {
        let bytes = http::request(&mut stream)?;
        let request = String::from_utf8_lossy(&bytes);
        let path = request.split_whitespace().nth(1).context("request path")?;
        self.requests.lock().unwrap().push(Request {
            path: path.to_owned(),
            credentials: request.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, _)| {
                    name.eq_ignore_ascii_case("cookie")
                        || name.eq_ignore_ascii_case("authorization")
                })
            }),
        });
        if path == "/list-a.txt" {
            write!(stream, "HTTP/1.1 302 Found\r\nLocation: /filter-a.txt\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
            return Ok(());
        }
        if let Some(tag) = path.strip_prefix("/jump/") {
            let host = if tag.ends_with("to-a") {
                "127.0.0.1"
            } else {
                "localhost"
            };
            let location = self.url(host, &format!("page/{tag}"));
            write!(stream, "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
            return Ok(());
        }
        let (status, kind, body) = match path {
            "/filter-a.txt" | "/list-b.txt" => {
                let index = usize::from(path == "/list-b.txt");
                let (lists, _) = self
                    .release
                    .wait_timeout_while(
                        self.lists.lock().unwrap(),
                        Duration::from_secs(20),
                        |lists| lists[index] == List::Held,
                    )
                    .unwrap();
                match (index, lists[index]) {
                    (_, List::Held | List::Unavailable) => {
                        ("500 Unavailable", "text/plain", String::new())
                    }
                    (0, List::Initial) => (
                        "200 OK",
                        "text/plain",
                        "[Adblock Plus 2.0]\n/ads/*$script\n##.adbox\n".into(),
                    ),
                    (0, List::Updated) => (
                        "200 OK",
                        "text/plain",
                        "[Adblock Plus 2.0]\n/ads2/*$script\n##.adbox\n".into(),
                    ),
                    _ => (
                        "200 OK",
                        "text/plain",
                        "[Adblock Plus 2.0]\n/track/*$script\n".into(),
                    ),
                }
            }
            path if path.starts_with("/page/") => {
                let tag = path.trim_start_matches("/page/");
                let mut body = page_html(tag);
                use std::fmt::Write;
                if tag == "site-root" {
                    let child = self.url("localhost", "page/site-child");
                    write!(body, r#"<a id="child" href="{child}">Other site</a>"#)?;
                }
                if tag == "form-root" {
                    body.push_str(r#"<form method="post" action="/page/form-result"><button id="form-submit">Submit</button></form>"#);
                }
                if tag.starts_with("embed-") {
                    let frame = self.url("127.0.0.1", "frame");
                    write!(body, r#"<iframe src="{frame}"></iframe>"#)?;
                }
                ("200 OK", "text/html", body)
            }
            "/frame" => ("200 OK", "text/html", page_html("nested")),
            path if path
                .split('?')
                .next()
                .is_some_and(|path| path.ends_with(".js")) =>
            {
                ("200 OK", "text/javascript", "// fixture".into())
            }
            _ => ("404 Not Found", "text/plain", String::new()),
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        Ok(())
    }

    fn url(&self, host: &str, path: &str) -> Url {
        web_url(&format!("http://{host}:{}/{path}", self.port)).unwrap()
    }

    fn hits(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path == path)
            .count()
    }

    fn assert_late_script(&self, view: &WebView, path: &str, allowed: bool) -> Result<()> {
        let before = self.hits(path);
        let path_js = serde_json::to_string(path)?;
        let script = format!(
            "window.late = 'pending'; (() => {{ const probe = document.createElement('script');              probe.src = {path_js}; probe.onload = () => window.late = 'load';              probe.onerror = () => window.late = 'error'; document.head.append(probe); }})(); JSON.stringify(true)"
        );
        view_js(view, &script)?;
        wait("late script outcome", || {
            view_js(view, "JSON.stringify(window.late)")
                .is_ok_and(|value| value == if allowed { "load" } else { "error" })
        })?;
        assert_eq!(self.hits(path), before + usize::from(allowed), "{path}");
        Ok(())
    }

    fn assert_load(&self, view: &WebView, tag: &str, ads: [bool; 3], display: &str) -> Result<()> {
        self.assert_requests(view, tag, ads)?;
        assert_display(view, display)
    }

    fn assert_requests(&self, view: &WebView, tag: &str, ads: [bool; 3]) -> Result<()> {
        let kinds = ["ads", "ads2", "track"];
        let mut stopped = serde_json::Map::new();
        stopped.insert("control".into(), "load".into());
        for (kind, reaches_server) in kinds.into_iter().zip(ads) {
            stopped.insert(
                kind.into(),
                if reaches_server {
                    "load".into()
                } else {
                    "error".into()
                },
            );
        }
        wait(&format!("all script outcomes on {tag}"), || {
            view_js(view, "JSON.stringify(window.outcomes || {})")
                .is_ok_and(|value| value == Value::Object(stopped.clone()))
        })?;
        pump(300);
        assert!(
            self.hits(&format!("/control/{tag}.js")) > 0,
            "{tag} control request reached fixture"
        );
        for (kind, reaches_server) in kinds.into_iter().zip(ads) {
            assert_eq!(
                self.hits(&format!("/{kind}/{tag}.js")) > 0,
                reaches_server,
                "{tag} {kind}"
            );
        }
        Ok(())
    }
}

fn page_html(tag: &str) -> String {
    let mut body = format!(
        r#"<!doctype html><title>{tag}</title>
        <h1>{tag}</h1><div class="adbox">Sponsored</div><p>Article text</p>
        <a id="background" href="/page/background">Background page</a>
        <script>window.outcomes = {{}}; window.nonce = Math.random(); document.cookie = "blocking_fixture=1; path=/";
        addEventListener('pageshow', event => {{window.wasCached = event.persisted;}});</script>"#
    );
    for kind in ["control", "ads", "ads2", "track"] {
        use std::fmt::Write;
        write!(body, r#"<script src="/{kind}/{tag}.js" onload="outcomes.{kind}='load'" onerror="outcomes.{kind}='error'"></script>"#).unwrap();
    }
    body
}

fn view_js(view: &WebView, script: &str) -> Result<Value> {
    let output = Rc::new(RefCell::new(None));
    let result = output.clone();
    view.evaluate_javascript(
        script,
        None,
        None,
        None::<&gio::Cancellable>,
        move |value| {
            *result.borrow_mut() = Some(value.map(|value| value.to_str().to_string()));
        },
    );
    wait("page JavaScript", || output.borrow().is_some())?;
    let string = output.borrow_mut().take().unwrap()?;
    Ok(serde_json::from_str(&string)?)
}

fn assert_display(view: &WebView, expected: &str) -> Result<()> {
    let display = view_js(
        view,
        "JSON.stringify(getComputedStyle(document.querySelector('.adbox')).display)",
    )?;
    assert_eq!(display, expected);
    Ok(())
}

fn navigate(ui: &Ui, url: Url, target: NavigationTarget) -> Result<VisitId> {
    ui.pages.go(Go::Address {
        url: url.clone(),
        target,
    })?;
    wait_for_page(ui, &url)
}

fn wait_for_page(ui: &Ui, url: &Url) -> Result<VisitId> {
    wait(&format!("page {url} finished"), || {
        ui.snapshot().is_ok_and(|snapshot| {
            snapshot
                .current_visit()
                .is_some_and(|visit| &visit.url == url)
        }) && !ui.pages.selected_page().native.is_loading()
    })?;
    ui.snapshot()?.current.context("selected visit")
}

fn files(path: &Path) -> Result<Vec<(String, SystemTime)>> {
    let mut files = fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            Ok((
                entry.file_name().into_string().unwrap(),
                entry.metadata()?.modified()?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    files.sort();
    Ok(files)
}

fn compiled_count(folder: &Path) -> usize {
    files(&folder.join("compiled")).map_or(0, |files| files.len())
}

fn launch(path: &Path) -> Result<Rc<Ui>> {
    let ui = Ui::new(path)?;
    ui.window.show_all();
    ready(&ui)?;
    Ok(ui)
}

fn close(ui: Rc<Ui>) {
    ui.window.close();
    drop(ui);
    pump(250);
}

fn switch_on(ui: &Ui, id: &str) -> Result<bool> {
    chrome(
        ui,
        &format!("document.getElementById('{id}')?.getAttribute('aria-checked')"),
    )?
    .as_str()
    .context("switch is visible")?
    .parse()
    .context("switch state")
}

fn recent(ui: &Ui) -> usize {
    chrome(ui, "S.errors.recent.length")
        .ok()
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize
}

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let data = tempfile::tempdir()?;
    let server = Server::start()?;
    start(data.path())?;
    let folder = data.path().join("blocklists");
    let ui = launch(data.path())?;
    wait("held list fetch", || server.hits("/filter-a.txt") == 1)?;
    open_settings(&ui)?;
    assert!(text(&ui, "#blocking").contains("Preparing lists"));
    assert!(switch_on(&ui, "blocking")?);
    key("Escape")?;
    navigate(
        &ui,
        server.url("127.0.0.1", "page/first"),
        NavigationTarget::Root,
    )?;
    let first = ui.pages.selected_page().native.clone();
    server.assert_load(&first, "first", [true; 3], "block")?;
    server.assert_late_script(&first, "/ads/live-first.js?cold", true)?;
    server.set(List::Initial, List::Initial);
    wait("first list catalog", || folder.join("lists.json").exists())?;
    assert_eq!(server.hits("/page/first"), 1);
    server.assert_late_script(&first, "/ads/live-first.js?ready", false)?;
    assert!(server
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.path == "/control/first.js" && request.credentials));
    assert_eq!(server.hits("/list-a.txt"), 1);
    assert_eq!(server.hits("/list-b.txt"), 1);
    assert!(server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.path.ends_with(".txt"))
        .all(|r| !r.credentials));
    let initial = files(&folder.join("compiled"))?;
    assert_eq!(initial.len(), 2);
    assert_eq!(files(&folder.join("sources"))?.len(), 2);
    let next = server.url("127.0.0.1", "page/next");
    navigate(&ui, next, NavigationTarget::Root)?;
    let shown = ui.pages.selected_page().native.clone();
    server.assert_load(&shown, "next", [false, true, false], "none")?;
    println!("PASS first-run browsing, local redirect and cookie-free list fetch, first publication without a reload, blocking and element hiding.");

    let (x, y) = element_point(&ui, "background")?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    let background = RefCell::new(None);
    wait("queued background visit", || {
        ui.snapshot()
            .unwrap()
            .visits
            .iter()
            .any(|visit| visit.url.path() == "/page/background")
    })?;
    wait("never-shown background page loaded", || {
        let snapshot = ui.snapshot().unwrap();
        let id = snapshot
            .visits
            .iter()
            .find(|visit| visit.url.path() == "/page/background")
            .unwrap()
            .id;
        if let Some(view) = ui.pages.background_view(id) {
            if !view.is_loading()
                && view
                    .uri()
                    .as_deref()
                    .is_some_and(|uri| uri.ends_with("/page/background"))
            {
                *background.borrow_mut() = Some(view);
                return true;
            }
        }
        false
    })?;
    server.assert_load(
        &background.into_inner().unwrap(),
        "background",
        [false, true, false],
        "none",
    )?;
    let done = Rc::new(Cell::new(false));
    let done_cb = done.clone();
    let _errand = ui.pages.run_errand(
        &server.url("127.0.0.1", "page/errand"),
        Duration::from_millis(300),
        move |_| done_cb.set(true),
    );
    wait("errand finished", || done.get())?;
    assert!(server.hits("/ads/errand.js") > 0);
    assert!(server.hits("/track/errand.js") > 0);
    assert_eq!(files(&folder.join("compiled"))?, initial);
    println!("PASS background filtering before first show, unfiltered errands, and reuse of two shared compiled lists.");

    let now = SystemTime::now();
    let nonce = view_js(&shown, "JSON.stringify(window.nonce)")?;
    open_settings(&ui)?;
    assert!(!text(&ui, "#blocking").contains("Preparing lists"));
    click_chrome(&ui, "#blocking")?;
    wait("main switch off", || {
        switch_on(&ui, "blocking").is_ok_and(|on| !on)
    })?;
    focus_switch(&ui, "blocking")?;
    server.assert_late_script(&shown, "/ads/live-switch.js?off", true)?;
    server.assert_late_script(&first, "/ads/retained-switch.js?off", true)?;
    ui.pages.check_blocklists(now + DAY * 8);
    pump(300);
    assert_eq!(
        server.hits("/list-a.txt"),
        1,
        "off suppresses the daily check"
    );
    key("space")?;
    wait("main switch on", || {
        switch_on(&ui, "blocking").is_ok_and(|on| on)
    })?;
    server.assert_late_script(&shown, "/ads/live-switch.js?on", false)?;
    server.assert_late_script(&first, "/ads/retained-switch.js?on", false)?;
    assert_eq!(view_js(&shown, "JSON.stringify(window.nonce)")?, nonce);
    assert_eq!(server.hits("/page/next"), 1);
    key("Escape")?;
    println!("PASS main switch changes requests on shown and retained pages, accepts pointer and keyboard, and pauses refresh without reloading.");

    ui.pages.check_blocklists(now + DAY * 6);
    pump(300);
    assert_eq!(server.hits("/list-a.txt"), 1);
    server.set(List::Updated, List::Initial);
    ui.pages.check_blocklists(now + DAY * 8);
    wait("refreshed list artifacts replace old ones", || {
        files(&folder.join("compiled")).is_ok_and(|files| files.len() == 2 && files != initial)
            && files(&folder.join("sources")).is_ok_and(|files| files.len() == 2)
    })?;
    navigate(
        &ui,
        server.url("localhost", "page/updated"),
        NavigationTarget::Root,
    )?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "updated",
        [true, false, false],
        "none",
    )?;
    println!("PASS weekly age gate and new rules apply without restarting.");

    let before = recent(&ui);
    server.set(List::Unavailable, List::Unavailable);
    ui.pages.check_blocklists(now + DAY * 9);
    wait("one Recent errors entry for both failed lists", || {
        recent(&ui) == before + 1
    })?;
    assert_eq!(server.hits("/list-a.txt"), 3);
    assert_eq!(server.hits("/list-b.txt"), 3);
    navigate(
        &ui,
        server.url("localhost", "page/failed"),
        NavigationTarget::Root,
    )?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "failed",
        [true, false, false],
        "none",
    )?;
    assert_eq!(recent(&ui), before + 1);
    println!("PASS failed round keeps previous filters and adds a single Recent errors entry.");

    drop((first, shown));
    close(ui);
    let last = server.url("localhost", "page/failed");
    let before = server.hits("/page/failed");
    let ui = Ui::new(data.path())?;
    ui.window.show_all();
    ui.resume(&[])?;
    wait("restored page's first main request", || {
        server.hits("/page/failed") == before + 1
    })?;
    wait_for_page(&ui, &last)?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "failed",
        [true, false, false],
        "none",
    )?;
    assert_eq!(
        server.hits("/list-a.txt"),
        3,
        "fresh compiled lists opened without another fetch"
    );
    println!("PASS a warm start filters the first restored page while its list endpoints are unavailable.");
    close(ui);

    fs::remove_dir_all(&folder)?;
    server.set(List::Updated, List::Initial);
    let ui = launch(data.path())?;
    wait("removed cache rebuilt", || {
        compiled_count(&folder) == 2 && folder.join("lists.json").is_file()
    })?;
    navigate(
        &ui,
        server.url("localhost", "page/rebuilt"),
        NavigationTarget::Root,
    )?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "rebuilt",
        [true, false, false],
        "none",
    )?;
    assert_eq!(server.hits("/list-a.txt"), 4);
    println!("PASS deleting the profile's blocklists folder rebuilds the two lists.");
    let manifest = fs::read(folder.join("lists.json"))?;
    let b_hits = server.hits("/list-b.txt");
    server.set(List::Initial, List::Held);
    ui.pages.check_blocklists(now + DAY * 8);
    wait("second fetch held after the first list compiled", || {
        server.hits("/list-b.txt") == b_hits + 1
    })?;
    open_settings(&ui)?;
    click_chrome(&ui, "#blocking")?;
    wait("main switch off during refresh", || {
        switch_on(&ui, "blocking").is_ok_and(|on| !on)
    })?;
    server.set(List::Initial, List::Initial);
    pump(400);
    assert_eq!(fs::read(folder.join("lists.json"))?, manifest);
    assert_eq!(recent(&ui), 0);
    close(ui);

    let last = server.url("localhost", "page/rebuilt");
    let ui = Ui::new(data.path())?;
    ui.window.show_all();
    ui.resume(&[])?;
    wait_for_page(&ui, &last)?;
    ready(&ui)?;
    open_settings(&ui)?;
    assert!(!switch_on(&ui, "blocking")?);
    server.assert_late_script(
        &ui.pages.selected_page().native,
        "/ads2/cancel.js?off",
        true,
    )?;
    click_chrome(&ui, "#blocking")?;
    wait("main switch on after restored off", || {
        switch_on(&ui, "blocking").is_ok_and(|on| on)
    })?;
    wait("cancelled compilation reclaimed", || {
        compiled_count(&folder) == 2
    })?;
    server.assert_late_script(
        &ui.pages.selected_page().native,
        "/ads2/cancel.js?on",
        false,
    )?;
    assert_eq!(fs::read(folder.join("lists.json"))?, manifest);
    assert_eq!(server.hits("/list-b.txt"), b_hits + 1);
    println!(
        "PASS a cancelled refresh publishes nothing and the saved Off switch survives restart."
    );
    let persisted = site_journey(&ui, &server)?;
    close(ui);
    let ui = Ui::new(data.path())?;
    ui.window.show_all();
    ui.resume(&[])?;
    wait_for_page(&ui, &persisted)?;
    ready(&ui)?;
    open_settings(&ui)?;
    assert!(switch_on(&ui, "blocking")?);
    assert!(!switch_on(&ui, "site-blocking")?);
    server.assert_load(
        &ui.pages.selected_page().native,
        "persist-site",
        [true; 3],
        "block",
    )?;
    assert_eq!(server.hits("/list-b.txt"), b_hits + 1);
    println!("PASS the site exception survives restart and its row is available only on a web page while blocking is on.");
    close(ui);
    startup_recovery(data.path(), &folder, &server)?;
    Ok(())
}

fn focus_switch(ui: &Ui, id: &str) -> Result<()> {
    let selector = format!("#{id}");
    key("Home")?;
    for _ in 0..24 {
        if chrome_focused(ui, &selector) {
            return Ok(());
        }
        key("Down")?;
    }
    anyhow::bail!("Could not focus {id} with the keyboard.")
}

fn site_journey(ui: &Ui, server: &Server) -> Result<Url> {
    key("Escape")?;
    let filtered = [true, false, false];
    navigate(
        ui,
        server.url("127.0.0.1", "page/site-hidden"),
        NavigationTarget::Root,
    )?;
    let hidden = ui.pages.selected_page().native.clone();
    server.assert_load(&hidden, "site-hidden", filtered, "none")?;
    let stale = chrome(ui, "S.frame.blocking.site")?;
    assert_eq!(stale["on"], true);
    let root = server.url("127.0.0.1", "page/site-root");
    navigate(ui, root.clone(), NavigationTarget::Root)?;
    let view = ui.pages.selected_page().native.clone();
    server.assert_load(&view, "site-root", filtered, "none")?;
    let stale = serde_json::to_string(&stale)?;
    chrome(
        ui,
        &format!("(() => {{ intent('block-site', {{ ...{stale}, on: false }}); return true; }})()"),
    )?;
    pump(150);
    open_settings(ui)?;
    assert!(
        switch_on(ui, "site-blocking")?,
        "a stale same-origin row was ignored"
    );
    assert_eq!(server.hits("/page/site-root"), 1);
    click_chrome(ui, "#site-blocking")?;
    wait("one site reload", || server.hits("/page/site-root") == 2)?;
    let root_visit = wait_for_page(ui, &root)?;
    server.assert_load(&view, "site-root", [true; 3], "block")?;
    assert_eq!(server.hits("/page/site-root"), 2);
    server.assert_late_script(&hidden, "/ads2/retained-site.js", true)?;
    assert_eq!(server.hits("/page/site-hidden"), 1);
    let cached = view_js(&view, "JSON.stringify(window.nonce)")?;
    key("Escape")?;

    let (x, y) = element_point(ui, "child")?;
    click_at(x, y)?;
    let child = server.url("localhost", "page/site-child");
    wait_for_page(ui, &child)?;
    assert_eq!(view, ui.pages.selected_page().native);
    server.assert_load(&view, "site-child", filtered, "none")?;
    key("alt+Left")?;
    wait_for_page(ui, &root)?;
    assert_eq!(ui.snapshot()?.current, Some(root_visit));
    wait("native Back restored the cached document", || {
        view_js(&view, "JSON.stringify(window.wasCached === true)").is_ok_and(|value| value == true)
    })?;
    assert_eq!(view_js(&view, "JSON.stringify(window.nonce)")?, cached);
    assert_eq!(server.hits("/page/site-root"), 2);
    assert_display(&view, "block")?;
    server.assert_late_script(&view, "/ads2/cached-site.js", true)?;
    key("alt+Right")?;
    let child_visit = wait_for_page(ui, &child)?;
    assert_display(&view, "none")?;
    server.assert_late_script(&view, "/ads2/forward-site.js", false)?;
    let return_url = server.url("127.0.0.1", "page/site-return");
    navigate(ui, return_url, NavigationTarget::ChildOf(child_visit))?;
    server.assert_load(&view, "site-return", [true; 3], "block")?;
    println!("PASS a site switch updates retained pages and reloads only the initiating document. Stale same-origin intents are ignored. Canopy's cross-origin loads and cached Back and Forward use the destination's policy.");

    navigate(
        ui,
        server.url("127.0.0.1", "page/embed-off"),
        NavigationTarget::Root,
    )?;
    wait("excepted top page allowed the iframe probe", || {
        server.hits("/ads2/nested.js") == 1
    })?;
    let parent = navigate(
        ui,
        server.url("localhost", "page/embed-on"),
        NavigationTarget::Root,
    )?;
    wait("other top page loaded the iframe control", || {
        server.hits("/control/nested.js") == 2
    })?;
    assert_eq!(server.hits("/ads2/nested.js"), 1);
    assert_eq!(server.hits("/track/nested.js"), 1);
    let redirect_to_a = server.url("127.0.0.1", "page/redirect-to-a");
    ui.pages.go(Go::Address {
        url: server.url("localhost", "jump/redirect-to-a"),
        target: NavigationTarget::ChildOf(parent),
    })?;
    let parent = wait_for_page(ui, &redirect_to_a)?;
    server.assert_requests(&ui.pages.selected_page().native, "redirect-to-a", [true; 3])?;
    let redirect_to_b = server.url("localhost", "page/redirect-to-b");
    ui.pages.go(Go::Address {
        url: server.url("127.0.0.1", "jump/redirect-to-b"),
        target: NavigationTarget::ChildOf(parent),
    })?;
    wait_for_page(ui, &redirect_to_b)?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "redirect-to-b",
        filtered,
        "none",
    )?;
    println!("PASS redirects use the final site's subresource policy in both directions. An iframe from the excepted site remains filtered under another top page.");

    form_return_policy(ui, server)?;

    let keyboard = server.url("127.0.0.1", "page/site-keyboard");
    navigate(ui, keyboard.clone(), NavigationTarget::Root)?;
    open_settings(ui)?;
    focus_switch(ui, "site-blocking")?;
    key("Return")?;
    wait("site re-enabled and reloaded", || {
        server.hits("/page/site-keyboard") == 2
    })?;
    wait_for_page(ui, &keyboard)?;
    assert_display(&ui.pages.selected_page().native, "none")?;
    server.assert_late_script(
        &ui.pages.selected_page().native,
        "/ads2/site-keyboard-late.js",
        false,
    )?;
    assert_eq!(server.hits("/page/site-keyboard"), 2);
    key("Escape")?;
    key("ctrl+t")?;
    wait("new tree surface", || {
        chrome(ui, "canopy.state().surface").is_ok_and(|value| value == "start")
    })?;
    open_settings(ui)?;
    assert!(!visible(ui, "#site-blocking"));
    key("Escape")?;

    let persisted = server.url("127.0.0.1", "page/persist-site");
    navigate(ui, persisted.clone(), NavigationTarget::Root)?;
    open_settings(ui)?;
    click_chrome(ui, "#blocking")?;
    wait("site row absent while blocking is off", || {
        !visible(ui, "#site-blocking")
    })?;
    focus_switch(ui, "blocking")?;
    key("space")?;
    wait("site row returns with blocking on", || {
        visible(ui, "#site-blocking")
    })?;
    click_chrome(ui, "#site-blocking")?;
    wait("persisted site excepted", || {
        switch_on(ui, "site-blocking").is_ok_and(|on| !on)
    })?;
    Ok(persisted)
}

fn startup_recovery(data: &Path, folder: &Path, server: &Server) -> Result<()> {
    let catalog = folder.join("lists.json");
    let mut aged: Value = serde_json::from_slice(&fs::read(&catalog)?)?;
    for record in aged.as_object_mut().unwrap().values_mut() {
        record["compiled_at"] =
            (record["compiled_at"].as_u64().unwrap() - (DAY * 8).as_secs()).into();
    }
    let aged = serde_json::to_vec(&aged)?;
    fs::write(&catalog, &aged)?;
    let before = server.hits("/list-a.txt");
    server.set(List::Held, List::Initial);
    let ui = Ui::new(data)?;
    ui.window.show_all();
    let command_line = server.url("localhost", "page/command-line");
    ui.resume(std::slice::from_ref(&command_line))?;
    let forwarded = server.url("localhost", "page/forwarded");
    ui.open(std::slice::from_ref(&forwarded))?;
    wait_for_page(&ui, &forwarded)?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "forwarded",
        [true, false, false],
        "none",
    )?;
    wait("stale lists fetched on start", || {
        server.hits("/list-a.txt") == before + 1
    })?;
    server.set(List::Updated, List::Initial);
    wait("stale catalog replaced", || {
        fs::read(&catalog).is_ok_and(|current| current != aged)
    })?;
    println!("PASS warm startup opens the first forwarded page with its old filters while a stale catalog refreshes.");
    close(ui);

    let previous = fs::read(&catalog)?;
    for entry in fs::read_dir(folder.join("compiled"))? {
        let path = entry?.path();
        let broken = folder.join("unreadable-filter");
        fs::write(&broken, b"invalid WebKit content filter")?;
        fs::rename(broken, path)?;
    }
    server.set(List::Held, List::Initial);
    let ui = Ui::new(data)?;
    ui.window.show_all();
    let unreadable = server.url("localhost", "page/unreadable");
    ui.resume(std::slice::from_ref(&unreadable))?;
    wait_for_page(&ui, &unreadable)?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "unreadable",
        [true; 3],
        "block",
    )?;
    server.set(List::Updated, List::Initial);
    wait("unreadable filters rebuilt", || {
        fs::read(&catalog).is_ok_and(|current| current != previous)
    })?;
    server.assert_late_script(
        &ui.pages.selected_page().native,
        "/ads2/after-recovery.js",
        false,
    )?;
    println!("PASS a start with unreadable compiled files browses while fresh filters are prepared and then installs them live.");
    close(ui);

    let another = tempfile::tempdir()?;
    let ui = launch(another.path())?;
    wait("second data folder has its own catalog", || {
        another.path().join("blocklists/lists.json").is_file()
    })?;
    navigate(
        &ui,
        server.url("127.0.0.1", "page/second-profile"),
        NavigationTarget::Root,
    )?;
    server.assert_load(
        &ui.pages.selected_page().native,
        "second-profile",
        [true, false, false],
        "none",
    )?;
    assert!(!another.path().join("content-blocking").exists());
    println!(
        "PASS a second data folder fetches independent filters and inherits no site exception."
    );
    close(ui);
    Ok(())
}

fn form_return_policy(ui: &Ui, server: &Server) -> Result<()> {
    navigate(
        ui,
        server.url("127.0.0.1", "page/form-root"),
        NavigationTarget::Root,
    )?;
    let view = ui.pages.selected_page().native.clone();
    WebViewExt::settings(&view)
        .context("page settings")?
        .set_enable_page_cache(false);
    let (x, y) = element_point(ui, "form-submit")?;
    click_at(x, y)?;
    let result = server.url("127.0.0.1", "page/form-result");
    let post = wait_for_page(ui, &result)?;
    let child = server.url("localhost", "page/form-child");
    navigate(ui, child.clone(), NavigationTarget::ChildOf(post))?;
    key("alt+Left")?;
    wait("Back asks before returning to the form result", || {
        visible(ui, "#form-return")
    })?;
    server.assert_late_script(&view, "/ads2/form-policy.js?held", false)?;
    key("Escape")?;
    wait_for_page(ui, &child)?;
    server.assert_late_script(&view, "/ads2/form-policy.js?cancelled", false)?;

    key("alt+Left")?;
    wait("second form return question", || {
        visible(ui, "#form-return")
    })?;
    click_chrome(ui, "#form-return-continue")?;
    wait_for_page(ui, &result)?;
    assert_eq!(server.hits("/page/form-result"), 2);
    server.assert_load(&view, "form-result", [true; 3], "block")?;
    println!("PASS a held or cancelled form return keeps the shown page's filters. Continue applies the form result's site exception before resending.");
    Ok(())
}
