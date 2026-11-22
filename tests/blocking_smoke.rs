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
use webkit2gtk::{WebView, WebViewExt};

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
                ("200 OK", "text/html", page_html(tag))
            }
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
        assert_eq!(
            view_js(
                view,
                "JSON.stringify(getComputedStyle(document.querySelector('.adbox')).display)"
            )?,
            display
        );
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
    ui.pages
        .check_blocklists(now + DAY * 7 - Duration::from_secs(2));
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
    close(ui);
    Ok(())
}
