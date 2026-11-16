// Branch navigation regressions.
// Every selected case asserts behavior; diagnostic RESULT lines aid comparison.
#![allow(dead_code, unused_imports, unused_variables)]
#[path = "common/http.rs"]
mod http;
use anyhow::{bail, Context, Result};
use canopy::{
    browser::{Browser, NavigationTarget, TreeId, VisitId},
    forest::ForestProjection,
    pages::{Go, PageEvent, Pages},
};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use webkit2gtk::{BackForwardListExt, WebViewExt};

fn pump() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}
fn pump_for(ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        pump();
        thread::sleep(Duration::from_millis(3));
    }
}
fn wait(label: &str, condition: impl Fn() -> bool) -> Result<()> {
    if wait_for(15_000, condition) {
        Ok(())
    } else {
        bail!("Timed out waiting for {label}")
    }
}
fn wait_for(ms: u64, condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        pump();
        if condition() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Clone)]
struct Server {
    origin: String,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
    gates: Arc<Mutex<HashSet<String>>>,
}
fn page_body(path: &str) -> String {
    format!("<!doctype html><title>{path}</title><body style='margin:0;height:10000px'><input id='field'><a id='fork' href='/d'>fork</a><div id='fragment' style='margin-top:5000px'>end</div></body>")
}
fn respond(stream: &mut std::net::TcpStream, status: &str, extra: &str, body: &str) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nCache-Control: no-store\r\nContent-Type: text/html\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
}
impl Server {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let server = Self {
            origin,
            hits: Arc::new(Mutex::new(BTreeMap::new())),
            gates: Arc::new(Mutex::new(HashSet::new())),
        };
        let shared = server.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let s = shared.clone();
                thread::spawn(move || s.handle(&mut stream));
            }
        });
        Ok(server)
    }
    fn handle(&self, stream: &mut std::net::TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let Ok(request) = http::request(stream) else {
            return;
        };
        let text = String::from_utf8_lossy(&request).to_string();
        let method = text.split_whitespace().next().unwrap_or("GET").to_string();
        let full = text.split_whitespace().nth(1).unwrap_or("/").to_string();
        let path = full.split('?').next().unwrap_or("/").to_string();
        {
            let mut hits = self.hits.lock().unwrap();
            *hits.entry(format!("{method} {path}")).or_default() += 1;
            *hits.entry(path.clone()).or_default() += 1;
        }
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match parts.as_slice() {
            ["gate", name, ..] => {
                let deadline = Instant::now() + Duration::from_secs(25);
                while self.gates.lock().unwrap().contains(*name) && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                respond(stream, "200 OK", "", &page_body(&path));
            }
            ["delay", ms, ..] => {
                thread::sleep(Duration::from_millis(ms.parse().unwrap_or(0)));
                respond(stream, "200 OK", "", &page_body(&path));
            }
            ["iframe-pulse"] => respond(stream, "200 OK", "", "<!doctype html><title>pulse</title><body style='height:4000px'><iframe id=f src='/frame/0' width=100 height=50></iframe><script>let n=0;setInterval(()=>{document.getElementById('f').src='/frame/'+(++n)},150)</script></body>"),
            ["iframe-once"] => respond(stream, "200 OK", "", "<!doctype html><title>once</title><body style='height:4000px'><script>setTimeout(()=>{const f=document.createElement('iframe');f.src='/frame/once';document.body.appendChild(f)},700)</script></body>"),
            ["hash-pulse"] => respond(stream, "200 OK", "", "<!doctype html><title>hash</title><body style='height:4000px'><script>let n=0;setInterval(()=>{location.hash='h'+(++n)},150)</script></body>"),
            ["iframe-rotate", ms] => respond(stream, "200 OK", "", &format!("<!doctype html><title>rotate</title><body style='height:4000px'><iframe id=f src='/frame/r0' width=100 height=50></iframe><script>let n=0;setInterval(()=>{{document.getElementById('f').src='/frame/r'+(++n)}},{ms})</script></body>")),
            ["script-nav"] => respond(stream, "200 OK", "", "<!doctype html><title>scriptnav</title><body style='height:4000px'><script>setTimeout(()=>{location.href='/after-script'},700)</script></body>"),
            ["busy"] => respond(stream, "200 OK", "", "<!doctype html><title>busy</title><body style='height:4000px'><script>setTimeout(()=>{const t=Date.now();while(Date.now()-t<4000){}},600)</script></body>"),
            ["replaced"] => respond(stream, "200 OK", "", "<!doctype html><title>replaced</title><body style='height:4000px'><script>history.replaceState({}, '', '/replaced?utm=gone')</script></body>"),
            ["frame-click"] => respond(stream, "200 OK", "", "<!doctype html><title>frameclick</title><body style='height:4000px'><iframe id=f name=f src='/frame/c0' width=200 height=80></iframe><a id=l target=f href='/frame/c1'>in frame</a></body>"),
            ["popup-timer"] => respond(stream, "200 OK", "", "<!doctype html><title>popup</title><body style='height:4000px'><script>setTimeout(()=>{window.open('/popup-target')},700)</script></body>"),
            ["ps-pulse"] => respond(stream, "200 OK", "", "<!doctype html><title>pspulse</title><body style='height:4000px'><script>let n=0;setInterval(()=>{history.pushState({},'','/ps-pulse?'+(++n))},150)</script></body>"),
            ["ps-once"] => respond(stream, "200 OK", "", "<!doctype html><title>psonce</title><body style='height:4000px'><script>setTimeout(()=>{history.pushState({},'','/ps-once?route')},700)</script></body>"),
            ["ro-once"] => respond(stream, "200 OK", "", "<!doctype html><title>roonce</title><body style='height:4000px'><script>setTimeout(()=>{history.replaceState({},'','/ro-once?clean')},700)</script></body>"),
            ["rs-same"] => respond(stream, "200 OK", "", "<!doctype html><title>rssame</title><body style='height:4000px'><script>setInterval(()=>{history.replaceState({n:Date.now()},'',location.href)},150)</script></body>"),
            ["heavy", ms] => respond(stream, "200 OK", "", &format!("<!doctype html><title>heavy</title><body style='height:4000px'><a id='fork' href='/d' style='display:block;width:300px;height:80px'>fork</a><script>addEventListener('beforeunload',()=>{{let t=Date.now();while(Date.now()-t<{ms}){{}}}})</script></body>")),
            ["hang-img"] => respond(stream, "200 OK", "", "<!doctype html><title>hangimg</title><body style='height:4000px'><a id='fork' href='/d'>fork</a><img src='/gate/img/x.png'></body>"),
            ["spa"] => respond(stream, "200 OK", "", "<!doctype html><title>spa</title><body style='height:4000px'><script>history.pushState({},'','/spa?tab=1')</script></body>"),
            ["spa-late"] => respond(stream, "200 OK", "", "<!doctype html><title>spalate</title><body style='height:4000px'><script>setTimeout(()=>history.pushState({},'','/spa-late?tab=1'),300)</script></body>"),
            ["cr"] => respond(stream, "200 OK", "", "<!doctype html><title>cr</title><script>location.replace('/cr2')</script>"),
            ["mr"] => respond(stream, "200 OK", "", "<!doctype html><title>mr</title><meta http-equiv='refresh' content='1;url=/mr2'><p>mr</p>"),
            ["pform", n] => respond(stream, "200 OK", "", &format!("<!doctype html><title>pform</title><body style='height:4000px'><form id=f method=post action='/post/{n}'><input name=x value=1><button id=go style='width:200px;height:60px'>go</button></form></body>")),
            ["post", n] => {
                let next = n.parse::<u32>().unwrap_or(0) + 1;
                respond(stream, "200 OK", "", &format!("<!doctype html><title>post{n}</title><body style='height:4000px'><form id=f method=post action='/post/{next}'><input name=x value=1><button id=go style='width:200px;height:60px'>go</button></form></body>"))
            }
            ["prg-form"] => respond(stream, "200 OK", "", "<!doctype html><title>prgform</title><body style='height:4000px'><form id=f method=post action='/prg'><input name=x value=1><button id=go style='width:200px;height:60px'>go</button></form></body>"),
            ["prg"] => {
                let _ = write!(stream, "HTTP/1.1 303 See Other\r\nLocation: /prg-result\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
            ["alert-timer"] => respond(stream, "200 OK", "", "<!doctype html><title>alert</title><body style='height:4000px'><script>setTimeout(()=>{alert('probe')},700)</script></body>"),
            ["print-timer"] => respond(stream, "200 OK", "", "<!doctype html><title>print</title><body style='height:4000px'><script>setTimeout(()=>{print()},700)</script></body>"),
            ["close-timer"] => respond(stream, "200 OK", "", "<!doctype html><title>close</title><body style='height:4000px'><script>setTimeout(()=>{window.close()},700)</script></body>"),
            ["file"] => {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=direct.bin\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(&[9; 4096]);
            }
            ["frame", ..] => respond(stream, "200 OK", "", "<!doctype html><title>frame</title><p>frame</p>"),
            ["form"] => respond(stream, "200 OK", "", "<!doctype html><title>form</title><body style='height:4000px'><form id=f method=post action='/post-result'><input name=x value=1><button id=go>go</button></form></body>"),
            ["post-result"] => respond(stream, "200 OK", "", &page_body("/post-result")),
            ["redirect", rest @ ..] => {
                let to = format!("/{}", rest.join("/"));
                let _ = write!(stream, "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
            ["nc205"] => {
                let _ = write!(stream, "HTTP/1.1 205 Reset Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
            ["nocontent"] => {
                let _ = write!(stream, "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
            ["late-tall", ms] => {
                // Headers and a short start at once; the tall part of the page arrives late.
                let head = "<!doctype html><title>late</title><body style='margin:0'><p>start</p>";
                let tail = "<div style='height:10000px'>tall</div></body>";
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", head.len() + tail.len(), head);
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(ms.parse().unwrap_or(0)));
                let _ = stream.write_all(tail.as_bytes());
            }
            ["slowfile"] => {
                thread::sleep(Duration::from_millis(600));
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=probe.bin\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(&[7; 4096]);
            }
            _ => respond(stream, "200 OK", "", &page_body(&path)),
        }
    }
    fn url(&self, path: &str) -> url::Url {
        format!("{}{path}", self.origin).parse().unwrap()
    }
    fn hits(&self, key: &str) -> usize {
        *self.hits.lock().unwrap().get(key).unwrap_or(&0)
    }
    fn gate(&self, name: &str, held: bool) {
        let mut gates = self.gates.lock().unwrap();
        if held {
            gates.insert(name.into());
        } else {
            gates.remove(name);
        }
    }
}

struct Fixture {
    pages: Rc<Pages>,
    fixed: gtk::Fixed,
    window: gtk::Window,
    server: Server,
    dir: PathBuf,
    errors: Rc<RefCell<Vec<String>>>,
    _temp: Option<tempfile::TempDir>,
}
impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let dir = temp.path().to_path_buf();
        let mut f = Self::at(&dir, Server::new()?)?;
        f._temp = Some(temp);
        Ok(f)
    }
    fn at(dir: &Path, server: Server) -> Result<Self> {
        let fixed = gtk::Fixed::new();
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(1000, 800);
        window.add(&fixed);
        let pages = Pages::mount(
            &fixed,
            Browser::open(&dir.join("forest.sqlite3"))?,
            &dir.join("profile"),
        )?;
        window.show_all();
        pages.resize(1000., 800.);
        wait("initial page", || {
            !pages.selected_page().native.is_loading()
        })?;
        let errors = Rc::new(RefCell::new(Vec::new()));
        let seen = errors.clone();
        pages.observe(move |event| {
            if let PageEvent::Error(message) = event {
                seen.borrow_mut().push(message);
            }
        });
        Ok(Self {
            pages,
            fixed,
            window,
            server,
            dir: dir.to_path_buf(),
            errors,
            _temp: None,
        })
    }
    fn eval(&self, expression: &str) -> Result<Value> {
        let result = Rc::new(RefCell::new(None));
        let received = result.clone();
        self.pages.selected_page().native.evaluate_javascript(
            &format!("JSON.stringify((() => {{ {expression} }})())"),
            None,
            None,
            None::<&gio::Cancellable>,
            move |value| {
                *received.borrow_mut() = Some(value.map(|v| v.to_str().to_string()));
            },
        );
        wait("JavaScript", || result.borrow().is_some())?;
        let text = result.borrow_mut().take().unwrap()?;
        Ok(serde_json::from_str(&text)?)
    }
    fn cursor(&self) -> Option<VisitId> {
        self.pages.read(|b| b.snapshot()).ok()?.current
    }
    fn cursor_path(&self) -> String {
        self.pages
            .read(|b| b.snapshot())
            .ok()
            .and_then(|s| s.current_visit().map(|v| v.url.path().to_string()))
            .unwrap_or_else(|| "<none>".into())
    }
    fn arrived(&self, id: VisitId) -> Result<()> {
        wait("visit committed", || {
            self.cursor() == Some(id) && !self.pages.selected_page().native.is_loading()
        })?;
        self.pages.resize(1000., 800.);
        Ok(())
    }
    fn arrives(&self, id: VisitId, ms: u64) -> bool {
        let ok = wait_for(ms, || {
            self.cursor() == Some(id) && !self.pages.selected_page().native.is_loading()
        });
        self.pages.resize(1000., 800.);
        ok
    }
    fn navigate(&self, path: &str) -> Result<VisitId> {
        let parent = self.cursor();
        let target = parent
            .map(NavigationTarget::ChildOf)
            .unwrap_or(NavigationTarget::Root);
        let before = self.pages.read(|b| b.snapshot())?.visits.len();
        self.pages.go(Go::Address {
            url: self.server.url(path),
            target,
        })?;
        wait(&format!("new visit {path}"), || {
            self.pages
                .read(|b| b.snapshot())
                .is_ok_and(|s| s.visits.len() == before + 1)
                && !self.pages.selected_page().native.is_loading()
                && self.cursor_path() == path
        })?;
        Ok(self.cursor().unwrap())
    }
    fn open_root(&self, path: &str) -> Result<VisitId> {
        self.pages.open_roots(&[self.server.url(path)])?;
        wait(&format!("root {path}"), || {
            self.cursor_path() == path && !self.pages.selected_page().native.is_loading()
        })?;
        Ok(self.cursor().unwrap())
    }
    fn visit_with_path(&self, path: &str) -> Option<VisitId> {
        self.pages
            .read(|b| b.snapshot())
            .ok()?
            .visits
            .iter()
            .filter(|v| v.url.path() == path)
            .map(|v| v.id)
            .next_back()
    }
    fn visits_with_path(&self, path: &str) -> usize {
        self.pages
            .read(|b| b.snapshot())
            .map(|s| s.visits.iter().filter(|v| v.url.path() == path).count())
            .unwrap_or(0)
    }
    fn tree(&self, visit: VisitId) -> TreeId {
        self.pages
            .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))
            .unwrap()
            .tree_of(visit)
            .unwrap()
    }
    fn history(&self) -> Vec<VisitId> {
        self.pages.selected_history_visits()
    }
    fn list_len(&self) -> u32 {
        self.pages
            .selected_page()
            .native
            .back_forward_list()
            .map(|l| l.length())
            .unwrap_or(0)
    }
    fn events(&self) -> Vec<(String, String)> {
        let connection = rusqlite::Connection::open(self.dir.join("forest.sqlite3")).unwrap();
        let mut statement = connection
            .prepare("SELECT v.url, e.kind FROM visit_events e JOIN visits v ON v.id=e.visit_id ORDER BY e.id")
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap();
        rows.map(|r| {
            let (u, k) = r.unwrap();
            (url::Url::parse(&u).unwrap().path().to_string(), k)
        })
        .collect()
    }
    fn scroll_y(&self) -> f64 {
        self.eval("return scrollY;")
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(-1.)
    }
    fn errors(&self) -> usize {
        self.errors.borrow().len()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.gates.lock().unwrap().clear();
        self.window.close();
        pump_for(200);
    }
}

fn web_process_count() -> usize {
    fn walk(pid: u32, count: &mut usize) {
        let path = format!("/proc/{pid}/task/{pid}/children");
        if let Ok(children) = std::fs::read_to_string(path) {
            for child in children
                .split_whitespace()
                .filter_map(|p| p.parse::<u32>().ok())
            {
                if std::fs::read(format!("/proc/{child}/cmdline")).is_ok_and(|s| {
                    s.split(|b| *b == 0)
                        .next()
                        .is_some_and(|p| p.ends_with(b"WebKitWebProcess"))
                }) {
                    *count += 1;
                }
                walk(child, count);
            }
        }
    }
    let mut count = 0;
    walk(std::process::id(), &mut count);
    count
}

// Native navigation and response probes.

fn loading(f: &Fixture) -> bool {
    f.pages.selected_page().native.is_loading()
}
fn uri_path(f: &Fixture) -> String {
    f.pages
        .selected_page()
        .native
        .uri()
        .map(|u| u.to_string().replace(&f.server.origin, ""))
        .unwrap_or_else(|| "<none>".into())
}
fn refused_url() -> url::Url {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/refused", socket.local_addr().unwrap());
    drop(socket);
    url.parse().unwrap()
}
fn quiet(f: &Fixture, ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut since = Instant::now();
    loop {
        pump();
        if loading(f) {
            since = Instant::now();
        }
        if since.elapsed() > Duration::from_millis(400) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
fn native_list(f: &Fixture) -> Vec<String> {
    let Some(list) = f.pages.selected_page().native.back_forward_list() else {
        return vec![];
    };
    let name = |i: &webkit2gtk::BackForwardListItem| {
        webkit2gtk::BackForwardListItemExt::uri(i)
            .map(|u| u.to_string().replace(&f.server.origin, ""))
            .unwrap_or_default()
    };
    let mut items = list.back_list();
    items.reverse();
    let mut out: Vec<String> = items.iter().map(name).collect();
    if let Some(c) = list.current_item() {
        out.push(format!("*{}", name(&c)));
    }
    out.extend(list.forward_list().iter().map(name));
    out
}
fn label(f: &Fixture, id: Option<VisitId>) -> String {
    id.and_then(|id| f.pages.read(|b| b.visit(id)).ok())
        .map(|v| v.url.path().to_string())
        .unwrap_or_else(|| "<none>".into())
}
fn qlink(f: &Fixture, parent: VisitId, path: &str) -> Result<VisitId> {
    let source = canopy::browser::SourceContext {
        page_url: f.pages.read(|b| b.visit(parent))?.url,
        citation_url: None,
        link_text: "probe".into(),
        excerpt: String::new(),
    };
    let mut browser = Browser::open(&f.dir.join("forest.sqlite3"))?;
    let q = browser.queue_link(parent, f.server.url(path), source)?;
    drop(browser);
    Ok(q)
}
fn click_el(f: &Fixture, id: &str, button: &str) -> Result<()> {
    let point = f.eval(&format!("const r=document.getElementById('{id}').getBoundingClientRect(); return [r.x+r.width/2,r.y+r.height/2];"))?;
    let native = f.pages.selected_page().native.clone();
    let (x, y) = native
        .translate_coordinates(
            &f.window,
            point[0].as_f64().unwrap() as i32,
            point[1].as_f64().unwrap() as i32,
        )
        .unwrap();
    let (_, ox, oy) = f.window.window().unwrap().origin();
    let status = std::process::Command::new("xdotool")
        .args([
            "mousemove",
            &(ox + x).to_string(),
            &(oy + y).to_string(),
            "click",
            button,
        ])
        .status()?;
    anyhow::ensure!(status.success(), "xdotool failed");
    Ok(())
}
fn painted(f: &Fixture) -> Result<()> {
    f.eval("globalThis.__p=false; requestAnimationFrame(()=>requestAnimationFrame(()=>{globalThis.__p=true})); return true;")?;
    wait("painted", || {
        f.eval("return globalThis.__p;").ok() == Some(json!(true))
    })
}
fn open_trees(f: &Fixture) -> Vec<TreeId> {
    f.pages
        .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))
        .map(|forest| forest.open().iter().map(|t| t.id).collect())
        .unwrap_or_default()
}

/// The old page changes its URL by script (pushState or replaceState) while a switch loads.
fn bg_nav(page: &str, after: u64) -> Result<()> {
    let f = Fixture::new()?;
    f.navigate(page)?;
    pump_for(after);
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/target")])?;
    let requested = wait_for(8000, || f.server.hits("/gate/x/target") > 0);
    pump_for(1500);
    let uri_mid = uri_path(&f);
    f.server.gate("x", false);
    let arrived = wait_for(6000, || f.cursor_path() == "/gate/x/target" && !loading(&f));
    pump_for(1500);
    println!(
        "RESULT bg_nav page={page} after={after} requested={requested} arrived={arrived} cursor={} uri_mid={uri_mid} uri={} target_requests={} errors={} procs={}",
        f.cursor_path(),
        uri_path(&f),
        f.server.hits("/gate/x/target"),
        f.errors(),
        web_process_count()
    );
    assert!(
        requested && arrived,
        "same-document change cancelled requested switch"
    );
    Ok(())
}

/// The old view's in-place load is held in a busy beforeunload when a switch is requested; the
/// switch then does not land. Does the old view still take the next in-place request?
fn policy_window(spin: u32) -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.pages.go(Go::Address {
        url: f.server.url("/delay/400/x"),
        target: NavigationTarget::ChildOf(a),
    })?;
    for _ in 0..spin {
        gtk::main_iteration_do(false);
    }
    let loading_at_switch = loading(&f);
    f.pages.open_roots(&[refused_url()])?;
    pump_for(2000);
    let mid = f.cursor_path();
    let parent = f.cursor().unwrap();
    f.pages.go(Go::Address {
        url: f.server.url("/after"),
        target: NavigationTarget::ChildOf(parent),
    })?;
    let arrived = wait_for(5000, || f.cursor_path() == "/after" && !loading(&f));
    println!(
        "RESULT policy_window spin={spin} loading_at_switch={loading_at_switch} mid={mid} x_requests={} x_visits={} after_arrived={arrived} after_requests={} cursor={} errors={}",
        f.server.hits("/delay/400/x"),
        f.visits_with_path("/delay/400/x"),
        f.server.hits("/after"),
        f.cursor_path(),
        f.errors()
    );
    assert!(arrived, "policy-phase cancellation left old page deaf");
    Ok(())
}

/// The old page is committed but still loading a held image when a switch fails.
fn alias_first_page(path: &str) -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let q = qlink(&f, a, path)?;
    f.pages.go(Go::Visit(q))?;
    wait("queued page", || f.cursor() == Some(q) && !loading(&f))?;
    pump_for(800);
    let list = native_list(&f);
    let uri_before = uri_path(&f);
    f.pages.go(Go::Parent)?;
    let back1 = f.arrives(a, 4000);
    let after1 = format!("{}|{}", f.cursor_path(), uri_path(&f));
    let mut back2 = back1;
    if !back1 {
        pump_for(500);
        f.pages.go(Go::Parent)?;
        back2 = f.arrives(a, 4000);
    }
    println!(
        "RESULT alias_first_page path={path} list={list:?} uri_before={uri_before} back1_reaches_parent={back1} after1={after1} back2_reaches_parent={back2} cursor={} uri={} page_requests={} a_requests={}",
        f.cursor_path(),
        uri_path(&f),
        f.server.hits(path),
        f.server.hits("/a")
    );
    assert!(back1, "Back must reach forest parent in one press");
    Ok(())
}

fn download_candidate(path: &str) -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen = log.clone();
    let errors = f.errors.clone();
    f.pages.observe(move |event| match event {
        PageEvent::Download(canopy::pages::DownloadEvent::Started(n)) => {
            seen.borrow_mut().push(format!("started:{n}"))
        }
        PageEvent::Download(canopy::pages::DownloadEvent::Saved(p)) => seen.borrow_mut().push(
            format!("saved:{}", p.file_name().unwrap().to_string_lossy()),
        ),
        PageEvent::Download(canopy::pages::DownloadEvent::Failed { reason, .. }) => {
            seen.borrow_mut().push(format!("failed:{reason}"))
        }
        PageEvent::Error(m) => errors.borrow_mut().push(m),
        _ => {}
    });
    f.pages.open_roots(&[f.server.url(path)])?;
    let done = wait_for(10_000, || {
        log.borrow()
            .iter()
            .any(|e| e.starts_with("saved") || e.starts_with("failed"))
    });
    pump_for(2000);
    let recorded = f.pages.read(|b| b.downloads())?;
    let on: Vec<String> = recorded.iter().map(|d| label(&f, Some(d.visit))).collect();
    let root = f.visit_with_path(path);
    let root_opened = root
        .and_then(|r| f.pages.read(|b| b.visit(r)).ok())
        .map(|v| v.opened_at.is_some());
    println!(
        "RESULT download_candidate path={path} done={done} events={:?} recorded_on={on:?} root_exists={} root_opened={root_opened:?} cursor={} uri={} procs={} errors={}",
        log.borrow(),
        root.is_some(),
        f.cursor_path(),
        uri_path(&f),
        web_process_count(),
        f.errors()
    );
    let _ = a;
    assert!(
        done && on == ["/a"] && f.cursor() == Some(a),
        "candidate download must save under visible page"
    );
    Ok(())
}

fn wedge(ms: u64, cancel: &str) -> Result<()> {
    let f = Fixture::new()?;
    let z = f.navigate("/z")?;
    let a = f.navigate("/a")?;
    f.pages.go(Go::Address {
        url: f.server.url("/delay/400/x"),
        target: NavigationTarget::ChildOf(a),
    })?;
    if ms == 0 {
        gtk::main_iteration_do(false);
    } else {
        pump_for(ms);
    }
    let loading_at_switch = loading(&f);
    match cancel {
        "nocontent" => f.pages.open_roots(&[f.server.url("/nocontent")])?,
        "reselect" => {
            f.server.gate("x", true);
            f.pages.open_roots(&[f.server.url("/gate/x/t")])?;
            pump_for(50);
            f.pages.go(Go::Visit(a))?;
        }
        _ => f.pages.open_roots(&[refused_url()])?,
    }
    pump_for(1500);
    f.server.gate("x", false);
    let x = f.server.hits("/delay/400/x");
    f.pages.go(Go::Address {
        url: f.server.url("/after"),
        target: NavigationTarget::ChildOf(a),
    })?;
    let child = wait_for(3000, || f.cursor_path() == "/after" && !loading(&f));
    let mut out = format!("child={child}");
    if !child {
        f.pages.go(Go::Parent)?;
        let back = wait_for(3000, || f.cursor() == Some(z) && !loading(&f));
        out += &format!(" back={back}");
        let before = f.server.hits("/a");
        f.pages.reload()?;
        let reload = wait_for(3000, || f.server.hits("/a") > before);
        out += &format!(" reload={reload}");
        click_el(&f, "fork", "1")?;
        let link = wait_for(3000, || f.cursor_path() == "/d" && !loading(&f));
        out += &format!(" link={link}");
        let r = f.open_root("/elsewhere");
        out += &format!(" switch_away={}", r.is_ok());
        f.pages.go(Go::Visit(a))?;
        let back_to_a = f.arrives(a, 5000);
        out += &format!(" return_to_a={back_to_a}");
        if back_to_a {
            f.pages.go(Go::Address {
                url: f.server.url("/after2"),
                target: NavigationTarget::ChildOf(a),
            })?;
            let after2 = wait_for(4000, || f.cursor_path() == "/after2" && !loading(&f));
            out += &format!(" child_after_return={after2}");
        }
    }
    println!(
        "RESULT wedge ms={ms} cancel={cancel} loading_at_switch={loading_at_switch} x_requests={x} {out} cursor={} uri={} errors={}",
        f.cursor_path(),
        uri_path(&f),
        f.errors()
    );
    assert!(child, "failed switch left old page deaf");
    Ok(())
}

/// A queued page opened in its own branch; the user follows an in-page anchor, then presses Back.
fn alias_anchor() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let q = qlink(&f, a, "/q")?;
    f.pages.go(Go::Visit(q))?;
    wait("queued page", || f.cursor() == Some(q) && !loading(&f))?;
    pump_for(500);
    f.eval("const l=document.createElement('a'); l.id='anchor'; l.href='#fragment'; l.textContent='jump'; l.style.cssText='display:block;width:200px;height:60px'; document.body.prepend(l); return true;")?;
    painted(&f)?;
    click_el(&f, "anchor", "1")?;
    wait_for(2000, || uri_path(&f).ends_with("#fragment"));
    pump_for(500);
    let list = native_list(&f);
    f.pages.go(Go::Parent)?;
    let back1 = f.arrives(a, 3000);
    let after1 = format!("{}|{}", f.cursor_path(), uri_path(&f));
    println!(
        "RESULT alias_anchor list={list:?} back1_reaches_parent={back1} after1={after1} q_requests={}",
        f.server.hits("/q")
    );
    assert!(back1, "anchor Back must reach parent");
    Ok(())
}

/// POST chain revived after a switch; print WebKit's list after each Back.
fn post_walk(posts: u32) -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    f.navigate("/pform/1")?;
    let mut last = None;
    for n in 1..=posts {
        painted(&f)?;
        click_el(&f, "go", "1")?;
        let path = format!("/post/{n}");
        if !wait_for(8000, || f.cursor_path() == path && !loading(&f)) {
            println!(
                "RESULT post_walk posts={posts} ERROR submit {n} not recorded (cursor {})",
                f.cursor_path()
            );
            bail!("setup submit {n} was not recorded");
        }
        last = f.cursor();
    }
    let counts = |f: &Fixture| {
        (1..=posts)
            .map(|n| {
                format!(
                    "P{n}={},G{n}={}",
                    f.server.hits(&format!("POST /post/{n}")),
                    f.server.hits(&format!("GET /post/{n}"))
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut lines = vec![format!("start list={:?} [{}]", native_list(&f), counts(&f))];
    f.open_root("/root")?;
    pump_for(800);
    f.pages.go(Go::Visit(last.unwrap()))?;
    let arrived = f.arrives(last.unwrap(), 8000);
    pump_for(500);
    lines.push(format!(
        "revived arrived={arrived} cursor={} list={:?} [{}]",
        f.cursor_path(),
        native_list(&f),
        counts(&f)
    ));
    for i in 1..=posts + 3 {
        let before = f.cursor_path();
        f.pages.go(Go::Parent)?;
        let expected = if i < posts {
            format!("/post/{}", posts - i)
        } else if i == posts {
            "/pform/1".into()
        } else {
            "/a".into()
        };
        wait_for(8000, || f.cursor_path() == expected && !loading(&f));
        lines.push(format!(
            "back{i} {before}->{} uri={} list={:?} [{}]",
            f.cursor_path(),
            uri_path(&f),
            native_list(&f),
            counts(&f)
        ));
        assert_eq!(
            f.cursor_path(),
            expected,
            "each Back reaches the preceding page"
        );
        for n in 1..=posts {
            assert_eq!(
                f.server.hits(&format!("POST /post/{n}")),
                1,
                "no form resubmission"
            );
        }
        if f.cursor_path() == "/a" {
            break;
        }
    }
    println!(
        "RESULT post_walk posts={posts} reached_a={} errors={}\n  {}",
        f.cursor_path() == "/a",
        f.errors(),
        lines.join("\n  ")
    );
    assert_eq!(f.cursor_path(), "/a", "POST Back walk must reach start");
    Ok(())
}

/// A switch to an HTTPS page with a self-signed certificate.
fn post_mid_walk() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    f.navigate("/pform/1")?;
    painted(&f)?;
    click_el(&f, "go", "1")?;
    if !wait_for(8000, || f.cursor_path() == "/post/1" && !loading(&f)) {
        println!(
            "RESULT post_mid_walk ERROR submit not recorded (cursor {})",
            f.cursor_path()
        );
        bail!("setup POST visit not recorded");
    }
    let mid = f.navigate("/mid")?;
    let mut lines = vec![format!("start list={:?}", native_list(&f))];
    f.open_root("/root")?;
    pump_for(800);
    f.pages.go(Go::Visit(mid))?;
    let arrived = f.arrives(mid, 8000);
    pump_for(500);
    lines.push(format!(
        "revived arrived={arrived} list={:?}",
        native_list(&f)
    ));
    for (step, expected) in ["/post/1", "/pform/1", "/a"].into_iter().enumerate() {
        let i = step + 1;
        let before = f.cursor_path();
        f.pages.go(Go::Parent)?;
        wait_for(8000, || f.cursor_path() == expected && !loading(&f));
        assert_eq!(
            f.cursor_path(),
            expected,
            "each Back reaches its preceding page"
        );
        lines.push(format!(
            "back{i} {before}->{} list={:?} P1={} G1={}",
            f.cursor_path(),
            native_list(&f),
            f.server.hits("POST /post/1"),
            f.server.hits("GET /post/1")
        ));
        if f.cursor_path() == "/a" {
            break;
        }
    }
    println!(
        "RESULT post_mid_walk reached_a={}\n  {}",
        f.cursor_path() == "/a",
        lines.join("\n  ")
    );
    assert_eq!(
        f.cursor_path(),
        "/a",
        "Back through noncurrent POST must reach start"
    );
    Ok(())
}

/// Twenty switches among three trees, then count web processes after 5 s and 10 s at rest.
fn wedge_script() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.pages.go(Go::Address {
        url: f.server.url("/delay/400/x"),
        target: NavigationTarget::ChildOf(a),
    })?;
    gtk::main_iteration_do(false);
    gtk::main_iteration_do(false);
    gtk::main_iteration_do(false);
    let loading_at_switch = loading(&f);
    f.pages.open_roots(&[refused_url()])?;
    pump_for(1500);
    let before = f.pages.read(|b| b.snapshot())?.visits.len();
    f.eval("setTimeout(()=>{location.href='/scripted'},50); return true;")?;
    wait_for(4000, || uri_path(&f) == "/scripted" && !loading(&f));
    pump_for(800);
    let after_script = format!(
        "cursor={} uri={} visits_added={}",
        f.cursor_path(),
        uri_path(&f),
        f.pages.read(|b| b.snapshot())?.visits.len() - before
    );
    assert_eq!(
        f.cursor_path(),
        "/scripted",
        "self navigation must select its visit"
    );
    assert_eq!(
        f.pages.read(|b| b.snapshot())?.visits.len(),
        before + 1,
        "exactly one script visit"
    );
    f.pages.go(Go::Address {
        url: f.server.url("/after"),
        target: NavigationTarget::ChildOf(f.cursor().unwrap()),
    })?;
    let child = wait_for(4000, || f.cursor_path() == "/after" && !loading(&f));
    println!(
        "RESULT wedge_script loading_at_switch={loading_at_switch} {after_script} child_after={child} cursor={} uri={} list={:?}",
        f.cursor_path(),
        uri_path(&f),
        native_list(&f)
    );
    assert_eq!(
        f.cursor_path(),
        "/after",
        "page-started visit must be recorded"
    );
    Ok(())
}

/// A POST result in the middle of a revived back list (current page is a GET child of it).
fn unload_stuck(kind: &str) -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let heavy = f.navigate("/heavy/2000")?;
    f.pages.go(Go::Address {
        url: f.server.url("/third"),
        target: NavigationTarget::ChildOf(heavy),
    })?;
    pump_for(300);
    match kind {
        "land" => f.pages.open_roots(&[f.server.url("/landing")])?,
        "refused" => f.pages.open_roots(&[refused_url()])?,
        "nocontent" => f.pages.open_roots(&[f.server.url("/nocontent")])?,
        "reselect" => {
            f.server.gate("x", true);
            f.pages.open_roots(&[f.server.url("/gate/x/t")])?;
            pump_for(100);
            f.pages.go(Go::Visit(heavy))?;
        }
        "back" => {
            f.server.gate("x", true);
            f.pages.open_roots(&[f.server.url("/gate/x/t")])?;
            pump_for(100);
            f.pages.go(Go::Parent)?;
        }
        _ => {}
    }
    pump_for(3500);
    f.server.gate("x", false);
    pump_for(300);
    let mid = f.cursor_path();
    let mid_uri = uri_path(&f);
    let mid_loading = loading(&f);
    let third_visits = f.visits_with_path("/third");
    let parent = f.cursor().unwrap();
    f.pages.go(Go::Address {
        url: f.server.url("/after"),
        target: NavigationTarget::ChildOf(parent),
    })?;
    let arrived = wait_for(6000, || f.cursor_path() == "/after" && !loading(&f));
    let after_requests = f.server.hits("/after");
    let mut reload_note = String::new();
    if !arrived {
        let path = f.cursor_path();
        let before = f.server.hits(&path);
        f.pages.reload()?;
        let reloaded = wait_for(4000, || f.server.hits(&path) > before);
        reload_note = format!(" reload_requested={reloaded}");
    }
    println!(
        "RESULT unload_stuck kind={kind} mid_cursor={mid} mid_uri={mid_uri} mid_loading={mid_loading} third_requests={} third_visits={third_visits} after_arrived={arrived} after_requests={after_requests} cursor={} uri={} errors={}{reload_note}",
        f.server.hits("/third"),
        f.cursor_path(),
        uri_path(&f),
        f.errors()
    );
    assert!(
        arrived,
        "old page must recover after beforeunload cancellation"
    );
    assert_eq!(
        third_visits, 0,
        "cancelled beforeunload request must not commit"
    );
    if kind == "land" {
        assert_eq!(mid, "/landing", "beforeunload must not cancel switch");
    }
    let _ = a;
    Ok(())
}

/// The old view has just issued an in-place load (still in WebKit's policy phase) when a switch
/// that then fails is requested. Does the old view take the next in-place request?
fn no_content_ends_load() -> Result<()> {
    for path in ["/nocontent", "/nc205"] {
        let f = Fixture::new()?;
        let a = f.navigate("/a")?;
        let ended = Rc::new(std::cell::Cell::new(false));
        let seen = ended.clone();
        f.pages.observe(move |event| {
            if matches!(event, PageEvent::Loading(false)) {
                seen.set(true);
            }
        });
        f.pages.open_roots(&[f.server.url(path)])?;
        anyhow::ensure!(
            wait_for(8000, || f.server.hits(path) > 0),
            "no-content response requested"
        );
        anyhow::ensure!(
            wait_for(3000, || ended.get()),
            "no-content must end loading promptly"
        );
        assert_eq!(
            f.fixed.children().len(),
            1,
            "no-content candidate must be removed at load end"
        );
        assert_eq!(f.cursor(), Some(a));
        ended.set(false);
        f.pages.go(Go::Address {
            url: f.server.url(path),
            target: NavigationTarget::ChildOf(a),
        })?;
        anyhow::ensure!(
            wait_for(3000, || ended.get()),
            "in-place no-content must end loading promptly"
        );
        ended.set(false);
        f.navigate("/b")?;
        anyhow::ensure!(
            wait_for(1000, || ended.get()),
            "next load must deliver its own end"
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    // These checkpoint and teardown cases run under forced memory pressure.
    std::env::set_var("CANOPY_MEMORY_BUDGET_MB", "0");
    let environment = tempfile::tempdir()?;
    let home = environment.path().join("home");
    let config = environment.path().join("config");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&config)?;
    std::fs::write(
        config.join("user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Saved files\"\n",
    )?;
    std::env::set_var("HOME", home);
    std::env::set_var("XDG_CONFIG_HOME", config);
    std::env::set_var("XDG_DATA_HOME", environment.path().join("data"));
    std::env::set_var("XDG_CACHE_HOME", environment.path().join("cache"));
    canopy::init_desktop()?;
    let only: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    type Case = (&'static str, fn() -> Result<()>);
    let cases: &[Case] = &[
        ("no_content", no_content_ends_load),
        ("hash_pulse", || bg_nav("/hash-pulse", 900)),
        ("ps_pulse", || bg_nav("/ps-pulse", 900)),
        ("ps_once", || bg_nav("/ps-once", 150)),
        ("replace_once", || bg_nav("/ro-once", 150)),
        ("policy", || policy_window(1)),
        ("wedge_refused", || wedge(0, "refused")),
        ("wedge_204", || wedge(0, "nocontent")),
        ("wedge_reselect", || wedge(0, "reselect")),
        ("wedge_script", wedge_script),
        ("post_two", || post_walk(2)),
        ("post_mid", post_mid_walk),
        ("file", || download_candidate("/file")),
        ("slowfile", || download_candidate("/slowfile")),
        ("spa", || alias_first_page("/spa")),
        ("spa_late", || alias_first_page("/spa-late")),
        ("anchor", alias_anchor),
        ("unload_lands", || unload_stuck("land")),
        ("unload_refused", || unload_stuck("refused")),
        ("unload_204", || unload_stuck("nocontent")),
    ];
    for (name, run) in cases {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        run().with_context(|| format!("branch behavior {name}"))?;
        println!("DONE {name}");
        pump_for(300);
    }
    Ok(())
}
