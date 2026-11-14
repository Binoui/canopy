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
use webkit2gtk::{BackForwardListExt, WebViewExt};

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
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
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
fn history_release() -> Result<()> {
    let f = Fixture::new()?;
    let [a, b, c] = f.chain()?;
    f.eval("document.getElementById('form').value='lost'; window.marker='lost'; scrollTo(0,1800); return true;")?;
    settle();
    wait("source scroll saved", || {
        f.pages
            .read(|browser| browser.scroll(b))
            .is_ok_and(|y| y == Some(1800))
            && f.eval("return scrollY;").is_ok_and(|y| y == json!(1800))
    })?;
    let before = f.server.hits("/b");
    let live = web_process_count();
    assert!(live >= 1);
    f.release()?;
    let released = Instant::now();
    wait("source web process exits", || web_process_count() < live)?;
    println!(
        "MEASURE release_process_exit_ms={} process_count_before={} after={}",
        released.elapsed().as_millis(),
        live,
        web_process_count()
    );
    let hits = f.server.hits.clone();
    let pages = Rc::downgrade(&f.pages);
    let observed = Rc::new(std::cell::Cell::new(false));
    let saw = observed.clone();
    f.pages.before_revival_traversal(move || {
        let pages = pages.upgrade().unwrap();
        assert_eq!(*hits.lock().unwrap().get("/b").unwrap(), before);
        assert!(!pages.selected_page().native.is_loading());
        assert_eq!(
            pages
                .selected_page()
                .native
                .back_forward_list()
                .unwrap()
                .length(),
            3
        );
        saw.set(true);
    });
    assert!(!f.revive(b)?);
    assert!(observed.get());
    assert_eq!(f.server.hits("/b"), before + 1);
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 3);
    wait("saved scroll", || {
        f.eval("return scrollY;").is_ok_and(|v| v == json!(1800))
    })?;
    assert_eq!(
        f.eval("return [document.getElementById('form').value, window.marker ?? null];")?,
        json!(["", null])
    );
    f.pages.selected_page().native.go_back();
    f.arrived(a)?;
    f.pages.selected_page().native.go_forward();
    f.arrived(b)?;
    f.pages.selected_page().native.go_forward();
    f.arrived(c)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 3);
    Ok(())
}
fn other_target() -> Result<()> {
    let f = Fixture::new()?;
    let [a, _, c] = f.chain()?;
    f.release()?;
    assert!(!f.revive(a)?);
    f.release()?;
    assert!(!f.revive(c)?);
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 3);
    Ok(())
}
fn fork() -> Result<()> {
    fork_case(false)
}
fn fork_without_binding() -> Result<()> {
    fork_case(true)
}
fn fork_case(unwrapped: bool) -> Result<()> {
    let f = Fixture::new()?;
    let [_, b, c] = f.chain()?;
    f.release()?;
    assert!(!f.revive(b)?);
    if unwrapped {
        f.pages.forget_history_visit(c);
    }
    f.eval("document.getElementById('fork').click(); return true;")?;
    wait("fork", || {
        f.pages.read(|b| b.snapshot()).unwrap().visits.len() == 4
            && !f.pages.selected_page().native.is_loading()
    })?;
    let list = f.pages.selected_page().native.back_forward_list().unwrap();
    assert!(list.forward_item().is_none());
    assert!(!f.pages.selected_history_visits().contains(&c));
    let before = f.server.hits("/c");
    f.pages.go(Go::Visit(c))?;
    f.arrived(c)?;
    assert_eq!(f.server.hits("/c"), before + 1);
    Ok(())
}
fn corrupt(kind: &str) -> Result<()> {
    let f = Fixture::new()?;
    let [_, b, _] = f.chain()?;
    f.eval("scrollTo(0,1700); return true;")?;
    settle();
    f.release()?;
    f.pages.corrupt_checkpoint(kind);
    assert!(f.revive(b)?);
    wait("cold saved scroll", || {
        f.eval("return scrollY;").is_ok_and(|v| v == json!(1700))
    })?;
    let list = f.pages.selected_page().native.back_forward_list().unwrap();
    assert_eq!(list.length(), 1);
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 3);
    Ok(())
}
fn absent() -> Result<()> {
    let f = Fixture::new()?;
    let _ = f.chain()?;
    let id = f.pages.read(|b| b.snapshot())?.visits[0].id;
    f.release()?;
    assert!(!f.revive(id)?);
    let d = f.navigate("/d")?;
    f.pages.go(Go::Visit(id))?;
    f.arrived(id)?;
    f.navigate("/e")?;
    f.release()?;
    assert!(f.revive(d)?);
    Ok(())
}
fn same_document() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("history.pushState({}, '', '#one'); location.hash='two'; return true;")?;
    settle();
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 1);
    f.pages.go(Go::Address {
        url: f.server.url("/a#fragment"),
        target: NavigationTarget::ChildOf(a),
    })?;
    wait("explicit fragment", || {
        f.pages.read(|b| b.snapshot()).unwrap().visits.len() == 2
    })?;
    let fragment = f.pages.read(|b| b.snapshot())?.current.unwrap();
    settle();
    f.release()?;
    assert!(!f.revive(fragment)?);
    f.pages.selected_page().native.go_back();
    f.arrived(a)?;
    f.pages.selected_page().native.go_forward();
    f.arrived(fragment)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 2);
    Ok(())
}
fn in_flight() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.server.held.store(true, Ordering::Relaxed);
    f.pages
        .open_path(f.server.url("/slow"), f.server.url("/never"))?;
    wait("slow requested", || f.server.hits("/slow") > 0)?;
    assert!(f.pages.has_pending_continuation());
    f.release()?;
    assert!(!f.pages.has_pending_continuation());
    f.server.held.store(false, Ordering::Relaxed);
    assert!(!f.revive(a)?);
    settle();
    assert_eq!(f.server.hits("/never"), 0);
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 1);
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(a));
    Ok(())
}
fn failure_retry() -> Result<()> {
    let f = Fixture::new()?;
    let [a, b, _] = f.chain()?;
    f.release()?;
    let errors = Rc::new(RefCell::new(0));
    let received = errors.clone();
    f.pages.observe(move |event| {
        if matches!(event, PageEvent::Error(_)) {
            *received.borrow_mut() += 1;
        }
    });
    f.server.refused.store(true, Ordering::Relaxed);
    assert!(!f.pages.revive_selected_page(&f.fixed, a)?);
    wait("revival error", || *errors.borrow() > 0)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(b));
    f.server.refused.store(false, Ordering::Relaxed);
    f.pages.go(Go::Visit(a))?;
    f.arrived(a)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 3);
    Ok(())
}
fn push_state_route() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("history.pushState({}, '', '/route'); return true;")?;
    settle();
    let b = f.navigate("/b")?;
    f.release()?;
    assert!(!f.revive(a)?);
    f.pages.selected_page().native.go_forward();
    wait("pushState route", || {
        f.pages
            .selected_page()
            .native
            .uri()
            .is_some_and(|u| u.ends_with("/route"))
            && !f.pages.selected_page().native.is_loading()
    })?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(a));
    f.pages.selected_page().native.go_forward();
    f.arrived(b)?;
    assert_eq!(f.pages.read(|b| b.snapshot())?.visits.len(), 2);
    assert_eq!(f.server.hits("/route"), 0);
    Ok(())
}

fn download_release() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    f.pages.observe(move |event| {
        if let PageEvent::Download(event) = event {
            received.borrow_mut().push(event);
        }
    });
    f.server.held.store(true, Ordering::Relaxed);
    f.pages
        .selected_page()
        .native
        .download_uri(f.server.url("/file").as_str());
    wait("download starts", || !events.borrow().is_empty())?;
    f.release()?;
    f.server.held.store(false, Ordering::Relaxed);
    wait("download saves", || events.borrow().len() >= 2)?;
    let saved = f.pages.read(|b| b.downloads())?;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].visit, a);
    assert_eq!(std::fs::read(&saved[0].path)?, vec![42; 131072]);
    assert_eq!(events.borrow().len(), 2);
    assert!(!f.revive(a)?);
    Ok(())
}

fn late_callbacks() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("const r=document.createRange();r.selectNodeContents(document.getElementById('fragment'));getSelection().removeAllRanges();getSelection().addRange(r);setTimeout(()=>{document.title='late title';scrollTo(0,4000)},100);return true;")?;
    let title = f.pages.read(|b| b.visit(a))?.title;
    let captures = f.pages.read(|b| b.captures(a))?.len();
    let scroll = f.pages.read(|b| b.scroll(a))?;
    f.pages.remember_selection()?;
    f.release()?;
    settle();
    assert_eq!(f.pages.read(|b| b.visit(a))?.title, title);
    assert_eq!(f.pages.read(|b| b.captures(a))?.len(), captures);
    assert_eq!(f.pages.read(|b| b.scroll(a))?, scroll);
    assert!(!f.revive(a)?);
    Ok(())
}

fn tree_boundary() -> Result<()> {
    let f = Fixture::new()?;
    let [a, _, _] = f.chain()?;
    let forest = f.pages.read(|b| {
        canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)
    })?;
    let old_tree = forest.tree_of(a).unwrap();
    f.pages.go(Go::Address {
        url: f.server.url("/other"),
        target: NavigationTarget::Root,
    })?;
    wait("other tree", || {
        f.pages.read(|b| b.snapshot()).unwrap().visits.len() == 4
            && !f.pages.selected_page().native.is_loading()
    })?;
    let other = f.pages.read(|b| b.snapshot())?.current.unwrap();
    f.release()?;
    assert!(!f.revive(other)?);
    f.pages.selected_page().native.go_back();
    settle();
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(other));
    f.pages.settle_tree(old_tree)?;
    f.release()?;
    assert!(!f.revive(other)?);
    f.pages.selected_page().native.go_back();
    settle();
    assert_eq!(f.pages.read(|b| b.snapshot())?.current, Some(other));
    Ok(())
}

fn unresponsive_release() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    let started = Rc::new(std::cell::Cell::new(false));
    let flag = started.clone();
    f.pages.selected_page().native.evaluate_javascript(
        "while(true) {}",
        None,
        None,
        None::<&gio::Cancellable>,
        move |_| flag.set(true),
    );
    let start = Instant::now();
    f.release()?;
    assert!(!started.get(), "release must not wait for the hung script");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "release wait must be bounded"
    );
    Ok(())
}
fn scroll_checkpoint_current() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("scrollTo(0,1000); return true;")?;
    wait("saved old scroll", || {
        f.pages.read(|b| b.scroll(a)).unwrap() == Some(1000)
    })?;
    f.eval("scrollTo(0,3000); return true;")?;
    f.release()?;
    assert_eq!(
        f.pages.read(|b| b.scroll(a))?,
        Some(3000),
        "checkpoint captures current scroll"
    );
    assert!(!f.revive(a)?);
    wait("current scroll restored", || {
        f.eval("return scrollY;").unwrap() == json!(3000)
    })?;
    Ok(())
}
fn scroll_checkpoint_back() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("scrollTo(0,1000); return true;")?;
    wait("saved old scroll", || {
        f.pages.read(|b| b.scroll(a)).unwrap() == Some(1000)
    })?;
    f.eval("scrollTo(0,2500); return true;")?;
    f.navigate("/b")?;
    f.release()?;
    assert!(!f.revive(a)?);
    wait("WebKit history scroll", || {
        f.eval("return scrollY;").unwrap() == json!(2500)
    })?;
    Ok(())
}
fn duplicate_entry() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("history.pushState({}, '', '/route'); history.pushState({}, '', '/a'); history.go(-2); return true;")?;
    wait("original history position", || {
        f.pages
            .selected_page()
            .native
            .back_forward_list()
            .unwrap()
            .back_list()
            .is_empty()
    })?;
    f.release()?;
    assert!(!f.revive(a)?);
    assert!(
        f.pages
            .selected_page()
            .native
            .back_forward_list()
            .unwrap()
            .back_list()
            .is_empty(),
        "saved cursor wins over duplicate URL"
    );
    Ok(())
}
fn observer_release() -> Result<()> {
    let f = Fixture::new()?;
    f.navigate("/a")?;
    let weak = Rc::downgrade(&f.pages);
    let once = Rc::new(std::cell::Cell::new(false));
    f.pages.observe(move |event| {
        if matches!(event, PageEvent::Changed) && !once.replace(true) {
            let pages = weak.upgrade().unwrap();
            pages.release_selected_page();
            pages.resize(1000., 800.);
            pages.focus();
        }
    });
    let _ = f.pages.open_roots(&[f.server.url("/root")]);
    wait("observer checkpoint", || {
        f.pages.checkpoint_size().is_some()
    })?;
    Ok(())
}
fn alias_only() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.navigate("/a")?;
    f.eval("history.replaceState({}, '', '/route'); return true;")?;
    wait("alias URL", || {
        f.pages.selected_page().native.uri().as_deref() == Some(f.server.url("/route").as_str())
    })?;
    f.release()?;
    assert!(f.revive(a)?);
    let list = f.pages.selected_page().native.back_forward_list().unwrap();
    assert_eq!(list.length(), 1);
    assert_eq!(
        f.pages.selected_page().native.uri().as_deref(),
        Some(f.server.url("/a").as_str())
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
    let selected = std::env::args().nth(1);
    type Case = (&'static str, fn() -> Result<()>);
    let cases: &[Case] = &[
        ("scroll_checkpoint_current", scroll_checkpoint_current),
        ("scroll_checkpoint_back", scroll_checkpoint_back),
        ("duplicate_entry", duplicate_entry),
        ("observer_release", observer_release),
        ("alias_only", alias_only),
        ("history_release", history_release),
        ("download_release", download_release),
        ("late_callbacks", late_callbacks),
        ("tree_boundary", tree_boundary),
        ("other_target", other_target),
        ("fork", fork),
        ("fork_without_binding", fork_without_binding),
        ("wrong_length", || corrupt("length")),
        ("wrong_position", || corrupt("position")),
        ("wrong_url", || corrupt("url")),
        ("absent", absent),
        ("same_document", same_document),
        ("push_state_route", push_state_route),
        ("in_flight", in_flight),
        ("failure_retry", failure_retry),
        ("unresponsive_release", unresponsive_release),
    ];
    for (name, run) in cases {
        if selected.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        run()?;
        println!("PASS {name}");
        settle();
    }
    Ok(())
}
