// GUI regressions adapted from the independent round-two fixture.
// Every selected case asserts behavior; diagnostic RESULT lines aid comparison.
#![allow(dead_code, unused_imports, unused_variables)]
use anyhow::{bail, Context, Result};
use canopy::{
    browser::{NavigationTarget, TreeId, VisitId},
    forest::ForestProjection,
    pages::Go,
    ui::Ui,
};
use gtk::prelude::*;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use webkit2gtk::{BackForwardListExt, WebViewExt};
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
mod common;
use common::*;

struct U {
    ui: Rc<Ui>,
    server: Server,
    _dir: tempfile::TempDir,
}
impl U {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let ui = Ui::new(dir.path())?;
        ui.window.show_all();
        ready(&ui)?;
        Ok(U {
            ui,
            server: Server::new()?,
            _dir: dir,
        })
    }
    fn cursor(&self) -> Option<VisitId> {
        self.ui.snapshot().ok()?.current
    }
    fn cursor_path(&self) -> String {
        self.ui
            .snapshot()
            .ok()
            .and_then(|s| s.current_visit().map(|v| v.url.path().to_string()))
            .unwrap_or_else(|| "<none>".into())
    }
    fn loading(&self) -> bool {
        self.ui.pages.selected_page().native.is_loading()
    }
    fn uri(&self) -> String {
        self.ui
            .pages
            .selected_page()
            .native
            .uri()
            .map(|u| u.to_string().replace(&self.server.origin, ""))
            .unwrap_or_else(|| "<none>".into())
    }
    fn arrives(&self, id: VisitId, ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            pump(5);
            if self.cursor() == Some(id) && !self.loading() {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
        }
    }
    fn until(&self, ms: u64, f: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            pump(5);
            if f() {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
        }
    }
    fn nav(&self, path: &str) -> Result<VisitId> {
        let target = self
            .cursor()
            .map(NavigationTarget::ChildOf)
            .unwrap_or(NavigationTarget::Root);
        self.ui.pages.go(Go::Address {
            url: self.server.url(path),
            target,
        })?;
        wait(&format!("nav {path}"), || {
            self.cursor_path() == path && !self.loading()
        })?;
        Ok(self.cursor().unwrap())
    }
    fn root(&self, path: &str) -> Result<VisitId> {
        self.ui.pages.open_roots(&[self.server.url(path)])?;
        wait(&format!("root {path}"), || {
            self.cursor_path() == path && !self.loading()
        })?;
        Ok(self.cursor().unwrap())
    }
    fn page_click(&self, id: &str, button: &str) -> Result<()> {
        let (x, y) = element_point(&self.ui, id)?;
        focus_window()?;
        move_to(x, y)?;
        xdo(&["click", button])?;
        pump(80);
        Ok(())
    }
    fn painted(&self) -> Result<()> {
        eval(&self.ui, "globalThis.__p=false; requestAnimationFrame(()=>requestAnimationFrame(()=>{globalThis.__p=true})); JSON.stringify(true)")?;
        wait("painted", || {
            eval(&self.ui, "JSON.stringify(globalThis.__p)").ok() == Some(json!(true))
        })
    }
    fn mouse_back(&self) -> Result<()> {
        let (x, y) = page_point(&self.ui, 600, 500)?;
        focus_window()?;
        move_to(x, y)?;
        xdo(&["click", "8"])?;
        pump(80);
        Ok(())
    }
    fn tab(&self, root: VisitId) -> String {
        format!("#tab-row [data-id='{root}']")
    }
    fn list(&self) -> Vec<String> {
        let Some(list) = self.ui.pages.selected_page().native.back_forward_list() else {
            return vec![];
        };
        let name = |i: &webkit2gtk::BackForwardListItem| {
            webkit2gtk::BackForwardListItemExt::uri(i)
                .map(|u| u.to_string().replace(&self.server.origin, ""))
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
}

/// Tabs, digit shortcuts and Alt+Left through the real chrome, then quick alternations.
fn ui_mouse_back() -> Result<()> {
    let u = U::new()?;
    let a = u.root("/a")?;
    let b = u.nav("/b")?;
    pump(300);
    u.mouse_back()?;
    let plain = u.arrives(a, 4000);
    pump(300);
    // A queued link opened from the strip's queue: middle-click the page's link.
    let q_before = u.ui.snapshot()?.visits.len();
    u.page_click("fork", "2")?;
    let queued = u.until(4000, || {
        u.ui.snapshot()
            .is_ok_and(|s| s.visits.len() == q_before + 1)
    });
    let q =
        u.ui.snapshot()?
            .visits
            .iter()
            .find(|v| v.url.path() == "/d")
            .map(|v| v.id);
    let mut on_q = false;
    let mut q_back = false;
    let mut after = String::new();
    let mut list_q = vec![];
    if let Some(q) = q {
        u.ui.pages.go(Go::Visit(q))?;
        on_q = u.arrives(q, 6000);
        pump(500);
        list_q = u.list();
        u.mouse_back()?;
        q_back = u.arrives(a, 4000);
        after = format!("{}|{}", u.cursor_path(), u.uri());
        if !q_back {
            key("alt+Left")?;
            let alt = u.arrives(a, 5000);
            after += &format!(" alt_left_reaches_parent={alt}");
        }
    }
    // After a switch away and back, the mouse back button inside a revived branch.
    let r = u.root("/r")?;
    pump(500);
    u.ui.pages.go(Go::Visit(b))?;
    let at_b = u.arrives(b, 6000);
    pump(500);
    let list_b = u.list();
    u.mouse_back()?;
    let revived_back = u.arrives(a, 4000);
    println!(
        "RESULT ui_mouse_back plain={plain} queued={queued} on_q={on_q} list_q={list_q:?} q_mouse_back_reaches_parent={q_back} after_q_back={after} at_b={at_b} list_b={list_b:?} revived_mouse_back={revived_back} cursor={} d_requests={} a_requests={}",
        u.cursor_path(),
        u.server.hits("/d"),
        u.server.hits("/a")
    );
    let _ = r;
    assert!(q_back, "N5: mouse Back must reach forest parent");
    Ok(())
}

/// A queued node clicked in the strip opens its page and then the branch palette.
fn ui_tab_collapse() -> Result<()> {
    let u = U::new()?;
    let widths = Rc::new(std::cell::RefCell::new(Vec::<i32>::new()));
    let seen = widths.clone();
    u.ui.chrome
        .native()
        .connect_size_allocate(move |_, a| seen.borrow_mut().push(a.width()));
    let mut collapses = Vec::new();
    for n in 1..=12 {
        let target = u
            .cursor()
            .map(|_| NavigationTarget::Root)
            .unwrap_or(NavigationTarget::Root);
        u.ui.pages.go(Go::Address {
            url: u.server.url(&format!("/topic-{n}")),
            target,
        })?;
        wait("topic", || {
            u.cursor_path() == format!("/topic-{n}") && !u.loading()
        })?;
        pump(700);
        let state = chrome(
            &u.ui,
            "(() => { const t=[...document.querySelectorAll('#tab-row .tab')]; return [t.length, t.filter(x=>x.hidden).length, t.length ? t[0].hidden : false, document.getElementById('tabs').clientWidth]; })()",
        )?;
        let total = state[0].as_u64().unwrap_or(0);
        let hidden = state[1].as_u64().unwrap_or(0);
        let first_hidden = state[2].as_bool().unwrap_or(false);
        if first_hidden {
            collapses.push(format!("after{n}:{hidden}/{total}@{}", state[3]));
        }
    }
    let w = widths.borrow();
    let window = u.ui.window.allocated_width();
    let small = w.iter().filter(|&&x| x < window).count();
    println!(
        "RESULT ui_tab_collapse window_width={window} chrome_allocations={} small_allocations={small} min_width={:?} collapsed_at_rest={collapses:?}",
        w.len(),
        w.iter().min()
    );
    assert_eq!(small, 0, "N7: chrome narrowed during page change");
    assert!(collapses.is_empty(), "N7: collapsed tab row");
    Ok(())
}

/// Opens a new tree, then 300 ms after it shows, clicks the first tree's tab, ten times.
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
    start(environment.path())?;
    let only: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    type Case = (&'static str, fn() -> Result<()>);
    let cases: &[Case] = &[("mouse", ui_mouse_back), ("allocation", ui_tab_collapse)];
    for (name, run) in cases {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        match run() {
            Ok(()) => println!("DONE {name}"),
            Err(error) => return Err(error.context(format!("round3 UI {name}"))),
        }
    }
    Ok(())
}
