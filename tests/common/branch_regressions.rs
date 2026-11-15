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
    assert!(arrived, "B1: frame navigation cancelled switch");
    Ok(())
}

/// Repeated quick switches away from a page whose ad-like frame rotates every `ms`.
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
    assert!(arrived);
    assert_eq!(
        f.visits_with_path("/delay/700/abandoned"),
        0,
        "S2: abandoned visit committed"
    );
    assert_eq!(events.len(), 1, "S2: arrival log moved more than once");
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
    assert!(arrived, "S2: retiring request cancelled switch");
    assert_eq!(f.server.hits("/two"), 0);
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
    assert!(arrived, "S1: settling unrelated tree cancelled switch");
    Ok(())
}

/// Settling the current tree while a switch to another tree loads.
fn post_switch_back() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let form = f.navigate("/form")?;
    let before = f.pages.read(|b| b.snapshot())?.visits.len();
    click(&f, "go")?;
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
    assert!(arrived);
    assert!(
        f.history().contains(&form),
        "S3: POST revival lost back list"
    );
    assert!(f.history().contains(&a));
    assert!(!f.history().contains(&next), "GET drops forward entries");
    assert_eq!(f.server.hits("POST /post-result"), posts_before);
    assert_eq!(f.server.hits("GET /post-result"), 1);
    f.pages.go(Go::Parent)?;
    assert!(f.arrives(form, 6000), "S3: Back must reach form");
    f.pages.go(Go::Parent)?;
    assert!(f.arrives(a, 6000));
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
    assert!(f.history().contains(&n1), "S4: branch was not kept");
    let revived = f.pages.selected_page().native;
    f.pages.go(Go::Parent)?;
    let back = f.arrives(n1, 6000);
    println!(
        "RESULT settle_start_back back_to_n1={back} cursor={} n1_requests={}",
        f.cursor_path(),
        f.server.hits("/n1")
    );
    assert!(back, "S4: Back did not reach new root");
    assert_eq!(
        f.pages.selected_page().native,
        revived,
        "S4: Back must traverse within revived view"
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
    assert!(back, "S5: Back stepped a frame");
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
    assert!(
        f.history().contains(&a),
        "S6: saved capture lost its branch history"
    );
    f.pages.go(Go::Visit(r2))?;
    let back = f.arrives(r2, 6000);
    pump_for(500);
    println!(
        "RESULT saved_routing_r2 arrived={back} list={:?} r2_requests={}",
        native_list(&f),
        f.server.hits("/r2")
    );
    assert!(arrived && back);
    assert!(
        f.history().contains(&r),
        "S6: saved capture truncated other branch"
    );
    assert_eq!(f.server.hits("/r2"), 2);
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
    wait("restart releases previous web process", || {
        web_process_count() == 1
    })?;
    let procs = web_process_count();
    f.pages.go(Go::Visit(d))?;
    let back = f.arrives(d, 8000);
    println!(
        "RESULT restart_only_cursor resumed={resumed} procs={procs} requests_after_restart={delta:?} revisit_d={back} d_list_len={}",
        f.list_len()
    );
    assert!(resumed && back);
    assert_eq!(
        delta,
        vec![("/re".into(), 1)],
        "restart fetched noncursor pages"
    );
    assert_eq!(procs, 1);
    Ok(())
}

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
    assert!(fired.get() && settled);
    assert_eq!(f.cursor_path(), "/root");
    assert_eq!(f.server.hits("/root"), 2);
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

fn click(f: &Fixture, id: &str) -> Result<()> {
    let point=f.eval(&format!("const r=document.getElementById('{id}').getBoundingClientRect(); return [r.x+r.width/2,r.y+r.height/2];"))?;
    let native = f.pages.selected_page().native.clone();
    let (x, y) = native
        .translate_coordinates(
            &f.window,
            point[0].as_f64().unwrap() as i32,
            point[1].as_f64().unwrap() as i32,
        )
        .unwrap();
    let (_, ox, oy) = f.window.window().unwrap().origin();
    assert!(std::process::Command::new("xdotool")
        .args([
            "mousemove",
            &(ox + x).to_string(),
            &(oy + y).to_string(),
            "click",
            "1"
        ])
        .status()?
        .success());
    Ok(())
}
fn click_during_switch() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    f.server.gate("x", true);
    f.pages.open_roots(&[f.server.url("/gate/x/target")])?;
    wait("candidate request", || f.server.hits("/gate/x/target") > 0)?;
    click(&f, "fork")?;
    wait("old page link", || f.cursor_path() == "/d")?;
    f.server.gate("x", false);
    pump_for(500);
    assert_eq!(f.cursor_path(), "/d");
    Ok(())
}
fn script_during_switch() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/script-nav")?;
    pump_for(150);
    f.pages.open_roots(&[f.server.url("/delay/1500/target")])?;
    pump_for(2500);
    assert_eq!(f.cursor_path(), "/after-script");
    Ok(())
}
fn failed_loading_signal() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    f.pages.observe(move |event| {
        if matches!(event, PageEvent::Error(_) | PageEvent::Loading(false)) {
            seen.borrow_mut()
                .push(matches!(event, PageEvent::Loading(false)));
        }
    });
    let socket = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/refused", socket.local_addr()?);
    drop(socket);
    f.pages.open_roots(&[url.parse()?])?;
    wait("candidate error", || !events.borrow().is_empty())?;
    pump_for(300);
    assert_eq!(
        &*events.borrow(),
        &[false, true],
        "M2: missing trailing Loading(false)"
    );
    Ok(())
}
fn citation_routing() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let child = f.pages.read(|b| b.snapshot())?.current.unwrap();
    // Queue through the page so the source has a citation and a real parent.
    f.eval("document.getElementById('fork').href='/d'; return true;")?;
    let source = canopy::browser::SourceContext {
        page_url: f.server.url("/a"),
        citation_url: Some(f.server.url("/a#fragment")),
        link_text: "citation".into(),
        excerpt: String::new(),
    };
    let mut browser = Browser::open(&f.dir.join("forest.sqlite3"))?;
    let queued = browser.queue_link(a, f.server.url("/d"), source)?;
    drop(browser);
    let r = f.open_root("/r")?;
    let r2 = f.navigate("/r2")?;
    f.pages.go(Go::Citation(queued))?;
    wait("citation child", || {
        f.cursor_path() == "/a" && f.cursor() != Some(a)
    })?;
    let citation = f.cursor().unwrap();
    assert_eq!(f.pages.read(|b| b.visit(citation))?.parent, Some(a));
    assert!(
        !f.history().contains(&r),
        "S6: citation entered unrelated branch"
    );
    f.pages.go(Go::Visit(r2))?;
    f.arrived(r2)?;
    assert!(f.history().contains(&r));
    f.pages.go(Go::Parent)?;
    assert!(f.arrives(r, 6000));
    let _ = child;
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
    assert_eq!(f.cursor_path(), "/a");
    assert!(
        f.server.hits("/a") <= 2,
        "M1: Back fetched target more than main"
    );
    Ok(())
}

fn saved_citation_routing() -> Result<()> {
    let f = Fixture::new()?;
    f.pages.set_capture_enabled(true)?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    wait("capture", || {
        f.pages
            .read(|br| br.captures(b))
            .is_ok_and(|c| !c.is_empty())
    })?;
    let capture = f.pages.read(|br| br.captures(b))?[0].id;
    rusqlite::Connection::open(f.dir.join("forest.sqlite3"))?.execute(
        "UPDATE captures SET citation_url=?1 WHERE id=?2",
        rusqlite::params![f.server.url("/b#fragment").as_str(), capture],
    )?;
    let r = f.open_root("/r")?;
    f.pages.go(Go::Saved(capture))?;
    wait("saved citation", || {
        f.cursor_path() == "/b" && f.cursor() != Some(b)
    })?;
    let child = f.cursor().unwrap();
    assert_eq!(f.pages.read(|br| br.visit(child))?.parent, Some(b));
    assert!(f.history().contains(&a));
    assert!(!f.history().contains(&r));
    f.pages.go(Go::Parent)?;
    assert!(f.arrives(b, 6000));
    Ok(())
}
fn tls_candidate_failure() -> Result<()> {
    struct ServerProcess(std::process::Child);
    impl Drop for ServerProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let key = f.dir.join("key.pem");
    let cert = f.dir.join("cert.pem");
    assert!(std::process::Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-keyout"
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?
        .success());
    let socket = TcpListener::bind("127.0.0.1:0")?;
    let address = socket.local_addr()?;
    drop(socket);
    let _server = ServerProcess(
        std::process::Command::new("openssl")
            .args(["s_server", "-accept", &address.to_string(), "-cert"])
            .arg(&cert)
            .arg("-key")
            .arg(&key)
            .arg("-www")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
    );
    wait("TLS fixture listening", || {
        std::net::TcpStream::connect(address).is_ok()
    })?;
    f.pages
        .open_roots(&[format!("https://{address}/").parse()?])?;
    wait("TLS certificate rejected", || {
        f.errors
            .borrow()
            .iter()
            .any(|e| e.contains("HTTPS certificate rejected"))
    })?;
    assert_eq!(f.cursor(), Some(a));
    wait("TLS candidate released", || web_process_count() == 1)?;
    Ok(())
}
fn arrival_once() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    let before = f.events().len();
    f.open_root("/r")?;
    pump_for(300);
    assert_eq!(f.events().len(), before + 1);
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    pump_for(300);
    assert_eq!(f.events().len(), before + 2);
    assert_eq!(f.pages.read(|br| br.tree_target(f.tree(a)))?, b);
    Ok(())
}
type Case = (&'static str, fn() -> Result<()>);
pub fn cases() -> Vec<Case> {
    vec![
        ("tls_candidate_failure", tls_candidate_failure),
        ("arrival_once", arrival_once),
        ("saved_citation_routing", saved_citation_routing),
        ("back_switch_requests", back_during_switch),
        ("click_during_switch", click_during_switch),
        ("script_during_switch", script_during_switch),
        ("failed_loading_signal", failed_loading_signal),
        ("citation_routing", citation_routing),
        ("frame_during_switch", || {
            background_navigation_after("/iframe-once", 150)
        }),
        ("frame_pulse_switch", || {
            background_navigation("/iframe-pulse")
        }),
        ("abandoned_foreground", old_inflight),
        ("retiring_foreground", retire_race),
        ("settle_unrelated", settle_other_during_switch),
        ("post_current_revival", post_switch_back),
        ("settle_fresh_branch", settle_start_new_tree),
        ("frame_page_back", || iframe_back("/iframe-rotate/300")),
        ("saved_branch_routing", saved_routing),
        ("restart_cursor_only", restart_only_cursor),
        ("release_settle_reload", release_window_commands),
    ]
}
