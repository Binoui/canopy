#![allow(dead_code, unused_imports)]
use anyhow::{bail, Result};
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
    let deadline = Instant::now() + Duration::from_secs(15);
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

fn branch_history() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let b = f.navigate("/b")?;
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
    let old = f.pages.selected_page().native.clone();
    f.pages.go(Go::Visit(q))?;
    wait("candidate request", || f.server.hits("/slow") > 0)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(a));
    assert_eq!(f.pages.selected_page().native, old);
    assert!(old.is_visible());
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
fn main() -> Result<()> {
    canopy::init_desktop()?;
    let only = std::env::args().nth(1);
    type Case = (&'static str, fn() -> Result<()>);
    let cases: &[Case] = &[
        ("branch_history", branch_history),
        ("owner_fork_queue_scroll", owner_fork_queue_scroll),
        ("middle_history", middle_history),
        ("double_activation", || replacement(true)),
        ("replacement", || replacement(false)),
        ("failed_switch", failed_switch),
        ("loading_source", loading_source),
    ];
    for (name, run) in cases {
        if only.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        run()?;
        println!("PASS {name}");
        settle();
    }
    Ok(())
}
