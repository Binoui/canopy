// Branch navigation regressions.
// Every selected case asserts behavior; diagnostic RESULT lines aid comparison.
#![allow(dead_code, unused_imports, unused_variables)]
#[path = "common/http.rs"]
mod http;
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
    io::Write,
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
    if path == "/colour/overlay-hold" {
        return "<!doctype html><body style='margin:0;background:#20b060'><script>const b=document.body;b.remove();setTimeout(()=>document.documentElement.append(b),750)</script>".into();
    }
    if path.starts_with("/colour/") {
        let colour = if path == "/colour/third-busy" {
            "#2040d0"
        } else if path.ends_with('a') {
            "#d02040"
        } else {
            "#20b060"
        };
        let busy = if path == "/colour/short-busy" {
            "<script>const until=Date.now()+600;while(Date.now()<until){}</script>"
        } else if path == "/colour/busy" || path == "/colour/third-busy" {
            "<script>const until=Date.now()+4000;while(Date.now()<until){}</script>"
        } else {
            ""
        };
        return format!("<!doctype html><body style='margin:0;background:{colour}'><a id='fork' href='/d'>fork</a><script>window.resizes=[];addEventListener('resize',()=>resizes.push([innerWidth,innerHeight]))</script>{busy}");
    }
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
impl Drop for U {
    fn drop(&mut self) {
        self.server.gates.lock().unwrap().clear();
        self.ui.window.close();
        pump(50);
    }
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
        if let Err(error) = wait(&format!("root {path}"), || {
            self.cursor_path() == path && !self.loading()
        }) {
            let parent = self
                .ui
                .pages
                .selected_page()
                .native
                .parent()
                .unwrap()
                .downcast::<gtk::Fixed>()
                .unwrap();
            let views: Vec<_> = parent
                .children()
                .into_iter()
                .filter_map(|w| w.downcast::<webkit2gtk::WebView>().ok())
                .map(|v| {
                    (
                        v.as_ptr() as usize,
                        v.uri(),
                        v.is_loading(),
                        v.estimated_load_progress(),
                        v.is_mapped(),
                        v.allocation(),
                    )
                })
                .collect();
            eprintln!(
                "ROOT_WAIT path={path} hits={} cursor={} selected_uri={} views={views:?}",
                self.server.hits(path),
                self.cursor_path(),
                self.uri()
            );
            return Err(error);
        }
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
    assert!(q_back, "mouse Back must reach forest parent");
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
    assert_eq!(small, 0, "chrome narrowed during page change");
    assert!(collapses.is_empty(), "collapsed tab row");
    Ok(())
}

fn geometry(u: &U) -> Result<()> {
    let native = u.ui.pages.selected_page().native.clone();
    let clock = Instant::now();
    let allocations = Rc::new(std::cell::RefCell::new(Vec::new()));
    let history = allocations.clone();
    let allocation_signal = native.connect_size_allocate(move |_, a| {
        history.borrow_mut().push((clock.elapsed().as_millis(), *a))
    });
    let presses = Rc::new(std::cell::RefCell::new(Vec::new()));
    let clicks = presses.clone();
    let press_signal = native.connect_button_press_event(move |view, event| {
        clicks.borrow_mut().push((
            "page",
            clock.elapsed().as_millis(),
            event.position(),
            view.allocation(),
            view.allocation(),
        ));
        glib::Propagation::Proceed
    });
    let mut other_presses = Vec::new();
    for (label, target) in [
        ("window", u.ui.window.clone().upcast::<gtk::Widget>()),
        ("chrome", u.ui.chrome.native().upcast::<gtk::Widget>()),
    ] {
        let clicks = presses.clone();
        let page = native.clone();
        let handler = target.connect_button_press_event(move |widget, event| {
            clicks.borrow_mut().push((
                label,
                clock.elapsed().as_millis(),
                event.position(),
                page.allocation(),
                widget.allocation(),
            ));
            glib::Propagation::Proceed
        });
        other_presses.push((target, handler));
    }
    let a = native.allocation();
    assert_eq!(
        (a.x(), a.y(), a.width(), a.height()),
        (
            0,
            94,
            u.ui.window.allocated_width(),
            u.ui.window.allocated_height() - 94
        ),
        "selected page must fill content rectangle"
    );
    wait("page viewport matches allocation", || {
        eval(&u.ui, "JSON.stringify([innerWidth,innerHeight])").ok()
            == Some(json!([a.width(), a.height()]))
    })?;
    // GDK returns native children in topmost-first stacking order.
    let chrome = u.ui.chrome.native().window().unwrap();
    let page = u.ui.pages.selected_page().native.window().unwrap();
    let windows = chrome.parent().unwrap().children();
    let ci = windows.iter().position(|w| *w == chrome).unwrap();
    let pi = windows.iter().position(|w| *w == page).unwrap();
    println!("STACK chrome={ci} page={pi} allocation={a:?}");
    assert!(
        ci < pi,
        "chrome must be above page immediately after showing it"
    );
    click_chrome(&u.ui, "#search-pill")?;
    wait("chrome responds immediately", || visible(&u.ui, "#palette"))?;
    key("Escape")?;
    if let Some(original) = u.cursor() {
        eval(&u.ui, "scrollTo(0,0);document.getElementById('fork').href='/d';document.getElementById('fork').style.cssText='display:block;margin-top:150px;height:60px';JSON.stringify(true)")?;
        eval(&u.ui, "window.name='[]';window.__placementEvents=[];for(const kind of ['resize','pointerdown'])addEventListener(kind,e=>{__placementEvents.push({kind,t:performance.now(),w:innerWidth,h:innerHeight,x:e.clientX,y:e.clientY,target:e.target?.id});window.name=JSON.stringify(__placementEvents)},true);JSON.stringify(true)")?;
        let click_at = clock.elapsed().as_millis();
        u.page_click("fork", "1")?;
        let followed = u.until(6000, || u.cursor_path() == "/d" && !u.loading());
        let dom_events = eval(&u.ui, "JSON.stringify(window.name)")?;
        println!("PLACEMENT_CLICK start={click_at} followed={followed} presses={:?} allocations={:?} dom={dom_events}", presses.borrow(), allocations.borrow());
        anyhow::ensure!(followed, "page follows a real link after placement");
        let (x, y) = page_point(&u.ui, u.ui.window.allocated_width() / 2, 400)?;
        move_to(x, y)?;
        xdo(&["click", "5"])?;
        wait("placed page receives wheel", || {
            eval(&u.ui, "JSON.stringify(scrollY)")
                .ok()
                .and_then(|v| v.as_f64())
                .is_some_and(|y| y > 0.)
        })?;
        u.ui.pages.go(Go::Visit(original))?;
        anyhow::ensure!(
            u.arrives(original, 6000),
            "return after checking placed page input"
        );
    }
    for (widget, signal) in other_presses {
        widget.disconnect(signal);
    }
    native.disconnect(allocation_signal);
    native.disconnect(press_signal);
    Ok(())
}

fn resize_and_show() -> Result<()> {
    for (w, h) in [
        (900, 640),
        (1440, 1000),
        (390, 600),
        (320, 500),
        (1240, 800),
    ] {
        let u = U::new()?;
        let a = u.root("/a")?;
        u.ui.window.resize(w, h);
        pump(250);
        let b = u.root("/b")?;
        geometry(&u)?;
        u.ui.pages.go(Go::Visit(a))?;
        anyhow::ensure!(u.arrives(a, 6000), "revived root arrives");
        geometry(&u)?;
        if w < 900 {
            key("ctrl+2")?;
        } else {
            click_chrome(&u.ui, &u.tab(b))?;
        }
        anyhow::ensure!(u.arrives(b, 6000), "tab click arrives");
        geometry(&u)?;
        key("ctrl+1")?;
        anyhow::ensure!(
            u.arrives(a, 6000),
            "digit switch arrives: cursor={} uri={} expected={a}",
            u.cursor_path(),
            u.uri()
        );
        geometry(&u)?;
        // Use a link below every chrome cover, and then scroll the loaded page.
        eval(&u.ui, "document.getElementById('fork').style.cssText='display:block;margin-top:150px;height:60px';JSON.stringify(true)")?;
        u.page_click("fork", "1")?;
        anyhow::ensure!(
            u.until(4000, || u.cursor_path() == "/d" && !u.loading()),
            "resized page follows real link"
        );
        let (x, y) = page_point(&u.ui, 200, 400)?;
        move_to(x, y)?;
        xdo(&["click", "5"])?;
        wait("resized page receives wheel", || {
            eval(&u.ui, "JSON.stringify(scrollY)")
                .ok()
                .and_then(|v| v.as_f64())
                .is_some_and(|y| y > 0.)
        })?;
        key("alt+Left")?;
        anyhow::ensure!(u.arrives(a, 6000), "Back arrives");
        geometry(&u)?;
        if w == 900 {
            eval(&u.ui, "document.getElementById('fork').href='/queued';document.getElementById('fork').style.cssText='display:block;margin-top:150px;height:60px';JSON.stringify(true)")?;
            u.page_click("fork", "2")?;
            wait("queued page recorded", || {
                u.ui.snapshot()
                    .is_ok_and(|s| s.visits.iter().any(|v| v.url.path() == "/queued"))
            })?;
            let q =
                u.ui.snapshot()?
                    .visits
                    .iter()
                    .find(|v| v.url.path() == "/queued")
                    .unwrap()
                    .id;
            item_list(&u.ui, "#strip-nodes .after")?;
            click_chrome(&u.ui, &node(q))?;
            anyhow::ensure!(u.arrives(q, 6000), "queued strip node arrives");
            geometry(&u)?;
            key("alt+Left")?;
            anyhow::ensure!(u.arrives(a, 6000), "Back revives parent branch");
            geometry(&u)?;
            u.server.gate("x", true);
            u.ui.pages.open_roots(&[u.server.url("/gate/x/resize")])?;
            anyhow::ensure!(
                u.until(4000, || u.server.hits("/gate/x/resize") > 0),
                "resize candidate requested"
            );
            u.ui.window.resize(1000, 700);
            pump(150);
            u.server.gate("x", false);
            anyhow::ensure!(
                u.until(6000, || u.cursor_path() == "/gate/x/resize" && !u.loading()),
                "resized candidate arrives"
            );
            geometry(&u)?;
        }
    }
    Ok(())
}

fn chrome_after_changes() -> Result<()> {
    let u = U::new()?;
    let chrome = u.ui.chrome.native().downgrade();
    let maps = Rc::new(std::cell::Cell::new(0));
    let seen = maps.clone();
    u.ui.pages.observe_page_placement(move |view| {
        let page = view.window().unwrap();
        let chrome = chrome.upgrade().unwrap().window().unwrap();
        let children = chrome.parent().unwrap().children();
        let ci = children.iter().position(|w| *w == chrome).unwrap();
        let pi = children.iter().position(|w| *w == page).unwrap();
        println!("MAP_STACK chrome={ci} page={pi}");
        assert!(
            ci < pi,
            "chrome must be above a mapped page before page events"
        );
        seen.set(seen.get() + 1);
    });
    let a = u.root("/a")?;
    let b = u.root("/b")?;
    u.ui.pages.go(Go::Visit(a))?;
    anyhow::ensure!(u.arrives(a, 6000), "revival arrives");
    geometry(&u)?;
    // Cancel a hidden candidate, then click the chrome immediately.
    u.server.gate("x", true);
    u.ui.pages.open_roots(&[u.server.url("/gate/x/target")])?;
    anyhow::ensure!(
        u.until(4000, || u.server.hits("/gate/x/target") > 0),
        "candidate starts"
    );
    let parent =
        u.ui.pages
            .selected_page()
            .native
            .parent()
            .unwrap()
            .downcast::<gtk::Fixed>()
            .unwrap();
    let rect = u.ui.pages.selected_page().native.allocation();
    for child in parent.children() {
        if child == u.ui.chrome.native().upcast::<gtk::Widget>() {
            continue;
        }
        println!(
            "HIDDEN_RECT visible={} allocation={:?}",
            child.is_visible(),
            child.allocation()
        );
        assert_eq!(
            child.allocation(),
            rect,
            "hidden candidates must already have the content rectangle"
        );
    }
    key("Escape")?;
    u.ui.pages.go(Go::Visit(a))?;
    u.server.gate("x", false);
    geometry(&u)?;
    let trees =
        u.ui.pages
            .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
    for t in trees.tabs() {
        u.ui.pages
            .place_tree(t.id, canopy::browser::Move::History)?;
        pump(100);
    }
    geometry(&u)?;
    // Trusted chrome click on the empty start page: opening the palette covers it.
    click_chrome(&u.ui, "#new-tree")?;
    assert!(
        maps.get() >= 4,
        "all kinds of view changes must place their native windows"
    );
    let _ = b;
    Ok(())
}

fn screen(ui: &Ui) -> Result<gtk::gdk_pixbuf::Pixbuf> {
    let (_, x, y) = ui.window.window().context("window")?.origin();
    let a = ui.window.allocation();
    gdk::Screen::default()
        .context("screen")?
        .root_window()
        .context("root")?
        .pixbuf(x, y, a.width(), a.height())
        .context("composed root capture")
}
fn screen_pixel(image: &gtk::gdk_pixbuf::Pixbuf, x: i32, y: i32) -> [u8; 3] {
    let bytes = image.read_pixel_bytes();
    let offset = (y * image.rowstride() + x * image.n_channels()) as usize;
    [bytes[offset], bytes[offset + 1], bytes[offset + 2]]
}
fn overlay_check(ui: &Ui, state: &str, surface: &str) -> Result<()> {
    let selector = if surface == "forest" {
        ".forest-pop"
    } else {
        ".palette-box"
    };
    let (x, y, w, h) = rect(ui, selector)?;
    anyhow::ensure!(
        x >= 0. && y >= 0. && w > 40. && h > 32.,
        "{state} {surface} must be open: {x},{y},{w},{h}"
    );
    let image = screen(ui)?;
    if let Ok(dir) = std::env::var("OVERLAY_SHOTS") {
        std::fs::create_dir_all(&dir)?;
        image.savev(format!("{dir}/{state}-{surface}.png"), "png", &[])?;
    }
    // The map legend reaches this edge; sample its padding rather than a glyph.
    let pop = screen_pixel(&image, (x + w - 20.) as i32, (y + h - 4.) as i32);
    let dim = screen_pixel(&image, 5, image.height() / 2);
    let chrome = ui.chrome.native().window().unwrap();
    for (i, window) in chrome.parent().unwrap().children().iter().enumerate() {
        println!(
            "STACK {state} {i} chrome={} native={} visible={}",
            *window == chrome,
            window.has_native(),
            window.is_visible()
        );
    }
    println!("PIXELS {state} {surface} pop={pop:?} dim={dim:?}");
    anyhow::ensure!(
        pop.iter().max().unwrap() - pop.iter().min().unwrap() < 30 && pop[1] < 100,
        "{state} {surface}: composed pop-in is covered: {pop:?}"
    );
    if state != "new-tree" {
        anyhow::ensure!(
            dim[1] > dim[0] + 10 && dim[1] > dim[2] + 10 && dim[1] < 160,
            "{state} {surface}: dimming must preserve the green page: {dim:?}"
        );
    } else {
        // HOME is neutral #0f0f0f; its scrim must transmit that surface, too.
        anyhow::ensure!(
            dim.iter().all(|c| *c > 0 && *c < 15),
            "new-tree {surface}: dimming must transmit HOME rather than opaque black: {dim:?}"
        );
    }
    Ok(())
}
fn overlays(u: &U, state: &str) -> Result<()> {
    for (keyname, surface) in [("ctrl+e", "forest"), ("ctrl+k", "palette")] {
        key(keyname)?;
        wait("overlay allocated", || {
            rect(
                &u.ui,
                if surface == "forest" {
                    ".forest-pop"
                } else {
                    ".palette-box"
                },
            )
            .is_ok_and(|(_, y, w, h)| y >= 0. && w > 40. && h > 40.)
        })?;
        pump(150);
        overlay_check(&u.ui, state, surface)?;
        key("Escape")?;
        pump(100);
    }
    println!("PASS overlay {state}");
    Ok(())
}
fn overlay_pixels() -> Result<()> {
    let u = U::new()?;
    let a = u.nav("/colour/b")?;
    pump(1200);
    overlays(&u, "first")?;
    let b = u.root("/colour/b-second")?;
    pump(1200);
    overlays(&u, "second")?;
    u.ui.pages.go(Go::Visit(a))?;
    anyhow::ensure!(u.arrives(a, 6000), "live return");
    overlays(&u, "live")?;
    u.ui.pages.inject_memory_sample(Some(u64::MAX));
    wait("checkpoint", || !u.ui.pages.live_visits().contains(&b))?;
    u.ui.pages.go(Go::Visit(b))?;
    anyhow::ensure!(u.arrives(b, 6000), "checkpoint return");
    pump(1200);
    overlays(&u, "revived")?;
    // A middle click loads a hidden branch before adoption.
    eval(
        &u.ui,
        "document.querySelector('#fork').href='/colour/b-background';JSON.stringify(true)",
    )?;
    let (x, y) = page_point(&u.ui, 12, 102)?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    pump(1200);
    key("ctrl+e")?;
    pump(250);
    let pending = chrome(&u.ui, "S.frame.waiting.ids[0]")?;
    key("Escape")?;
    if let Some(id) = pending.as_str() {
        u.ui.pages.go(Go::Visit(id.parse()?))?;
        pump(1200);
        overlays(&u, "background")?;
    } else {
        bail!("middle click must create a background page");
    }
    for (keyname, surface) in [("ctrl+e", "forest"), ("ctrl+k", "palette")] {
        key(keyname)?;
        pump(250);
        let sample = Rc::new(std::cell::RefCell::new(None));
        let recorded = sample.clone();
        let ui = u.ui.clone();
        u.ui.pages.observe_release(move |event| {
            if event.phase == canopy::pages::ReleasePhase::Holding {
                *recorded.borrow_mut() = Some(overlay_check(&ui, "holding", surface));
            }
        });
        u.ui.pages
            .open_roots(&[u.server.url("/colour/overlay-hold")])?;
        wait("overlay hold sampled", || sample.borrow().is_some())?;
        sample.borrow_mut().take().unwrap()?;
        pump(1200);
        if !visible(
            &u.ui,
            if surface == "forest" {
                "#forest"
            } else {
                "#palette"
            },
        ) {
            key(keyname)?;
        }
        pump(250);
        overlay_check(&u.ui, "after-hold", surface)?;
        key("Escape")?;
    }
    u.ui.window.resize(1000, 740);
    pump(400);
    overlays(&u, "resized")?;
    key("ctrl+w")?;
    pump(1200);
    overlays(&u, "closed")?;
    key("ctrl+t")?;
    pump(400);
    overlays(&u, "new-tree")?;
    Ok(())
}

fn forest_click_survives_frame() -> Result<()> {
    let u = U::new()?;
    let a = u.root("/a")?;
    let tree =
        u.ui.pages
            .read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
            .tree_of(a)
            .unwrap();
    u.ui.pages
        .place_tree(tree, canopy::browser::Move::History)?;
    wait("last tree settled", || u.cursor().is_none() && !u.loading())?;
    key("ctrl+e")?;
    wait("forest", || visible(&u.ui, "#forest"))?;
    click_chrome(&u.ui, "#forest-views [data-view=history]")?;
    let selector = format!(".forest-row[data-id='{a}']");
    wait("settled page visible", || visible(&u.ui, &selector))?;
    let (x, y) = chrome_point(&u.ui, &selector)?;
    focus_window()?;
    move_to(x, y)?;
    chrome(&u.ui,"(window.__pressed=false,window.addEventListener('mousedown',()=>window.__pressed=true,{once:true}),true)")?;
    xdo(&["mousedown", "1"])?;
    wait("chrome received mouse down", || {
        chrome(&u.ui, "window.__pressed").ok() == Some(json!(true))
    })?;
    chrome(&u.ui, "(canopy.present(S.frame),true)")?;
    xdo(&["mouseup", "1"])?;
    pump(100);
    key("Return")?;
    anyhow::ensure!(
        u.arrives(a, 6000),
        "unchanged forest frame must preserve a pressed page's click"
    );
    Ok(())
}

fn palette_anchor_stable() -> Result<()> {
    let u = U::new()?;
    u.root("/a")?;
    key("ctrl+k")?;
    wait("palette", || visible(&u.ui, "#palette"))?;
    chrome(
        &u.ui,
        "(window.__anchor=document.querySelector('#palette-context .fav'),true)",
    )?;
    for n in 0..4 {
        eval(
            &u.ui,
            &format!("document.title='changed {n}';JSON.stringify(true)"),
        )?;
        pump(200);
        assert_eq!(
            chrome(
                &u.ui,
                "window.__anchor === document.querySelector('#palette-context .fav')"
            )?,
            json!(true),
            "title update must preserve favicon element"
        );
    }
    Ok(())
}

fn first_address_survives_initial_home() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let server = Server::new()?;
    let ui = Ui::new(dir.path())?;
    ui.window.show_all();
    let home = ui.pages.selected_page().native.clone();
    server.gate("initial", true);
    ui.pages.open_roots(&[server.url("/gate/initial/first")])?;
    // A bootstrap notification already queued before stop_loading can arrive late.
    home.emit_by_name::<()>("load-changed", &[&webkit2gtk::LoadEvent::Started]);
    server.gate("initial", false);
    wait(
        "first address survives a late initial-home notification",
        || {
            ui.snapshot()
                .ok()
                .and_then(|s| s.current_visit().map(|v| v.url.path().to_string()))
                .as_deref()
                == Some("/gate/initial/first")
        },
    )?;
    assert!(server.hits("/gate/initial/first") > 0);
    ui.window.close();
    pump(50);
    Ok(())
}

#[derive(Default)]
struct HoldWatch {
    held: std::cell::Cell<Option<usize>>,
    started: std::cell::Cell<Option<Instant>>,
    ended: std::cell::Cell<Option<Instant>>,
    durations: std::cell::RefCell<Vec<u128>>,
}
fn watch_hold(u: &U) -> Rc<HoldWatch> {
    let state = Rc::new(HoldWatch::default());
    let record = state.clone();
    u.ui.pages.observe_release(move |event| {
        use canopy::pages::ReleasePhase::*;
        match event.phase {
            Holding => {
                assert!(record.held.replace(Some(event.widget)).is_none());
                record.started.set(Some(Instant::now()));
                record.ended.set(None);
            }
            Hidden => {
                assert_eq!(record.held.replace(None), Some(event.widget));
                let elapsed = record.started.get().unwrap().elapsed().as_millis();
                assert!(elapsed <= 1100, "hold cap exceeded: {elapsed} ms");
                record.durations.borrow_mut().push(elapsed);
                record.ended.set(Some(Instant::now()));
                println!("MEASURE hold_ms={elapsed}");
            }
            Checkpointing | Released => {}
        }
    });
    state
}
fn pixel(u: &U) -> Result<[u8; 3]> {
    let (_, x, y) = u.ui.window.window().context("window")?.origin();
    let image = gdk::Screen::default()
        .context("screen")?
        .root_window()
        .context("root")?
        .pixbuf(x + 600, y + 600, 1, 1)
        .context("pixel")?;
    let bytes = image.read_pixel_bytes();
    Ok([bytes[0], bytes[1], bytes[2]])
}
fn structure(u: &U, state: &HoldWatch) {
    let selected = u.ui.pages.selected_page().native;
    let parent = selected.parent().unwrap().downcast::<gtk::Fixed>().unwrap();
    for child in parent.children() {
        if let Ok(view) = child.downcast::<webkit2gtk::WebView>() {
            if view == u.ui.chrome.native() {
                continue;
            }
            assert!(
                !view.is_mapped()
                    || view == selected
                    || state.held.get() == Some(view.as_ptr() as usize),
                "only the selected and held outgoing page may be mapped"
            );
        }
    }
}
fn live_returns_paint_without_hold() -> Result<()> {
    picture_returns(false)
}
fn dormant_returns_paint_under_the_real_page_hold() -> Result<()> {
    picture_returns(true)
}
fn picture_returns(dormant: bool) -> Result<()> {
    let u = U::new()?;
    let roots = [u.root("/colour/a")?, u.root("/colour/b")?];
    pump(1200);
    let state = watch_hold(&u);
    let colours = [[208, 32, 64], [32, 176, 96]];
    for i in 0..24 {
        let old = colours[1 - i % 2];
        let target = colours[i % 2];
        if dormant {
            u.ui.pages.inject_memory_sample(Some(u64::MAX));
            wait("target checkpointed before revival", || {
                !u.ui.pages.live_visits().contains(&roots[i % 2])
            })?;
            pump(1200);
        }
        state.started.set(None);
        state.ended.set(None);
        u.ui.pages.go(Go::Visit(roots[i % 2]))?;
        let start = Instant::now();
        let mut blank_start = None;
        let mut presented_ms = 0;
        let mut actual_start = None;
        let mut actual_ms = 0u128;
        let mut grey_samples = 0usize;
        let mut white_samples = 0usize;
        let mut old_samples = 0usize;
        let mut painted = false;
        while start.elapsed() < Duration::from_millis(1400) || !painted {
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "switch must paint"
            );
            pump(3);
            structure(&u, &state);
            let colour = pixel(&u)?;
            if state.held.get().is_none() {
                if colour == [246, 245, 244] || colour == [255, 255, 255] {
                    if colour == [246, 245, 244] {
                        grey_samples += 1;
                    } else {
                        white_samples += 1;
                    }
                    actual_start.get_or_insert(Instant::now());
                } else {
                    if colour == old {
                        old_samples += 1;
                    }
                    if let Some(at) = actual_start.take() {
                        actual_ms += at.elapsed().as_millis();
                    }
                }
            }
            if state.held.get().is_some() {
                assert_eq!(colour, old, "the real outgoing page covers the entire hold");
            } else if colour != target {
                assert!(!painted, "incoming page must not revert");
                blank_start.get_or_insert(Instant::now());
            } else {
                if let Some(at) = blank_start.take() {
                    presented_ms += at.elapsed().as_millis();
                }
                painted = true;
            }
            assert!(
                colour == old
                    || colour == target
                    || colour == [246, 245, 244]
                    || colour == [255, 255, 255],
                "no third page colour: {colour:?}"
            );
        }
        println!("MEASURE switch={i} blank_ms={actual_ms} presented_ms={presented_ms} grey_samples={grey_samples} white_samples={white_samples} old_after_hold={old_samples}");
        assert!(
            actual_ms <= 50,
            "after-hold blank exceeded 50 ms: {actual_ms}"
        );
        assert!(painted);
        assert!(
            state.started.get().is_some() == dormant,
            "dormant revival uses a hold; live return shows directly"
        );
        assert!(
            dormant || presented_ms <= 100,
            "incoming colour was not presented within 100 ms: {presented_ms}"
        );
    }
    Ok(())
}
fn switch_while_picture_waits_for_script() -> Result<()> {
    let u = U::new()?;
    u.root("/colour/a")?;
    pump(1200);
    let outgoing = u.ui.pages.selected_page().native;
    let clicks = Rc::new(std::cell::Cell::new(0));
    let recorded = clicks.clone();
    outgoing.connect_button_press_event(move |_, _| {
        recorded.set(recorded.get() + 1);
        glib::Propagation::Proceed
    });
    let state = watch_hold(&u);
    u.ui.pages.open_roots(&[u.server.url("/colour/busy")])?;
    let start = Instant::now();
    let mut held = false;
    while start.elapsed() < Duration::from_secs(5) {
        pump(3);
        structure(&u, &state);
        if state.held.get().is_some() {
            held = true;
            assert_eq!(pixel(&u)?, [208, 32, 64]);
            if state.started.get().unwrap().elapsed() > Duration::from_millis(200)
                && clicks.get() == 0
            {
                move_to(600, 600)?;
                xdo(&["click", "1"])?;
            }
        }
    }
    assert!(held && state.ended.get().is_some());
    assert!(
        state.durations.borrow()[0] >= 950,
        "busy page holds until cap"
    );
    assert_eq!(clicks.get(), 0, "outgoing page never receives input");
    assert_eq!(pixel(&u)?, [32, 176, 96]);
    Ok(())
}
fn later_switch_releases_first_cover() -> Result<()> {
    let u = U::new()?;
    u.root("/colour/a")?;
    pump(1200);
    let state = watch_hold(&u);
    u.ui.pages
        .open_roots(&[u.server.url("/colour/short-busy")])?;
    wait("incoming hold", || state.held.get().is_some())?;
    let first_hold = state.started.get();
    u.ui.pages
        .open_roots(&[u.server.url("/colour/third-busy")])?;
    assert!(
        state.held.get().is_none(),
        "new switch releases the first outgoing page immediately"
    );
    wait("second commit", || state.started.get() != first_hold)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        pump(3);
        structure(&u, &state);
        assert_ne!(
            pixel(&u)?,
            [208, 32, 64],
            "first outgoing page cannot reappear"
        );
    }
    assert_eq!(pixel(&u)?, [32, 64, 208]);
    Ok(())
}
fn page_views_have_no_paint_handlers() -> Result<()> {
    let source = concat!(
        include_str!("../src/pages/mod.rs"),
        include_str!("../src/pages/live.rs"),
        include_str!("../src/pages/native.rs")
    );
    assert!(
        !source.contains("connect_draw")
            && !source.contains("\"draw\"")
            && !source.contains("create_from_window")
            && !source.contains(".draw("),
        "Canopy must never install page draw handlers or paint page windows"
    );
    live_returns_paint_without_hold()
}

fn page_focus_survives_inactive_switch() -> Result<()> {
    let u = U::new()?;
    u.root("/colour/a")?;
    pump(1200);
    let old = u.ui.pages.selected_page().native;
    focus_window()?;
    old.grab_focus();
    pump(50);
    let sink = gtk::Window::new(gtk::WindowType::Toplevel);
    sink.set_title("Focus sink");
    sink.show_all();
    pump(50);
    let output = std::process::Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "^Focus sink$"])
        .output()?;
    let id = String::from_utf8(output.stdout)?.trim().to_owned();
    xdo(&["windowfocus", &id])?;
    pump(100);
    assert!(old.is_focus());
    assert!(!old.has_focus());
    u.root("/colour/b")?;
    pump(1200);
    let selected = u.ui.pages.selected_page().native;
    let focused = selected.is_focus();
    assert!(
        !selected.has_focus(),
        "switch does not activate an inactive window"
    );
    sink.close();
    assert!(
        focused,
        "logical page focus must move even while the window is inactive"
    );
    key("ctrl+t")?;
    wait("new-tree focus after reactivating the window", || {
        chrome_focused(&u.ui, "#start-search")
    })?;
    Ok(())
}

fn layout_stays_in_slot() -> Result<()> {
    let u = U::new()?;
    let a = u.root("/colour/a")?;
    let child = u.nav("/colour/a-child")?;
    let b = u.root("/colour/b")?;
    pump(500);
    let bad = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = bad.clone();
    u.ui.pages.observe_page_placement(move |view| {
        if view.widget_name() == "watched" {
            return;
        }
        view.set_widget_name("watched");
        let seen = seen.clone();
        let expected = view.allocation();
        view.connect_size_allocate(move |_, rect| {
            if *rect != expected {
                seen.borrow_mut().push(*rect);
            }
        });
    });
    for i in 0..10 {
        let (left, _, width, _) = rect(&u.ui, "#strip-clip")?;
        let (x, y) = page_point(&u.ui, (left + width - 24.) as i32, 68)?;
        move_to(x, y)?;
        pump(300);
        move_to(600, 600)?;
        pump(500);
        u.ui.chrome
            .toast(canopy::chrome::Toast::Info("layout check".into()));
        pump(2600);
        if i % 3 == 0 {
            click_chrome(&u.ui, &u.tab(if i % 2 == 0 { a } else { b }))?;
        } else if i % 3 == 1 {
            key(if i % 2 == 0 { "ctrl+1" } else { "ctrl+2" })?;
        } else {
            u.ui.pages.go(Go::Visit(child))?;
            wait("child revived", || {
                u.cursor() == Some(child) && !u.loading()
            })?;
            u.ui.pages.go(Go::Parent)?;
            wait("Back reaches revived parent", || {
                u.cursor() == Some(a) && !u.loading()
            })?;
        }
        pump(700);
        assert_eq!(
            eval(&u.ui, "JSON.stringify(resizes)")?,
            json!([]),
            "page must not receive a resize during a switch or cover change"
        );
    }
    u.root("/colour/c")?;
    pump(500);
    key("ctrl+w")?;
    pump(1000);
    assert_eq!(eval(&u.ui, "JSON.stringify(resizes)")?, json!([]));
    assert!(
        bad.borrow().is_empty(),
        "every native allocation must equal the slot: {:?}",
        bad.borrow()
    );
    u.ui.window.resize(700, 500);
    pump(500);
    wait("page receives only the window resize", || {
        eval(&u.ui, "JSON.stringify(resizes)").ok() == Some(json!([[700, 406]]))
    })?;
    let scroller =
        u.ui.window
            .child()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
    assert_eq!(scroller.hadjustment().value(), 0.);
    assert_eq!(scroller.vadjustment().value(), 0.);
    assert!(!scroller.hscrollbar().is_some_and(|w| w.is_mapped()));
    assert!(!scroller.vscrollbar().is_some_and(|w| w.is_mapped()));
    geometry(&u)?;
    Ok(())
}

/// Opens a new tree, then 300 ms after it shows, clicks the first tree's tab, ten times.
fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
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
    let cases: &[Case] = &[
        ("overlay", overlay_pixels),
        ("first_address", first_address_survives_initial_home),
        ("inactive_focus", page_focus_survives_inactive_switch),
        ("picture", live_returns_paint_without_hold),
        (
            "dormant_picture",
            dormant_returns_paint_under_the_real_page_hold,
        ),
        ("structure", page_views_have_no_paint_handlers),
        ("busy_picture", switch_while_picture_waits_for_script),
        ("later_picture", later_switch_releases_first_cover),
        ("slot", layout_stays_in_slot),
        ("mouse", ui_mouse_back),
        ("allocation", ui_tab_collapse),
        ("resize", resize_and_show),
        ("anchor", palette_anchor_stable),
        ("stacking", chrome_after_changes),
        ("forest_press", forest_click_survives_frame),
    ];
    for (name, run) in cases {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        match run() {
            Ok(()) => println!("DONE {name}"),
            Err(error) => return Err(error.context(format!("branch UI {name}"))),
        }
    }
    Ok(())
}
