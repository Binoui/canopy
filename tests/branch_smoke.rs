#![allow(dead_code, unused_imports)]
use anyhow::{bail, Result};
mod common;
#[path = "common/branch_regressions.rs"]
mod regressions;
use canopy::{
    browser::{Browser, NavigationTarget, VisitId},
    pages::{Go, PageEvent, Pages},
};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use webkit2gtk::{BackForwardListExt, BackForwardListItemExt, UserContentManagerExt, WebViewExt};

fn pump() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}
fn wait(label: &str, condition: impl Fn() -> bool) -> Result<()> {
    // Destroyed WebKit processes can take 20s to exit under CPU load.
    let deadline =
        Instant::now() + Duration::from_secs(if label.contains("process") { 60 } else { 15 });
    loop {
        pump();
        if condition() {
            return Ok(());
        }
        if Instant::now() > deadline {
            bail!("Timed out waiting for {label}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}
fn settle() {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        pump();
        thread::sleep(Duration::from_millis(5));
    }
}

struct Server {
    origin: String,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
    held: Arc<AtomicBool>,
    refused: Arc<AtomicBool>,
}
impl Server {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(BTreeMap::new()));
        let held = Arc::new(AtomicBool::new(false));
        let refused = Arc::new(AtomicBool::new(false));
        let count = hits.clone();
        let hold = held.clone();
        let refuse = refused.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let count = count.clone();
                let hold = hold.clone();
                let refuse = refuse.clone();
                thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut request = [0; 8192];
                    let Ok(n) = stream.read(&mut request) else {
                        return;
                    };
                    let text = String::from_utf8_lossy(&request[..n]);
                    let method = text.split_whitespace().next().unwrap_or("GET");
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                    *count
                        .lock()
                        .unwrap()
                        .entry(format!("{method} {path}"))
                        .or_default() += 1;
                    *count.lock().unwrap().entry(path.clone()).or_default() += 1;
                    if refuse.load(Ordering::Relaxed) {
                        return;
                    }
                    if path == "/file" {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=held.bin\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n");
                        let _ = stream.write_all(&vec![42; 65536]);
                        let _ = stream.flush();
                        let deadline = Instant::now() + Duration::from_secs(20);
                        while hold.load(Ordering::Relaxed) && Instant::now() < deadline {
                            thread::sleep(Duration::from_millis(5));
                        }
                        let _ = stream.write_all(&vec![42; 65536]);
                        return;
                    }
                    if path == "/slow" {
                        let deadline = Instant::now() + Duration::from_secs(20);
                        while hold.load(Ordering::Relaxed) && Instant::now() < deadline {
                            thread::sleep(Duration::from_millis(5));
                        }
                    }
                    let body = format!("<!doctype html><title>{path}</title><body style='margin:0;height:10000px'><input id='form'><a id='fork' href='/d'>fork</a><div id='fragment' style='margin-top:5000px'>end</div></body>");
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                });
            }
        });
        Ok(Self {
            origin,
            hits,
            held,
            refused,
        })
    }
    fn url(&self, path: &str) -> url::Url {
        format!("{}{path}", self.origin).parse().unwrap()
    }
    fn hits(&self, path: &str) -> usize {
        *self.hits.lock().unwrap().get(path).unwrap_or(&0)
    }
}
struct Fixture {
    pages: Rc<Pages>,
    fixed: gtk::Fixed,
    window: gtk::Window,
    server: Server,
    _directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let fixed = gtk::Fixed::new();
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(1000, 800);
        window.add(&fixed);
        let pages = Pages::mount(
            &fixed,
            Browser::open(&directory.path().join("forest.sqlite3"))?,
            &directory.path().join("profile"),
        )?;
        window.show_all();
        pages.resize(1000., 800.);
        wait("initial page", || {
            !pages.selected_page().native.is_loading()
        })?;
        Ok(Self {
            pages,
            fixed,
            window,
            server: Server::new()?,
            _directory: directory,
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
    fn arrived(&self, id: VisitId) -> Result<()> {
        wait("visit committed", || {
            self.pages
                .read(|b| b.snapshot())
                .is_ok_and(|s| s.current == Some(id))
                && !self.pages.selected_page().native.is_loading()
        })?;
        self.pages.resize(1000., 800.);
        Ok(())
    }
    fn navigate(&self, path: &str) -> Result<VisitId> {
        let parent = self.pages.read(|b| b.snapshot())?.current;
        let target = parent
            .map(NavigationTarget::ChildOf)
            .unwrap_or(NavigationTarget::Root);
        let before = self.pages.read(|b| b.snapshot())?.visits.len();
        self.pages.go(Go::Address {
            url: self.server.url(path),
            target,
        })?;
        wait("new visit", || {
            self.pages
                .read(|b| b.snapshot())
                .is_ok_and(|s| s.visits.len() == before + 1)
                && !self.pages.selected_page().native.is_loading()
        })?;
        Ok(self.pages.read(|b| b.snapshot())?.current.unwrap())
    }
    fn chain(&self) -> Result<[VisitId; 3]> {
        let a = self.navigate("/a")?;
        let b = self.navigate("/b")?;
        let c = self.navigate("/c")?;
        self.pages.go(Go::Visit(b))?;
        self.arrived(b)?;
        Ok([a, b, c])
    }
    fn release(&self) -> Result<()> {
        let weak = self.pages.selected_page().native.downgrade();
        self.pages.release_selected_page();
        wait("view finalized", || weak.upgrade().is_none())?;
        assert!(
            self.pages.checkpoint_size().is_some(),
            "checkpoint must succeed"
        );
        println!(
            "MEASURE checkpoint_bytes={}",
            self.pages.checkpoint_size().unwrap()
        );
        Ok(())
    }
    fn revive(&self, visit: VisitId) -> Result<bool> {
        let start = Instant::now();
        let cold = self.pages.revive_selected_page(&self.fixed, visit)?;
        self.arrived(visit)?;
        println!(
            "MEASURE revival_to_finished_ms={}",
            start.elapsed().as_millis()
        );
        Ok(cold)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.held.store(false, Ordering::Relaxed);
        self.window.close();
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

fn branch_history() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    wait("one source web process", || web_process_count() == 1)?;
    let old = f.pages.selected_page().native.downgrade();
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("root", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url == f.server.url("/root"))
    })?;
    wait("previous view destroyed", || old.upgrade().is_none())?;
    wait("previous web process exited", || web_process_count() == 1)?;
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    assert!(
        f.pages.selected_history_visits().contains(&a),
        "branch back list must survive switching"
    );
    assert_eq!(f.server.hits("/b"), 2);
    Ok(())
}

fn queued(f: &Fixture, path: &str) -> Result<VisitId> {
    f.eval(&format!(
        "document.getElementById('fork').href = '{}'; scrollTo(0,0); return true;",
        path
    ))?;
    settle();
    let point = f.eval("const r=document.getElementById('fork').getBoundingClientRect(); return [r.x+r.width/2,r.y+r.height/2];")?;
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
            "2",
        ])
        .status()?;
    assert!(status.success());
    wait("queued visit", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .visits
            .iter()
            .any(|v| v.opened_at.is_none() && v.url == f.server.url(path))
    })?;
    Ok(f.pages
        .read(|b| b.snapshot())?
        .visits
        .iter()
        .find(|v| v.opened_at.is_none() && v.url == f.server.url(path))
        .unwrap()
        .id)
}
fn owner_fork_queue_scroll() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let search = f.navigate("/search")?;
    let r1 = f.navigate("/r1")?;
    f.pages.go(Go::Parent)?;
    f.arrived(search)?;
    let r2 = f.navigate("/r2")?;
    f.pages.go(Go::Parent)?;
    f.arrived(search)?;
    let q = queued(&f, "/d")?;
    assert_eq!(f.server.hits("/d"), 0);
    f.eval("scrollTo(0,1600); return true;")?;
    settle();
    f.pages.go(Go::Visit(q))?;
    f.arrived(q)?;
    assert_eq!(f.server.hits("/d"), 1);
    assert_eq!(
        f.pages
            .selected_page()
            .native
            .back_forward_list()
            .unwrap()
            .length(),
        1
    );
    f.pages.go(Go::Parent)?;
    f.arrived(search)?;
    wait("scroll restored", || {
        f.eval("return scrollY;").unwrap() == json!(1600)
    })?;
    assert!(f.pages.selected_history_visits().contains(&a));
    assert!(f.pages.selected_history_visits().contains(&r2));
    assert!(!f.pages.selected_history_visits().contains(&r1));
    f.pages.go(Go::Visit(r1))?;
    f.arrived(r1)?;
    assert_eq!(
        f.pages
            .selected_page()
            .native
            .back_forward_list()
            .unwrap()
            .length(),
        1
    );
    assert_eq!(f.server.hits("/r1"), 2);
    Ok(())
}
fn middle_history() -> Result<()> {
    let f = Fixture::new()?;
    let [a, b, c] = f.chain()?;
    f.pages.go(Go::Visit(c))?;
    f.arrived(c)?;
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("root", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            == f.server.url("/root")
    })?;
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    let list = f.pages.selected_page().native.back_forward_list().unwrap();
    assert_eq!(list.back_list().len(), 1);
    assert_eq!(list.forward_list().len(), 1);
    assert!(f.pages.selected_history_visits().contains(&a));
    assert!(f.pages.selected_history_visits().contains(&c));
    Ok(())
}
fn replacement(same: bool) -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let q = queued(&f, "/slow")?;
    f.server.held.store(true, Ordering::Relaxed);
    f.eval("document.body.style.background='rgb(210,30,50)'; return true;")?;
    settle();
    let painted = pixel(&f);
    assert_eq!(painted, [210, 30, 50]);
    let old = f.pages.selected_page().native.clone();
    f.pages.go(Go::Visit(q))?;
    wait("candidate request", || f.server.hits("/slow") > 0)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(a));
    assert_eq!(f.pages.selected_page().native, old);
    assert!(old.is_visible());
    assert_eq!(
        pixel(&f),
        painted,
        "old page must remain painted while candidate loads"
    );
    if same {
        f.pages.go(Go::Visit(q))?;
        settle();
        assert_eq!(
            f.server.hits("/slow"),
            1,
            "double activation must reuse pending target"
        );
        f.server.held.store(false, Ordering::Relaxed);
        f.arrived(q)?;
        assert_eq!(
            f.pages
                .selected_page()
                .native
                .back_forward_list()
                .unwrap()
                .length(),
            1
        );
    } else {
        f.pages.open_roots(&[f.server.url("/root")])?;
        wait("replacement", || {
            f.pages
                .read(|b| b.snapshot())
                .unwrap()
                .current_visit()
                .unwrap()
                .url
                == f.server.url("/root")
        })?;
        let root = f.pages.read(|b| b.snapshot())?.current;
        f.server.held.store(false, Ordering::Relaxed);
        settle();
        assert_eq!(f.pages.read(|b| b.snapshot())?.current, root);
        assert!(f.pages.read(|b| b.visit(q))?.opened_at.is_none());
    }
    drop(old);
    wait("one web process after activation", || {
        web_process_count() == 1
    })?;
    Ok(())
}
fn failed_switch() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let q = queued(&f, "/d")?;
    let old = f.pages.selected_page().native.clone();
    let errors = Rc::new(RefCell::new(0));
    let count = errors.clone();
    f.pages.observe(move |event| {
        if matches!(event, PageEvent::Error(_)) {
            *count.borrow_mut() += 1;
        }
    });
    f.server.refused.store(true, Ordering::Relaxed);
    f.pages.go(Go::Visit(q))?;
    wait("failed switch error", || *errors.borrow() > 0)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(a));
    assert_eq!(f.pages.selected_page().native, old);
    assert!(f.pages.read(|b| b.visit(q))?.opened_at.is_none());
    f.server.refused.store(false, Ordering::Relaxed);
    f.pages.go(Go::Visit(q))?;
    f.arrived(q)?;
    assert_eq!(
        f.pages
            .selected_page()
            .native
            .back_forward_list()
            .unwrap()
            .length(),
        1
    );
    Ok(())
}
fn loading_source() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.server.held.store(true, Ordering::Relaxed);
    f.pages.go(Go::Address {
        url: f.server.url("/slow"),
        target: NavigationTarget::ChildOf(a),
    })?;
    wait("old load", || f.server.hits("/slow") > 0)?;
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("root", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            == f.server.url("/root")
    })?;
    f.server.held.store(false, Ordering::Relaxed);
    settle();
    assert!(!f
        .pages
        .read(|b| b.snapshot())?
        .visits
        .iter()
        .any(|v| v.url == f.server.url("/slow")));
    f.pages.go(Go::Visit(a))?;
    f.arrived(a)?;
    Ok(())
}

fn pixel(f: &Fixture) -> [u8; 3] {
    let pix = f.window.window().unwrap().pixbuf(30, 300, 1, 1).unwrap();
    let bytes = pix.read_pixel_bytes();
    [bytes[0], bytes[1], bytes[2]]
}
fn cold_fallback(invalid: bool) -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    if !invalid {
        f.pages.forget_history_visit(a);
    }
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("root", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            == f.server.url("/root")
    })?;
    settle();
    if invalid {
        rusqlite::Connection::open(f._directory.path().join("forest.sqlite3"))?.execute(
            "UPDATE visits SET url=?1 WHERE id=?2",
            [f.server.url("/changed").as_str(), &a.to_string()],
        )?;
    }
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    assert_eq!(
        f.pages
            .selected_page()
            .native
            .back_forward_list()
            .unwrap()
            .length(),
        1,
        "bad checkpoint must fall back cold"
    );
    Ok(())
}
fn settled_dormant() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    let tree = f
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(a)
        .unwrap();
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("root", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            == f.server.url("/root")
    })?;
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    assert!(
        f.pages.selected_history_visits().contains(&a),
        "checkpoint retained before settle"
    );
    f.pages.open_roots(&[f.server.url("/root2")])?;
    wait("root2", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            == f.server.url("/root2")
    })?;
    f.pages.settle_tree(tree)?;
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    assert_eq!(
        f.pages
            .selected_page()
            .native
            .back_forward_list()
            .unwrap()
            .length(),
        1,
        "settled tree loses its checkpoints"
    );
    Ok(())
}
fn services_after_switches() -> Result<()> {
    use canopy::{
        cookies::{Cookie, CookieDomain, Inbox, SameSite},
        pages::DownloadEvent,
        secret::Secret,
    };
    let f = Fixture::new()?;
    f.navigate("/a")?;
    for path in ["/one", "/two", "/three"] {
        let old = f.pages.selected_page().native.downgrade();
        f.pages.open_roots(&[f.server.url(path)])?;
        wait("root", || {
            f.pages
                .read(|b| b.snapshot())
                .unwrap()
                .current_visit()
                .unwrap()
                .url
                == f.server.url(path)
        })?;
        wait("source released", || old.upgrade().is_none())?;
    }
    let inbox = Inbox::new(f._directory.path());
    inbox.post(&[Cookie {
        domain: CookieDomain::Host("127.0.0.1".into()),
        path: "/".into(),
        name: "branch_cookie".into(),
        value: Secret::new("yes".into()),
        expires: None,
        secure: false,
        http_only: false,
        same_site: SameSite::Lax,
    }])?;
    let applied = Rc::new(RefCell::new(None));
    let result = applied.clone();
    f.pages
        .apply_cookie_batches(inbox.collect()?, move |n| *result.borrow_mut() = Some(n));
    wait("cookie batch", || applied.borrow().is_some())?;
    assert_eq!(applied.borrow_mut().take().unwrap().unwrap(), 1);
    assert!(f
        .eval("return document.cookie;")?
        .as_str()
        .unwrap()
        .contains("branch_cookie=yes"));
    let source = f.pages.read(|b| b.snapshot())?.current.unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    f.pages.observe(move |event| {
        if let PageEvent::Download(event) = event {
            observed.borrow_mut().push(event)
        }
    });
    f.server.held.store(true, Ordering::Relaxed);
    f.pages
        .selected_page()
        .native
        .download_uri(f.server.url("/file").as_str());
    wait("download started", || !events.borrow().is_empty())?;
    let old = f.pages.selected_page().native.downgrade();
    f.pages.open_roots(&[f.server.url("/four")])?;
    wait("download source released", || old.upgrade().is_none())?;
    f.server.held.store(false, Ordering::Relaxed);
    wait("download saved", || {
        events
            .borrow()
            .iter()
            .any(|e| matches!(e, DownloadEvent::Saved(_)))
    })?;
    let downloads = f.pages.read(|b| b.downloads())?;
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].visit, source);
    assert_eq!(std::fs::read(&downloads[0].path)?, vec![42; 131072]);
    Ok(())
}
fn chrome_after_switch() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let server = Server::new()?;
    let ui = canopy::ui::Ui::new(directory.path())?;
    ui.window.show_all();
    common::ready(&ui)?;
    for path in ["/a", "/b", "/c"] {
        ui.pages.open_roots(&[server.url(path)])?;
        wait("root", || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.url == server.url(path))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        common::pump(200);
        common::click_chrome(&ui, "#settings-button")?;
        wait("settings pointer", || {
            common::visible(&ui, "#popover #theme-mode")
        })?;
        common::click_chrome(&ui, "#theme-mode [data-mode='light']")?;
        wait("settings receives pointer", || {
            ui.pages
                .read(|b| b.theme_mode())
                .is_ok_and(|m| m == canopy::browser::ThemeMode::Light)
        })?;
        common::key("Escape")?;
        common::key("ctrl+l")?;
        wait("palette keyboard", || {
            common::chrome_focused(&ui, "#palette-input")
        })?;
        common::key("Escape")?;
    }
    server.held.store(true, Ordering::Relaxed);
    ui.pages.open_roots(&[server.url("/slow")])?;
    wait("hidden candidate", || server.hits("/slow") > 0)?;
    common::key("ctrl+l")?;
    wait("editing palette", || {
        common::chrome_focused(&ui, "#palette-input")
    })?;
    server.held.store(false, Ordering::Relaxed);
    wait("candidate shown", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == "/slow")
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(
        common::chrome_focused(&ui, "#palette-input"),
        "switch must preserve an active chrome editor"
    );
    ui.window.close();
    Ok(())
}
fn overlapping_ancestors() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let parent = f.navigate("/search")?;
    let q1 = queued(&f, "/d")?;
    f.pages.go(Go::Visit(q1))?;
    f.arrived(q1)?;
    f.pages.go(Go::Parent)?;
    f.arrived(parent)?;
    let q2 = queued(&f, "/d")?;
    assert_ne!(q1, q2);
    f.pages.go(Go::Visit(q2))?;
    f.arrived(q2)?;
    assert_eq!(f.pages.selected_history_visits(), vec![q2]);
    f.pages.go(Go::Parent)?;
    f.arrived(parent)?;
    assert!(f.pages.selected_history_visits().contains(&a));
    assert!(!f.pages.selected_history_visits().contains(&q1));
    assert!(!f.pages.selected_history_visits().contains(&q2));
    f.pages.go(Go::Visit(q1))?;
    f.arrived(q1)?;
    assert_eq!(f.pages.selected_history_visits(), vec![q1]);
    assert_eq!(f.server.hits("/d"), 3);
    drop(f);

    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("root", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            .path()
            == "/root"
    })?;
    settle();
    // A detached cold revival seeds a second native history for the same ancestor.
    f.release()?;
    assert!(f.revive(a)?);
    let x = f.navigate("/x")?;
    f.pages.open_roots(&[f.server.url("/other")])?;
    wait("other", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            .path()
            == "/other"
    })?;
    let errors = Rc::new(std::cell::Cell::new(0));
    let observed = errors.clone();
    f.pages.observe(move |event| {
        if matches!(event, PageEvent::Error(_)) {
            observed.set(observed.get() + 1)
        }
    });
    f.server.refused.store(true, Ordering::Relaxed);
    f.pages.go(Go::Visit(b))?;
    wait("cancelled overlap revival", || errors.get() > 0)?;
    f.server.refused.store(false, Ordering::Relaxed);
    f.pages.go(Go::Visit(a))?;
    f.arrived(a)?;
    assert!(
        f.pages.selected_history_visits().contains(&x),
        "MRU overlapping history wins"
    );
    assert!(!f.pages.selected_history_visits().contains(&b));
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    assert!(f.pages.selected_history_visits().contains(&a));
    f.pages.open_roots(&[f.server.url("/last")])?;
    wait("last", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            .path()
            == "/last"
    })?;
    f.pages.go(Go::Visit(a))?;
    f.arrived(a)?;
    assert!(
        f.pages.selected_history_visits().contains(&b),
        "selection changes overlapping histories' MRU order"
    );
    assert!(!f.pages.selected_history_visits().contains(&x));
    Ok(())
}
fn release_commands() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    let weak = Rc::downgrade(&f.pages);
    let once = Rc::new(std::cell::Cell::new(false));
    let called = once.clone();
    f.pages.observe(move |event| {
        let pages = weak.upgrade().unwrap();
        let root = pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == "/root");
        if matches!(event, PageEvent::Changed) && root && !called.replace(true) {
            let base = pages
                .read(|b| b.snapshot())
                .unwrap()
                .current_visit()
                .unwrap()
                .url
                .clone();
            pages
                .open_path(base.join("/first").unwrap(), base.join("/second").unwrap())
                .unwrap();
        }
    });
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("queued path", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == "/second")
    })?;
    assert!(once.get());
    assert_eq!(f.server.hits("/first"), 1);
    assert_eq!(f.server.hits("/second"), 1);
    Ok(())
}
fn native_alias_back() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("history.pushState({},'', '/route'); return true;")?;
    settle();
    f.pages.go(Go::Parent)?;
    wait("actual alias Back", || {
        f.pages.selected_page().native.uri().as_deref() == Some(f.server.url("/a").as_str())
    })?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(a));
    assert_eq!(f.server.hits("/a"), 1);
    let child = f.navigate("/b")?;
    f.eval("history.pushState({},'', '/route2'); return true;")?;
    settle();
    f.pages.go(Go::Parent)?;
    f.arrived(a)?;
    assert_ne!(f.pages.read(|b| b.snapshot())?.current, Some(child));
    Ok(())
}
fn failed_revival_retry() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
    f.pages.open_roots(&[f.server.url("/root")])?;
    wait("root", || {
        f.pages
            .read(|b| b.snapshot())
            .unwrap()
            .current_visit()
            .unwrap()
            .url
            .path()
            == "/root"
    })?;
    settle();
    let root = f.pages.read(|b| b.snapshot())?.current;
    let errors = Rc::new(std::cell::Cell::new(0));
    let observed = errors.clone();
    f.pages.observe(move |event| {
        if matches!(event, PageEvent::Error(_)) {
            observed.set(observed.get() + 1)
        }
    });
    f.server.refused.store(true, Ordering::Relaxed);
    f.pages.go(Go::Visit(b))?;
    wait("revival error", || errors.get() > 0)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, root);
    f.server.refused.store(false, Ordering::Relaxed);
    f.pages.go(Go::Visit(b))?;
    f.arrived(b)?;
    assert!(
        f.pages.selected_history_visits().contains(&a),
        "failed revival retains its checkpoint for retry"
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
    let only = std::env::args().nth(1);
    type Case = (&'static str, fn() -> Result<()>);
    let cases: &[Case] = &[
        ("branch_history", branch_history),
        ("native_alias_back", native_alias_back),
        ("failed_revival_retry", failed_revival_retry),
        ("release_commands", release_commands),
        ("overlapping_ancestors", overlapping_ancestors),
        ("owner_fork_queue_scroll", owner_fork_queue_scroll),
        ("middle_history", middle_history),
        ("double_activation", || replacement(true)),
        ("replacement", || replacement(false)),
        ("failed_switch", failed_switch),
        ("loading_source", loading_source),
        ("failed_checkpoint", || cold_fallback(false)),
        ("invalid_checkpoint", || cold_fallback(true)),
        ("settled_dormant", settled_dormant),
        ("services_after_switches", services_after_switches),
        ("chrome_after_switch", chrome_after_switch),
    ];
    let all: Vec<_> = regressions::cases()
        .into_iter()
        .chain(cases.iter().copied())
        .collect();
    for (name, run) in all {
        if only.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        run()?;
        println!("PASS {name}");
        settle();
    }
    Ok(())
}
