// Review probes for stage 2 step 5. Compiles against both 67f4c16 (main) and 24f2ed7.
// Every case prints RESULT lines; no case aborts on an unexpected observation.
#![allow(dead_code, unused_imports, unused_variables)]
use anyhow::{bail, Result};
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
    io::{Read, Write},
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
        let mut request = [0; 16384];
        let Ok(n) = stream.read(&mut request) else {
            return;
        };
        let text = String::from_utf8_lossy(&request[..n]).to_string();
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
            ["frame", ..] => respond(stream, "200 OK", "", "<!doctype html><title>frame</title><p>frame</p>"),
            ["form"] => respond(stream, "200 OK", "", "<!doctype html><title>form</title><body style='height:4000px'><form id=f method=post action='/post-result'><input name=x value=1><button id=go>go</button></form></body>"),
            ["post-result"] => respond(stream, "200 OK", "", &page_body("/post-result")),
            ["redirect", rest @ ..] => {
                let to = format!("/{}", rest.join("/"));
                let _ = write!(stream, "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
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

// --- Probes -----------------------------------------------------------------

/// The old page keeps running while a switch loads; a frame or script navigation in it
/// must not cancel the switch the user asked for.
fn background_navigation(page: &str) -> Result<()> {
    background_navigation_after(page, 900)
}
fn background_navigation_after(page: &str, after: u64) -> Result<()> {
    let f = Fixture::new()?;
    f.navigate(page)?;
    pump_for(after);
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/target")])?;
    wait("target requested", || f.server.hits("/gate/x/target") > 0)?;
    pump_for(1500);
    f.server.gate("x", false);
    let arrived = wait_for(6000, || {
        f.cursor_path() == "/gate/x/target" && !f.pages.selected_page().native.is_loading()
    });
    println!(
        "RESULT background_navigation page={page} after={after} arrived={arrived} cursor={} target_requests={} frame_requests={} errors={}",
        f.cursor_path(),
        f.server.hits("/gate/x/target"),
        f.server.hits("/frame/once") + (0..40).map(|i| f.server.hits(&format!("/frame/{i}"))).sum::<usize>(),
        f.errors()
    );
    Ok(())
}

/// Repeated quick switches away from a page whose ad-like frame rotates every `ms`.
fn rotate_success(ms: u64) -> Result<()> {
    let f = Fixture::new()?;
    let page = f.navigate(&format!("/iframe-rotate/{ms}"))?;
    pump_for(500);
    let mut ok = 0;
    let tries = 10;
    for i in 0..tries {
        let path = format!("/delay/300/t{i}");
        f.pages.open_roots(&[f.server.url(&path)])?;
        let arrived = wait_for(4000, || {
            f.cursor_path() == path && !f.pages.selected_page().native.is_loading()
        });
        if arrived {
            ok += 1;
            pump_for(700);
            f.pages.go(Go::Visit(page))?;
            if !f.arrives(page, 6000) {
                println!("NOTE rotate could not return to frame page");
            }
            pump_for(300 + (i as u64 * 97) % 700);
        } else {
            pump_for(200);
        }
    }
    println!(
        "RESULT rotate_success frame_every_ms={ms} switches_arrived={ok}/{tries} errors={}",
        f.errors()
    );
    Ok(())
}

/// An in-place load in the selected view that is still in flight when a switch starts.
fn old_inflight() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.pages.go(Go::Address {
        url: f.server.url("/delay/700/abandoned"),
        target: NavigationTarget::ChildOf(a),
    })?;
    pump_for(80);
    let before = f.events().len();
    f.pages.open_roots(&[f.server.url("/delay/1800/target")])?;
    let arrived = wait_for(6000, || {
        f.cursor_path() == "/delay/1800/target" && !f.pages.selected_page().native.is_loading()
    });
    pump_for(800);
    let events: Vec<_> = f.events().into_iter().skip(before).collect();
    println!(
        "RESULT old_inflight arrived={arrived} cursor={} abandoned_visits={} abandoned_requests={} events_after_switch={:?}",
        f.cursor_path(),
        f.visits_with_path("/delay/700/abandoned"),
        f.server.hits("/delay/700/abandoned"),
        events
    );
    Ok(())
}

/// A second in-place request superseding an in-flight one, immediately followed by a switch.
fn retire_race() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.server.gate("s1", true);
    f.pages.go(Go::Address {
        url: f.server.url("/gate/s1/one"),
        target: NavigationTarget::ChildOf(a),
    })?;
    wait("first requested", || f.server.hits("/gate/s1/one") > 0)?;
    pump_for(100);
    f.pages.go(Go::Address {
        url: f.server.url("/two"),
        target: NavigationTarget::ChildOf(a),
    })?;
    f.pages.open_roots(&[f.server.url("/target")])?;
    pump_for(300);
    f.server.gate("s1", false);
    let arrived = wait_for(5000, || {
        f.cursor_path() == "/target" && !f.pages.selected_page().native.is_loading()
    });
    pump_for(800);
    println!(
        "RESULT retire_race arrived={arrived} cursor={} two_requests={} two_visits={} target_requests={} one_visits={}",
        f.cursor_path(),
        f.server.hits("/two"),
        f.visits_with_path("/two"),
        f.server.hits("/target"),
        f.visits_with_path("/gate/s1/one")
    );
    Ok(())
}

/// Settling an unrelated tree while a switch to another tree loads.
fn settle_other_during_switch() -> Result<()> {
    let f = Fixture::new()?;
    let y = f.navigate("/y")?;
    let tree_y = f.tree(y);
    f.open_root("/w")?;
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/x")])?;
    wait("x requested", || f.server.hits("/gate/x/x") > 0)?;
    f.pages.settle_tree(tree_y)?;
    pump_for(300);
    f.server.gate("x", false);
    let arrived = wait_for(5000, || {
        f.cursor_path() == "/gate/x/x" && !f.pages.selected_page().native.is_loading()
    });
    println!(
        "RESULT settle_other_during_switch arrived={arrived} cursor={} x_requests={} errors={}",
        f.cursor_path(),
        f.server.hits("/gate/x/x"),
        f.errors()
    );
    Ok(())
}

/// Settling the current tree while a switch to another tree loads.
fn settle_current_during_switch() -> Result<()> {
    let f = Fixture::new()?;
    let y = f.navigate("/y")?;
    let w = f.open_root("/w")?;
    let tree_w = f.tree(w);
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/x")])?;
    wait("x requested", || f.server.hits("/gate/x/x") > 0)?;
    f.pages.settle_tree(tree_w)?;
    pump_for(300);
    f.server.gate("x", false);
    wait_for(5000, || !f.pages.selected_page().native.is_loading());
    pump_for(1500);
    println!(
        "RESULT settle_current_during_switch cursor={} x_requests={} x_opened={} errors={}",
        f.cursor_path(),
        f.server.hits("/gate/x/x"),
        f.visit_with_path("/gate/x/x").is_some(),
        f.errors()
    );
    Ok(())
}

fn rapid_roots() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    pump_for(500);
    let start = web_process_count();
    for i in 0..6 {
        f.pages
            .open_roots(&[f.server.url(&format!("/delay/150/r{i}"))])?;
        pump_for(40);
    }
    let arrived = wait_for(8000, || {
        f.cursor_path() == "/delay/150/r5" && !f.pages.selected_page().native.is_loading()
    });
    pump_for(2500);
    let hits: Vec<usize> = (0..6)
        .map(|i| f.server.hits(&format!("/delay/150/r{i}")))
        .collect();
    println!(
        "RESULT rapid_roots arrived={arrived} cursor={} procs_before={start} procs_after={} requests={hits:?} events={:?} errors={}",
        f.cursor_path(),
        web_process_count(),
        f.events().iter().map(|e| e.0.clone()).collect::<Vec<_>>(),
        f.errors()
    );
    Ok(())
}

fn rapid_trees() -> Result<()> {
    let f = Fixture::new()?;
    let t1a = f.navigate("/t1a")?;
    let t1b = f.navigate("/t1b")?;
    let t2a = f.open_root("/t2a")?;
    let t2b = f.navigate("/t2b")?;
    let t3a = f.open_root("/t3a")?;
    let t3b = f.navigate("/t3b")?;
    let (k1, k2, k3) = (f.tree(t1a), f.tree(t2a), f.tree(t3a));
    pump_for(800);
    for tree in [k1, k2, k1, k3, k2, k1] {
        f.pages.go(Go::Tree(tree))?;
        pump_for(25);
    }
    let arrived = f.arrives(t1b, 8000);
    pump_for(1000);
    let h1 = f.history().contains(&t1a);
    f.pages.go(Go::Tree(k2))?;
    let a2 = f.arrives(t2b, 8000);
    pump_for(700);
    let h2 = f.history().contains(&t2a);
    f.pages.go(Go::Tree(k3))?;
    let a3 = f.arrives(t3b, 8000);
    pump_for(700);
    let h3 = f.history().contains(&t3a);
    pump_for(1500);
    println!(
        "RESULT rapid_trees arrived={arrived} t1_back={h1} t2_arrived={a2} t2_back={h2} t3_arrived={a3} t3_back={h3} procs={} errors={} t1b_requests={} t2b_requests={}",
        web_process_count(),
        f.errors(),
        f.server.hits("/t1b"),
        f.server.hits("/t2b"),
    );
    Ok(())
}

fn post_switch_back() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let form = f.navigate("/form")?;
    let before = f.pages.read(|b| b.snapshot())?.visits.len();
    f.eval("document.getElementById('go').click(); return true;")?;
    let posted = wait_for(8000, || {
        f.cursor_path() == "/post-result" && !f.pages.selected_page().native.is_loading()
    });
    let post = f.visit_with_path("/post-result");
    let next = f.navigate("/after-post")?;
    f.pages.go(Go::Visit(post.unwrap()))?;
    let back = f.arrives(post.unwrap(), 8000);
    pump_for(800);
    f.open_root("/root")?;
    pump_for(800);
    let posts_before = f.server.hits("POST /post-result");
    f.pages.go(Go::Visit(post.unwrap()))?;
    let arrived = f.arrives(post.unwrap(), 8000);
    pump_for(800);
    println!(
        "RESULT post_switch_back posted={posted} arrived={arrived} history_has_form={} history_has_a={} history_has_after={} list_len={} post_requests_total={} posts_on_switch={} get_post_result={}",
        f.history().contains(&form),
        f.history().contains(&a),
        f.history().contains(&next),
        f.list_len(),
        f.server.hits("POST /post-result"),
        f.server.hits("POST /post-result") - posts_before,
        f.server.hits("GET /post-result"),
    );
    Ok(())
}

/// After settling the last tree the selected view is blanked and reused by the next tree.
fn settle_start_new_tree() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.navigate("/b")?;
    let tree = f.tree(a);
    f.pages.settle_tree(tree)?;
    pump_for(800);
    println!("NOTE settle_start cursor_after_settle={}", f.cursor_path());
    let n1 = f.navigate("/n1")?;
    let n2 = f.navigate("/n2")?;
    let len_before = f.list_len();
    f.open_root("/other")?;
    pump_for(800);
    f.pages.go(Go::Visit(n2))?;
    let arrived = f.arrives(n2, 8000);
    pump_for(500);
    println!(
        "RESULT settle_start_new_tree arrived={arrived} list_len_before_switch={len_before} history_has_n1={} list_len_after={} n2_requests={} n1_requests={}",
        f.history().contains(&n1),
        f.list_len(),
        f.server.hits("/n2"),
        f.server.hits("/n1"),
    );
    f.pages.go(Go::Parent)?;
    let back = f.arrives(n1, 6000);
    println!(
        "RESULT settle_start_back back_to_n1={back} cursor={} n1_requests={}",
        f.cursor_path(),
        f.server.hits("/n1")
    );
    Ok(())
}

fn nocontent_switch() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    pump_for(800);
    let start = web_process_count();
    f.pages.open_roots(&[f.server.url("/nocontent")])?;
    pump_for(3000);
    let during = web_process_count();
    println!(
        "RESULT nocontent_switch cursor={} errors={} procs_before={start} procs_after_3s={during} requests={} loading={}",
        f.cursor_path(),
        f.errors(),
        f.server.hits("/nocontent"),
        f.pages.selected_page().native.is_loading()
    );
    f.open_root("/after")?;
    pump_for(2500);
    println!("RESULT nocontent_after procs={}", web_process_count());
    Ok(())
}

fn restart_only_cursor() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let server = Server::new()?;
    let (b, d) = {
        let f = Fixture::at(temp.path(), server.clone())?;
        f.navigate("/ra")?;
        let b = f.navigate("/rb")?;
        f.open_root("/rc")?;
        let d = f.navigate("/rd")?;
        f.open_root("/re")?;
        pump_for(1000);
        (b, d)
    };
    pump_for(1500);
    let before: BTreeMap<String, usize> = server.hits.lock().unwrap().clone();
    let f = Fixture::at(temp.path(), server.clone())?;
    f.pages.prepare_resume(&[])?;
    f.pages.resume()?;
    let resumed = wait_for(8000, || {
        f.cursor_path() == "/re" && !f.pages.selected_page().native.is_loading()
    });
    pump_for(1500);
    let after = server.hits.lock().unwrap().clone();
    let delta: Vec<(String, usize)> = after
        .iter()
        .filter(|(k, _)| k.starts_with('/'))
        .map(|(k, v)| (k.clone(), v - before.get(k).copied().unwrap_or(0)))
        .filter(|(_, v)| *v > 0)
        .collect();
    let procs = web_process_count();
    f.pages.go(Go::Visit(d))?;
    let back = f.arrives(d, 8000);
    println!(
        "RESULT restart_only_cursor resumed={resumed} procs={procs} requests_after_restart={delta:?} revisit_d={back} d_list_len={}",
        f.list_len()
    );
    Ok(())
}

fn many_branches() -> Result<()> {
    let f = Fixture::new()?;
    let mut pairs = Vec::new();
    let first = f.navigate("/m0")?;
    let second = f.navigate("/m0x")?;
    pairs.push((first, second));
    for i in 1..10 {
        let a = f.open_root(&format!("/m{i}"))?;
        let b = f.navigate(&format!("/m{i}x"))?;
        pairs.push((a, b));
    }
    pump_for(1500);
    let procs_rest = web_process_count();
    let mut lost = Vec::new();
    for i in [3usize, 7, 0, 9, 5, 1, 8, 2, 6, 4] {
        let (a, b) = pairs[i];
        f.pages.go(Go::Visit(b))?;
        if !f.arrives(b, 8000) {
            lost.push(format!("m{i}x not arrived"));
            continue;
        }
        if !f.history().contains(&a) {
            lost.push(format!("m{i} back list lost"));
        }
    }
    pump_for(1500);
    println!(
        "RESULT many_branches procs_at_rest={procs_rest} procs_end={} problems={lost:?} errors={}",
        web_process_count(),
        f.errors()
    );
    Ok(())
}

fn back_during_switch() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/x")])?;
    wait("x requested", || f.server.hits("/gate/x/x") > 0)?;
    for _ in 0..3 {
        f.pages.go(Go::Parent)?;
        pump_for(10);
    }
    pump_for(300);
    f.server.gate("x", false);
    wait_for(5000, || !f.pages.selected_page().native.is_loading());
    pump_for(1500);
    println!(
        "RESULT back_during_switch cursor={} a_requests={} x_opened={} events={:?}",
        f.cursor_path(),
        f.server.hits("/a"),
        f.events()
            .iter()
            .map(|e| format!("{}:{}", e.0, e.1))
            .collect::<Vec<_>>()
            .join(","),
        ""
    );
    Ok(())
}

fn reload_during_switch() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    f.navigate("/b")?;
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/x")])?;
    wait("x requested", || f.server.hits("/gate/x/x") > 0)?;
    f.pages.reload()?;
    pump_for(300);
    f.server.gate("x", false);
    wait_for(5000, || !f.pages.selected_page().native.is_loading());
    pump_for(1500);
    println!(
        "RESULT reload_during_switch cursor={} b_requests={} x_requests={}",
        f.cursor_path(),
        f.server.hits("/b"),
        f.server.hits("/gate/x/x")
    );
    Ok(())
}

/// Scroll restore when the switched-to page's tall content arrives late.
fn late_scroll(delay: u64) -> Result<()> {
    let f = Fixture::new()?;
    let path = format!("/late-tall/{delay}");
    // First load: fast (delay applies, but we wait it out).
    let s = f.navigate(&path)?;
    pump_for(500);
    f.eval("scrollTo(0,3000); return true;")?;
    pump_for(1200);
    let saved = f.pages.read(|b| b.scroll(s))?;
    f.open_root("/other")?;
    pump_for(800);
    f.pages.go(Go::Visit(s))?;
    let arrived = f.arrives(s, 15000);
    pump_for(4500);
    println!(
        "RESULT late_scroll delay={delay} saved={saved:?} arrived={arrived} scrollY={} requests={}",
        f.scroll_y(),
        f.server.hits(&path)
    );
    Ok(())
}

/// Scroll restore when a switch revives a checkpoint at a non-current entry.
fn scroll_noncurrent() -> Result<()> {
    let f = Fixture::new()?;
    let p1 = f.navigate("/p1")?;
    pump_for(400);
    f.eval("scrollTo(0,2000); return true;")?;
    pump_for(1200);
    let p2 = f.navigate("/p2")?;
    f.open_root("/other")?;
    pump_for(800);
    f.pages.go(Go::Visit(p1))?;
    let arrived = f.arrives(p1, 8000);
    pump_for(3000);
    println!(
        "RESULT scroll_noncurrent arrived={arrived} scrollY={} p1_requests={} history_has_p2={}",
        f.scroll_y(),
        f.server.hits("/p1"),
        f.history().contains(&p2)
    );
    Ok(())
}

fn same_url_two_branches() -> Result<()> {
    let f = Fixture::new()?;
    let a1 = f.navigate("/same")?;
    let x1 = f.navigate("/x1")?;
    let a2 = f.open_root("/same")?;
    let x2 = f.navigate("/x2")?;
    f.pages.go(Go::Visit(x1))?;
    let r1 = f.arrives(x1, 8000);
    pump_for(600);
    let h1 = (f.history().contains(&a1), f.history().contains(&a2));
    f.pages.go(Go::Visit(x2))?;
    let r2 = f.arrives(x2, 8000);
    pump_for(600);
    let h2 = (f.history().contains(&a1), f.history().contains(&a2));
    f.pages.go(Go::Parent)?;
    let r3 = f.arrives(a2, 8000);
    println!(
        "RESULT same_url arrived=({r1},{r2}) x1_history(a1,a2)={h1:?} x2_history(a1,a2)={h2:?} back_from_x2_reaches_a2={r3} cursor_is_a1={}",
        f.cursor() == Some(a1)
    );
    Ok(())
}

fn download_then_switch() -> Result<()> {
    use canopy::pages::DownloadEvent;
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let saved = Rc::new(Cell::new(0));
    let failed = Rc::new(Cell::new(0));
    let (s, x) = (saved.clone(), failed.clone());
    let errors = f.errors.clone();
    f.pages.observe(move |event| match event {
        PageEvent::Download(DownloadEvent::Saved(_)) => s.set(s.get() + 1),
        PageEvent::Download(DownloadEvent::Failed { .. }) => x.set(x.get() + 1),
        PageEvent::Error(e) => errors.borrow_mut().push(e),
        _ => {}
    });
    f.pages.go(Go::Address {
        url: f.server.url("/slowfile"),
        target: NavigationTarget::ChildOf(a),
    })?;
    pump_for(100);
    f.pages.open_roots(&[f.server.url("/delay/1500/root")])?;
    wait_for(6000, || f.cursor_path() == "/delay/1500/root");
    pump_for(2000);
    let downloads = f.pages.read(|b| b.downloads())?;
    println!(
        "RESULT download_then_switch cursor={} saved={} failed={} recorded={} recorded_on_a={}",
        f.cursor_path(),
        saved.get(),
        failed.get(),
        downloads.len(),
        downloads.iter().any(|d| d.visit == a)
    );
    Ok(())
}

/// A queued link activated, and while its switch loads, a second queued link from the
/// same page activated; then the first again.
fn redirect_switch() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    let root = {
        f.pages.open_roots(&[f.server.url("/redirect/landing")])?;
        let ok = wait_for(6000, || {
            f.cursor_path() == "/landing" && !f.pages.selected_page().native.is_loading()
        });
        println!("NOTE redirect arrived={ok} cursor={}", f.cursor_path());
        f.cursor().unwrap()
    };
    let child = f.navigate("/after-landing")?;
    f.open_root("/z")?;
    pump_for(600);
    f.pages.go(Go::Visit(root))?;
    let back = f.arrives(root, 6000);
    pump_for(600);
    println!(
        "RESULT redirect_switch revisit_landing={back} history_has_child={} landing_requests={} redirect_requests={}",
        f.history().contains(&child),
        f.server.hits("/landing"),
        f.server.hits("/redirect/landing")
    );
    Ok(())
}

/// A fragment link in a branch, then switch away and back.
fn fragment_switch() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    f.eval("location.hash='fragment'; return true;")?;
    pump_for(800);
    f.open_root("/z")?;
    pump_for(600);
    f.pages.go(Go::Visit(b))?;
    let back = f.arrives(b, 6000);
    pump_for(800);
    println!(
        "RESULT fragment_switch arrived={back} history_has_a={} list_len={} uri={:?} b_requests={}",
        f.history().contains(&a),
        f.list_len(),
        f.pages.selected_page().native.uri().map(|u| u.to_string()),
        f.server.hits("/b")
    );
    Ok(())
}

/// pushState in a branch, then switch away and back.
fn pushstate_switch() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    f.eval("history.pushState({}, '', '/b-pushed'); return true;")?;
    pump_for(800);
    f.open_root("/z")?;
    pump_for(600);
    f.pages.go(Go::Visit(b))?;
    let back = f.arrives(b, 6000);
    pump_for(800);
    println!(
        "RESULT pushstate_switch arrived={back} history_has_a={} list_len={} uri={:?} b_requests={} pushed_requests={}",
        f.history().contains(&a),
        f.list_len(),
        f.pages.selected_page().native.uri().map(|u| u.to_string()),
        f.server.hits("/b"),
        f.server.hits("/b-pushed")
    );
    Ok(())
}

fn native_list(f: &Fixture) -> Vec<String> {
    let list = f.pages.selected_page().native.back_forward_list().unwrap();
    let mut items = list.back_list();
    items.reverse();
    let mut out: Vec<String> = items
        .iter()
        .map(|i| {
            webkit2gtk::BackForwardListItemExt::uri(i)
                .map(|u| u.to_string())
                .unwrap_or_default()
        })
        .collect();
    if let Some(c) = list.current_item() {
        out.push(format!(
            "*{}",
            webkit2gtk::BackForwardListItemExt::uri(&c)
                .map(|u| u.to_string())
                .unwrap_or_default()
        ));
    }
    out.extend(list.forward_list().iter().map(|i| {
        webkit2gtk::BackForwardListItemExt::uri(i)
            .map(|u| u.to_string())
            .unwrap_or_default()
    }));
    out.into_iter()
        .map(|u| u.replace(&f.server.origin, ""))
        .collect()
}

fn back_hammer() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/ha")?;
    f.navigate("/hb")?;
    f.navigate("/hc")?;
    f.navigate("/hd")?;
    pump_for(500);
    for _ in 0..3 {
        f.pages.go(Go::Parent)?;
        pump_for(15);
    }
    wait_for(5000, || !f.pages.selected_page().native.is_loading());
    pump_for(1500);
    println!(
        "RESULT back_hammer cursor={} requests(ha,hb,hc)=({},{},{}) list={:?}",
        f.cursor_path(),
        f.server.hits("/ha"),
        f.server.hits("/hb"),
        f.server.hits("/hc"),
        native_list(&f)
    );
    Ok(())
}

fn settle_start_list() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.navigate("/b")?;
    let tree = f.tree(a);
    f.pages.settle_tree(tree)?;
    pump_for(800);
    f.navigate("/n1")?;
    f.navigate("/n2")?;
    println!(
        "RESULT settle_start_list native={:?} bound={}",
        native_list(&f),
        f.history().len()
    );
    Ok(())
}

/// Commands that arrive while the previous branch is being released.
fn release_window_commands() -> Result<()> {
    let f = Fixture::new()?;
    let y = f.navigate("/y")?;
    let tree_y = f.tree(y);
    f.open_root("/w")?;
    let weak = Rc::downgrade(&f.pages);
    let fired = Rc::new(Cell::new(false));
    let once = fired.clone();
    let errors = f.errors.clone();
    f.pages.observe(move |event| {
        if let PageEvent::Error(e) = &event {
            errors.borrow_mut().push(e.clone());
        }
        let Some(pages) = weak.upgrade() else { return };
        let on_root = pages
            .read(|b| b.snapshot())
            .ok()
            .and_then(|s| s.current_visit().map(|v| v.url.path() == "/root"))
            .unwrap_or(false);
        if matches!(event, PageEvent::Changed) && on_root && !once.replace(true) {
            let r1 = pages.settle_tree(tree_y);
            let r2 = pages.reload();
            println!(
                "NOTE release_window settle={:?} reload={:?}",
                r1.is_ok(),
                r2.is_ok()
            );
        }
    });
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait_for(6000, || fired.get());
    pump_for(3000);
    let settled = f
        .pages
        .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))
        .map(|forest| !forest.open().iter().any(|t| t.id == tree_y))
        .unwrap_or(false);
    println!(
        "RESULT release_window_commands fired={} cursor={} y_settled={settled} root_requests={} errors={}",
        fired.get(),
        f.cursor_path(),
        f.server.hits("/root"),
        f.errors()
    );
    Ok(())
}

fn back_cross_tree() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let r = f.open_root("/r")?;
    let before = f.pages.read(|b| b.snapshot())?.visits.len();
    f.pages.go(Go::Address {
        url: f.server.url("/c"),
        target: NavigationTarget::ChildOf(a),
    })?;
    wait("c", || {
        f.cursor_path() == "/c" && !f.pages.selected_page().native.is_loading()
    })?;
    pump_for(500);
    let list = native_list(&f);
    f.pages.go(Go::Parent)?;
    wait_for(5000, || {
        f.cursor_path() != "/c" && !f.pages.selected_page().native.is_loading()
    });
    pump_for(800);
    println!(
        "RESULT back_cross_tree list_before_back={list:?} cursor_after_back={} r_requests={}",
        f.cursor_path(),
        f.server.hits("/r")
    );
    Ok(())
}

fn procs_during_candidate() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    pump_for(800);
    let rest = web_process_count();
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/x")])?;
    wait("x requested", || f.server.hits("/gate/x/x") > 0)?;
    pump_for(300);
    let during = web_process_count();
    f.server.gate("x", false);
    f.arrives(f.visit_with_path("/gate/x/x").unwrap(), 5000);
    pump_for(2000);
    println!(
        "RESULT procs_during_candidate rest={rest} during={during} after={}",
        web_process_count()
    );
    Ok(())
}

fn busy_old_page() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/busy")?;
    pump_for(800); // busy loop running now (starts 600 ms after load, lasts 4 s)
    let start = Instant::now();
    f.pages.open_roots(&[f.server.url("/target")])?;
    let arrived = wait_for(8000, || {
        f.cursor_path() == "/target" && !f.pages.selected_page().native.is_loading()
    });
    let at = start.elapsed().as_millis();
    let start2 = Instant::now();
    f.pages.open_roots(&[f.server.url("/next")])?;
    let next = wait_for(8000, || {
        f.cursor_path() == "/next" && !f.pages.selected_page().native.is_loading()
    });
    println!(
        "RESULT busy_old_page arrived={arrived} ms={at} next_arrived={next} next_ms={}",
        start2.elapsed().as_millis()
    );
    Ok(())
}

fn restart_resume_branch() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let server = Server::new()?;
    {
        let f = Fixture::at(temp.path(), server.clone())?;
        f.navigate("/ra")?;
        f.navigate("/rb")?;
        pump_for(500);
    }
    pump_for(1000);
    let f = Fixture::at(temp.path(), server.clone())?;
    f.pages.prepare_resume(&[])?;
    f.pages.resume()?;
    wait("resumed", || {
        f.cursor_path() == "/rb" && !f.pages.selected_page().native.is_loading()
    })?;
    let rb = f.cursor().unwrap();
    let rc = f.navigate("/rc")?;
    let before = native_list(&f);
    f.open_root("/other")?;
    pump_for(800);
    f.pages.go(Go::Visit(rc))?;
    let back = f.arrives(rc, 6000);
    println!("RESULT restart_resume_branch arrived={back} list_before={before:?} history_has_rb={} list_after={:?}", f.history().contains(&rb), native_list(&f));
    Ok(())
}

fn focus_after_switch() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    f.window.present();
    f.pages.focus();
    pump_for(300);
    let before = f.pages.selected_page().native.has_focus();
    f.open_root("/b")?;
    pump_for(500);
    println!(
        "RESULT focus_after_switch before={before} after={}",
        f.pages.selected_page().native.has_focus()
    );
    Ok(())
}

fn replace_switch() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let r = f.navigate("/replaced")?;
    pump_for(800);
    let uri = f
        .pages
        .selected_page()
        .native
        .uri()
        .map(|u| u.to_string().replace(&f.server.origin, ""));
    let next = f.navigate("/after-replaced")?;
    f.pages.go(Go::Parent)?;
    let back = f.arrives(r, 6000);
    pump_for(500);
    f.open_root("/z")?;
    pump_for(800);
    f.pages.go(Go::Visit(r))?;
    let arrived = f.arrives(r, 6000);
    pump_for(500);
    println!(
        "RESULT replace_switch uri_after_load={uri:?} back_in_branch={back} arrived={arrived} history_has_a={} history_has_after={} list={:?}",
        f.history().contains(&a),
        f.history().contains(&next),
        native_list(&f)
    );
    Ok(())
}

fn iframe_back(path: &str) -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.navigate(path)?;
    pump_for(500);
    if path == "/frame-click" {
        // A user click on a link that targets the frame.
        let point = f.eval("const r=document.getElementById('l').getBoundingClientRect(); return [r.x+r.width/2,r.y+r.height/2];")?;
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
                "1",
            ])
            .status()?;
        assert!(status.success());
        wait_for(3000, || f.server.hits("/frame/c1") > 0);
        pump_for(800);
    } else {
        pump_for(1500);
    }
    let list = native_list(&f);
    let len = f.list_len();
    f.pages.go(Go::Parent)?;
    let back = f.arrives(a, 3000);
    println!(
        "RESULT iframe_back page={path} list_len={len} back_reaches_a_in_3s={back} cursor={} list={:?}",
        f.cursor_path(),
        list.iter().take(6).collect::<Vec<_>>()
    );
    Ok(())
}

fn saved_routing() -> Result<()> {
    let f = Fixture::new()?;
    f.pages.set_capture_enabled(true)?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    let captured = wait_for(6000, || {
        f.pages
            .read(|br| br.captures(b))
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    });
    let capture = f.pages.read(|br| br.captures(b))?.first().map(|c| c.id);
    let r = f.open_root("/r")?;
    let r2 = f.navigate("/r2")?;
    f.pages.go(Go::Parent)?;
    f.arrives(r, 5000);
    pump_for(500);
    let Some(id) = capture else {
        println!("RESULT saved_routing ERROR no capture (captured={captured})");
        return Ok(());
    };
    f.pages.go(Go::Saved(id))?;
    let arrived = f.arrives(b, 6000);
    pump_for(600);
    println!(
        "RESULT saved_routing arrived={arrived} list={:?} history_has_a={} history_has_r={} b_requests={}",
        native_list(&f),
        f.history().contains(&a),
        f.history().contains(&r),
        f.server.hits("/b")
    );
    f.pages.go(Go::Visit(r2))?;
    let back = f.arrives(r2, 6000);
    pump_for(500);
    println!(
        "RESULT saved_routing_r2 arrived={back} list={:?} r2_requests={}",
        native_list(&f),
        f.server.hits("/r2")
    );
    Ok(())
}

fn pushstate_back_measure() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/pa")?;
    f.navigate("/pb")?;
    f.eval("history.pushState({},'', '/pb?1'); history.pushState({},'', '/pb?2'); return true;")?;
    pump_for(300);
    f.pages.go(Go::Parent)?;
    pump_for(1500);
    println!(
        "RESULT pushstate_back cursor={} uri={:?}",
        f.cursor_path(),
        f.pages.selected_page().native.uri()
    );
    Ok(())
}
fn main() -> Result<()> {
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
        ("pushstate_back", pushstate_back_measure),
        ("iframe_pulse", || background_navigation("/iframe-pulse")),
        ("iframe_once", || background_navigation("/iframe-once")),
        ("hash_pulse", || background_navigation("/hash-pulse")),
        ("iframe_once_during", || {
            background_navigation_after("/iframe-once", 150)
        }),
        ("script_nav_during", || {
            background_navigation_after("/script-nav", 150)
        }),
        ("rotate_1000", || rotate_success(1000)),
        ("rotate_3000", || rotate_success(3000)),
        ("old_inflight", old_inflight),
        ("retire_race", retire_race),
        ("settle_other", settle_other_during_switch),
        ("settle_current", settle_current_during_switch),
        ("rapid_roots", rapid_roots),
        ("rapid_trees", rapid_trees),
        ("post_switch_back", post_switch_back),
        ("settle_start", settle_start_new_tree),
        ("nocontent", nocontent_switch),
        ("restart", restart_only_cursor),
        ("many_branches", many_branches),
        ("back_during_switch", back_during_switch),
        ("reload_during_switch", reload_during_switch),
        ("late_scroll_0", || late_scroll(0)),
        ("late_scroll_4000", || late_scroll(4000)),
        ("scroll_noncurrent", scroll_noncurrent),
        ("same_url", same_url_two_branches),
        ("download_then_switch", download_then_switch),
        ("redirect_switch", redirect_switch),
        ("fragment_switch", fragment_switch),
        ("pushstate_switch", pushstate_switch),
        ("back_hammer", back_hammer),
        ("settle_start_list", settle_start_list),
        ("release_window_commands", release_window_commands),
        ("back_cross_tree", back_cross_tree),
        ("procs_during_candidate", procs_during_candidate),
        ("busy_old_page", busy_old_page),
        ("restart_resume_branch", restart_resume_branch),
        ("focus_after_switch", focus_after_switch),
        ("popup_during", || {
            background_navigation_after("/popup-timer", 150)
        }),
        ("saved_routing", saved_routing),
        ("replace_switch", replace_switch),
        ("iframe_back_rotate", || iframe_back("/iframe-rotate/300")),
        ("iframe_back_click", || iframe_back("/frame-click")),
    ];
    for (name, run) in cases {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        match run() {
            Ok(()) => println!("DONE {name}"),
            Err(error) => println!("RESULT {name} ERROR {error:#}"),
        }
        pump_for(300);
    }
    Ok(())
}
