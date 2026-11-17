mod common;
#[path = "common/http.rs"]
mod http;
#[path = "common/live_measure.rs"]
mod measurements;
use anyhow::Result;
use canopy::pages::Go;
use gtk::prelude::*;
use std::{
    collections::BTreeMap,
    io::Write,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};
use webkit2gtk::WebViewExt;

struct Fixture {
    ui: std::rc::Rc<canopy::ui::Ui>,
    origin: String,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
    _directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let ui = canopy::ui::Ui::new(directory.path())?;
        ui.window.show_all();
        common::ready(&ui)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let counts = hits.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let counts = counts.clone();
                thread::spawn(move || {
                    let Ok(request) = http::request(&mut stream) else {
                        return;
                    };
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or("/");
                    *counts.lock().unwrap().entry(path.to_owned()).or_default() += 1;
                    if path.starts_with("/slow") {
                        thread::sleep(std::time::Duration::from_secs(2));
                    }
                    if path == "/slow-fail"
                        && counts.lock().unwrap().get(path).copied().unwrap_or(0) > 1
                    {
                        return;
                    }
                    if path == "/attachment" {
                        thread::sleep(std::time::Duration::from_millis(700));
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=hidden.txt\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfile");
                        return;
                    }
                    let body = "<!doctype html><title>live</title><body style='height:10000px'><input id=field><a id=link href='/clicked' style='display:block;height:100px'>click</a><script>window.counter=0;setInterval(()=>counter++,100)</script>";
                    let colour = if path.starts_with("/red") {
                        "#ff0000"
                    } else if path.starts_with("/blue") {
                        "#0000ff"
                    } else {
                        "#ffffff"
                    };
                    let mut body = body.replace(
                        "height:10000px",
                        &format!("height:10000px;background:{colour}"),
                    );
                    if path.starts_with("/plain") {
                        body = body.replace("setInterval(()=>counter++,100)", "");
                    }
                    if path.starts_with("/loop") {
                        body.push_str("<script>setTimeout(()=>location.href='/loop?n='+Date.now(),100)</script>");
                    }
                    if path == "/heavy" {
                        body.push_str("<script>window.memory=new Uint8Array(128*1024*1024);memory.fill(42)</script>");
                    }
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        Ok(Self {
            ui,
            origin,
            hits,
            _directory: directory,
        })
    }
    fn root(&self, path: &str) -> Result<canopy::browser::VisitId> {
        self.ui.pages.open_roots(&[self.url(path)])?;
        common::wait("root visible", || {
            self.ui
                .snapshot()
                .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == path))
                && !self.ui.pages.selected_page().native.is_loading()
        })?;
        common::pump(200);
        Ok(self.ui.snapshot()?.current.unwrap())
    }
    fn url(&self, path: &str) -> url::Url {
        format!("{}{path}", self.origin).parse().unwrap()
    }
    fn select(&self, visit: canopy::browser::VisitId) -> Result<()> {
        self.ui.pages.go(Go::Visit(visit))?;
        common::wait("visit selected", || {
            self.ui.snapshot().is_ok_and(|s| s.current == Some(visit))
                && !self.ui.pages.selected_page().native.is_loading()
        })?;
        common::pump(200);
        Ok(())
    }
    fn hits(&self, path: &str) -> usize {
        *self.hits.lock().unwrap().get(path).unwrap_or(&0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.ui.window.close();
        common::pump(100);
    }
}
fn eval_native(view: &webkit2gtk::WebView, script: &str) -> Result<serde_json::Value> {
    use javascriptcore::ValueExt;
    let result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let received = result.clone();
    view.evaluate_javascript(
        script,
        None,
        None,
        None::<&gio::Cancellable>,
        move |value| {
            *received.borrow_mut() = Some(value.map(|value| value.to_str().to_string()));
        },
    );
    common::wait("hidden script result", || result.borrow().is_some())?;
    let text = result.borrow_mut().take().unwrap()?;
    Ok(serde_json::from_str(&text)?)
}
fn a_new_tree_editor_keeps_focus_after_an_instant_live_return() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    f.root("/b")?;
    f.ui.pages.go(Go::Visit(a))?;
    f.ui.chrome.new_tree();
    common::wait("new tree editor focused after live return", || {
        common::chrome_focused(&f.ui, "#start-search")
    })?;
    common::pump(200);
    assert!(
        common::chrome_focused(&f.ui, "#start-search"),
        "a late selected-page frame must not hide the newly opened editor"
    );
    Ok(())
}
fn live_switch_keeps_document_and_running_script() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    common::eval(
        &f.ui,
        "field.value='kept';scrollTo(0,800);window.startCounter=counter;JSON.stringify(true)",
    )?;
    let native = f.ui.pages.selected_page().native.clone();
    f.root("/b")?;
    let hidden_before = eval_native(&native, "JSON.stringify(counter)")?
        .as_u64()
        .unwrap();
    let sample = thread::spawn(canopy::budget::measure).join().unwrap();
    println!(
        "MEASURE sandbox={} sample={sample:?}",
        f.ui.pages.sandbox_enabled()
    );
    sample?;
    common::pump(1200);
    let hidden_after = eval_native(&native, "JSON.stringify(counter)")?
        .as_u64()
        .unwrap();
    assert!(
        hidden_after > hidden_before,
        "the script counter advances while the page stays hidden"
    );
    f.select(a)?;
    assert_eq!(f.hits("/a"), 1, "live return must send no request");
    let state = common::eval(
        &f.ui,
        "JSON.stringify([field.value,counter>startCounter,scrollY])",
    )?;
    assert_eq!(state[0], "kept");
    assert_eq!(state[1], true);
    assert_eq!(state[2], 800);
    Ok(())
}
fn hidden_navigation_stays_in_its_branch_and_alert_returns() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    let native = f.ui.pages.selected_page().native.clone();
    let b = f.root("/b")?;
    native.evaluate_javascript(
        "history.pushState({},'', '/hidden-route');alert('hidden');location.href='/after'",
        None,
        None,
        None::<&gio::Cancellable>,
        |_| {},
    );
    common::wait("hidden navigation recorded", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.visits
                .iter()
                .any(|v| v.url.path() == "/after" && v.parent == Some(a))
        })
    })?;
    assert_eq!(f.ui.snapshot()?.current, Some(b));
    assert!(
        !gtk::Window::list_toplevels().iter().any(|w| w.is_visible()
            && w.downcast_ref::<gtk::Window>()
                .is_some_and(|w| w.is_modal())),
        "hidden alert must not present modal UI"
    );
    assert_eq!(
        f.ui.pages.selected_page().native.uri().as_deref(),
        Some(f.url("/b").as_str())
    );
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(a)
        .unwrap();
    f.ui.pages.go(Go::Tree(tree))?;
    assert_eq!(
        f.ui.snapshot()?.current_visit().unwrap().url.path(),
        "/after",
        "tree return shows the branch's latest hidden document"
    );
    assert_eq!(f.hits("/after"), 1);
    Ok(())
}
fn a_link_parent_in_another_tree_gets_its_own_branch() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(a)
        .unwrap();
    let b = f.root("/b")?;
    let landing = f.ui.pages.selected_page().native.clone();
    f.ui.pages.go(Go::Address {
        url: f.url("/child"),
        target: canopy::browser::NavigationTarget::ChildOf(a),
    })?;
    common::wait("child in its own branch", || {
        f.ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == "/child"))
            && !f.ui.pages.selected_page().native.is_loading()
    })?;
    assert_ne!(f.ui.pages.selected_page().native, landing);
    assert!(
        !f.ui.pages.selected_history_visits().contains(&b),
        "history cannot mix two trees"
    );
    f.ui.pages.settle_tree(tree)?;
    common::wait("live landing selected", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(b))
            && f.ui.pages.selected_page().native == landing
    })?;
    assert_eq!(f.hits("/b"), 1, "live landing is retained");
    Ok(())
}
fn settling_a_tree_cancels_its_loading_child_candidate() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(a)
        .unwrap();
    let b = f.root("/b")?;
    f.ui.pages.go(Go::Address {
        url: f.url("/slow"),
        target: canopy::browser::NavigationTarget::ChildOf(a),
    })?;
    common::wait("child candidate requested", || f.hits("/slow") == 1)?;
    f.ui.pages.settle_tree(tree)?;
    common::pump(2400);
    assert_eq!(f.ui.snapshot()?.current, Some(b));
    assert!(
        !f.ui
            .snapshot()?
            .visits
            .iter()
            .any(|v| v.url.path() == "/slow"),
        "settled candidate must not reopen its tree at commit"
    );
    Ok(())
}
fn selecting_a_live_page_keeps_its_inflight_navigation() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    let native = f.ui.pages.selected_page().native.clone();
    f.root("/b")?;
    native.evaluate_javascript(
        "location.href='/slow'",
        None,
        None,
        None::<&gio::Cancellable>,
        |_| {},
    );
    common::wait("hidden navigation started", || {
        f.hits("/slow") == 1 && native.is_loading()
    })?;
    f.ui.pages.go(Go::Visit(a))?;
    assert_eq!(
        f.ui.snapshot()?.current,
        Some(a),
        "selection moves while its navigation is in flight"
    );
    assert_eq!(f.ui.pages.selected_page().native, native);
    assert!(native.is_loading());
    assert_eq!(f.hits("/a"), 1);
    common::wait("retained load commits", || {
        f.ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == "/slow"))
            && !native.is_loading()
    })?;
    assert_eq!(f.hits("/slow"), 1);
    Ok(())
}
fn windows_share_one_eviction_at_a_time() -> Result<()> {
    let first = Fixture::new()?;
    let a = first.root("/a")?;
    first.root("/b")?;
    let second = Fixture::new()?;
    let c = second.root("/c")?;
    second.root("/d")?;
    common::pump(1200);
    first.ui.pages.inject_memory_sample(Some(u64::MAX));
    second.ui.pages.inject_memory_sample(Some(u64::MAX));
    assert!(!first.ui.pages.live_visits().contains(&a));
    assert!(
        second.ui.pages.live_visits().contains(&c),
        "another window must wait for the first drop"
    );
    common::pump(1700);
    second.ui.pages.inject_memory_sample(Some(u64::MAX));
    assert!(!second.ui.pages.live_visits().contains(&c));
    Ok(())
}
fn a_candidate_hold_does_not_finish_another_pages_checkpoint() -> Result<()> {
    let f = Fixture::new()?;
    let ancestor = f.root("/ancestor")?;
    f.ui.pages.go(Go::Address {
        url: f.url("/a"),
        target: canopy::browser::NavigationTarget::ChildOf(ancestor),
    })?;
    common::wait("source committed", || {
        f.ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == "/a"))
            && !f.ui.pages.selected_page().native.is_loading()
    })?;
    let a = f.ui.snapshot()?.current.unwrap();
    let source = f.ui.pages.selected_page().native.as_ptr() as usize;
    f.root("/b")?;
    let outgoing = f.ui.pages.selected_page().native.as_ptr() as usize;
    common::pump(1200);
    f.ui.pages.open_roots(&[f.url("/slow")])?;
    common::wait("candidate requested", || f.hits("/slow") == 1)?;
    let held_ended = std::rc::Rc::new(std::cell::Cell::new(false));
    let called = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = called.clone();
    let ended = held_ended.clone();
    let weak = std::rc::Rc::downgrade(&f.ui);
    let hits = f.hits.clone();
    f.ui.pages.observe_release(move |event| {
        if event.widget == outgoing && event.phase == canopy::pages::ReleasePhase::Hidden {
            ended.set(true);
        }
        if event.widget != source
            || event.phase != canopy::pages::ReleasePhase::Checkpointing
            || observed.replace(true)
        {
            return;
        }
        let ui = weak.upgrade().unwrap();
        common::wait(
            "candidate finishes while checkpoint observer retains the source",
            || {
                ui.snapshot()
                    .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == "/slow"))
                    && !ui.pages.selected_page().native.is_loading()
                    && ended.get()
            },
        )
        .unwrap();
        ui.pages.go(Go::Visit(a)).unwrap();
        common::pump(150);
        assert_eq!(
            *hits.lock().unwrap().get("/a").unwrap(),
            1,
            "the return waits for the outstanding checkpoint instead of loading cold"
        );
        assert_eq!(
            ui.snapshot().unwrap().current_visit().unwrap().url.path(),
            "/slow"
        );
    });
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    assert!(called.get());
    common::wait("return uses completed checkpoint", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(a))
            && !f.ui.pages.selected_page().native.is_loading()
    })?;
    assert!(f.ui.pages.selected_history_visits().contains(&ancestor));
    assert_eq!(f.hits("/a"), 2);
    Ok(())
}
fn incomplete_sample_drops_nothing_and_fresh_sample_drops_oldest() -> Result<()> {
    let f = Fixture::new()?;
    let ancestor = f.root("/ancestor")?;
    f.ui.pages.go(Go::Address {
        url: f.url("/a"),
        target: canopy::browser::NavigationTarget::ChildOf(ancestor),
    })?;
    common::wait("child committed", || {
        f.ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == "/a"))
            && !f.ui.pages.selected_page().native.is_loading()
    })?;
    let a = f.ui.snapshot()?.current.unwrap();
    common::eval(&f.ui, "scrollTo(0,900);JSON.stringify(true)")?;
    let b = f.root("/b")?;
    let c = f.root("/c")?;
    common::pump(1200);
    f.ui.pages.inject_memory_sample(None);
    assert!(f.ui.pages.live_visits().contains(&a));
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    common::wait("oldest dropped", || !f.ui.pages.live_visits().contains(&a))?;
    assert!(
        f.ui.pages.live_visits().contains(&b),
        "only one drop may use this sample"
    );
    assert!(
        f.ui.pages.live_visits().contains(&c),
        "selected is protected"
    );
    common::pump(700);
    f.select(a)?;
    assert_eq!(f.hits("/a"), 2, "dropped page must reload");
    assert!(
        f.ui.pages.selected_history_visits().contains(&ancestor),
        "checkpoint restores back list"
    );
    common::wait("checkpoint scroll restored", || {
        common::eval(&f.ui, "JSON.stringify(scrollY)").is_ok_and(|y| y == 900)
    })?;
    Ok(())
}
fn selected_and_candidate_are_protected() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/a")?;
    let b = f.root("/b")?;
    f.ui.pages.open_roots(&[f.url("/slow")])?;
    common::wait("candidate requested", || f.hits("/slow") == 1)?;
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    common::wait("candidate commits", || {
        f.ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == "/slow"))
    })?;
    assert_eq!(f.hits("/slow"), 1);
    assert!(
        f.ui.pages.live_visits().contains(&b),
        "outgoing is protected while held"
    );
    Ok(())
}
fn settle_removes_live_pages_and_disables_old_input() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    let (x, y) = common::element_point(&f.ui, "link")?;
    common::xdo(&["mousemove", &x.to_string(), &y.to_string(), "click", "2"])?;
    common::wait("queued branch", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.visits
                .iter()
                .any(|v| v.parent == Some(a) && v.opened_at.is_none())
        })
    })?;
    let child =
        f.ui.snapshot()?
            .visits
            .into_iter()
            .find(|v| v.parent == Some(a) && v.opened_at.is_none())
            .unwrap()
            .id;
    f.select(child)?;
    let native = f.ui.pages.selected_page().native.clone();
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(a)
        .unwrap();
    let b = f.root("/b")?;
    f.select(child)?;
    f.ui.pages.settle_tree(tree)?;
    assert!(
        !native.is_sensitive(),
        "settled view rejects input immediately"
    );
    common::wait("landing selected", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(b))
    })?;
    common::pump(1300);
    assert!(!f.ui.pages.live_visits().contains(&a));
    assert!(!f.ui.pages.live_visits().contains(&child));
    Ok(())
}
fn settled_page_rejects_clicks_while_dormant_landing_loads() -> Result<()> {
    let f = Fixture::new()?;
    let landing = f.root("/slow")?;
    let old = f.root("/old")?;
    common::pump(1200);
    let released = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = released.clone();
    f.ui.pages.observe_release(move |event| {
        if event.phase == canopy::pages::ReleasePhase::Released {
            observed.set(true);
        }
    });
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    common::wait("landing checkpointed", || released.get())?;
    assert!(!f.ui.pages.live_visits().contains(&landing));
    let native = f.ui.pages.selected_page().native.clone();
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(old)
        .unwrap();
    let (x, y) = common::element_point(&f.ui, "link")?;
    f.ui.pages.settle_tree(tree)?;
    assert!(!native.is_sensitive());
    common::xdo(&["mousemove", &x.to_string(), &y.to_string(), "click", "1"])?;
    common::pump(200);
    assert_eq!(
        f.hits("/clicked"),
        0,
        "settled page must reject the click while landing loads"
    );
    common::wait("dormant landing selected", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(landing))
            && f.ui.pages.selected_page().native.uri().as_deref() == Some(f.url("/slow").as_str())
            && !f.ui.pages.selected_page().native.is_loading()
    })?;
    assert_eq!(f.hits("/slow"), 2);
    Ok(())
}
fn commands_wait_for_a_settle_landing() -> Result<()> {
    for action in ["address", "reload", "failure"] {
        let f = Fixture::new()?;
        let path = if action == "failure" {
            "/slow-fail"
        } else {
            "/slow"
        };
        let landing = f.root(path)?;
        let old = f.root("/old")?;
        common::pump(1200);
        f.ui.pages.inject_memory_sample(Some(u64::MAX));
        common::wait("landing dropped", || {
            !f.ui.pages.live_visits().contains(&landing)
        })?;
        let old_view = f.ui.pages.selected_page().native.clone();
        let tree =
            f.ui.pages
                .read(|b| {
                    canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)
                })?
                .tree_of(old)
                .unwrap();
        f.ui.pages.settle_tree(tree)?;
        common::pump(100);
        if action == "address" {
            f.ui.pages.go(Go::Address {
                url: f.url("/typed"),
                target: canopy::browser::NavigationTarget::ChildOf(old),
            })?;
            assert_eq!(f.hits("/typed"), 0, "address waits for the landing");
        } else {
            f.ui.pages.reload()?;
        }
        common::wait("landing and queued command complete", || {
            let snapshot = f.ui.snapshot().unwrap();
            match action {
                "address" => snapshot
                    .current_visit()
                    .is_some_and(|v| v.url.path() == "/typed"),
                "reload" => {
                    snapshot.current == Some(landing)
                        && !f.ui.pages.selected_page().native.is_loading()
                }
                _ => snapshot.current.is_none() && f.ui.pages.selected_page().native != old_view,
            }
        })?;
        common::pump(700);
        assert!(old_view.parent().is_none(), "retired page is released");
        assert!(f.ui.pages.selected_page().native.is_sensitive());
        assert_eq!(f.hits(path), 2, "reload during landing is ignored");
        if action == "address" {
            assert_eq!(
                f.ui.snapshot()?.current_visit().unwrap().parent,
                Some(landing)
            );
        }
        if action == "failure" {
            f.root("/usable")?;
        }
        let (x, y) = common::element_point(&f.ui, "link")?;
        common::xdo(&["mousemove", &x.to_string(), &y.to_string(), "click", "1"])?;
        common::wait("real click after landing", || f.hits("/clicked") == 1)?;
    }
    Ok(())
}
fn a_fragment_navigation_does_not_protect_a_hidden_page_from_eviction() -> Result<()> {
    let f = Fixture::new()?;
    let root = f.root("/fragment")?;
    f.ui.pages.go(Go::Address {
        url: f.url("/fragment#part"),
        target: canopy::browser::NavigationTarget::ChildOf(root),
    })?;
    common::wait("fragment selected", || {
        f.ui.pages
            .selected_page()
            .native
            .uri()
            .is_some_and(|u| u.ends_with("#part"))
            && !f.ui.pages.selected_page().native.is_loading()
    })?;
    let fragment = f.ui.snapshot()?.current.unwrap();
    f.root("/other")?;
    common::pump(1200);
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    common::wait("hidden fragment page evicted", || {
        !f.ui.pages.live_visits().contains(&fragment)
    })?;
    Ok(())
}
fn hidden_download_uses_its_source_visit() -> Result<()> {
    let f = Fixture::new()?;
    let source = f.root("/source")?;
    let view = f.ui.pages.selected_page().native.clone();
    f.root("/visible")?;
    eval_native(&view, "location.href='/attachment';JSON.stringify(true)")?;
    common::pump(100);
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    assert!(
        f.ui.pages.live_visits().contains(&source),
        "an unclassified response may become a download and must keep its source"
    );
    common::wait("hidden download recorded", || {
        f.ui.downloads().is_ok_and(|d| !d.is_empty())
    })?;
    assert_eq!(f.ui.downloads()?[0].visit, source);
    Ok(())
}
fn audible_hidden_page_is_protected() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/audio")?;
    common::eval(&f.ui, "window.audio=new AudioContext();window.osc=audio.createOscillator();osc.connect(audio.destination);osc.start();audio.resume();JSON.stringify(true)")?;
    common::pump(1200);
    let native = f.ui.pages.selected_page().native.clone();
    if !native.is_playing_audio() {
        println!("UNTESTED WebKit does not report oscillator audio under this Xvfb audio setup");
        return Ok(());
    }
    f.root("/other")?;
    common::pump(1200);
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    common::pump(700);
    assert!(f.ui.pages.live_visits().contains(&a));
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(a)
        .unwrap();
    f.select(a)?;
    f.ui.pages.settle_tree(tree)?;
    assert!(
        native.is_muted(),
        "settling stops selected audio immediately, including while it covers a landing"
    );
    Ok(())
}
fn small_budget_drops_hidden_page() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/a")?;
    f.root("/b")?;
    common::wait("small budget drops hidden page", || {
        !f.ui.pages.live_visits().contains(&a)
    })?;
    Ok(())
}
fn redirect_loop_commands() -> Result<()> {
    for command in ["new-tree", "palette", "tab", "settle", "middle"] {
        let f = Fixture::new()?;
        let other = f.root("/slow-other")?;
        f.root("/loop")?;
        common::wait("redirect loop active", || {
            f.hits
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p.starts_with("/loop"))
                .map(|(_, n)| *n)
                .sum::<usize>()
                >= 3
        })?;
        match command {
            "new-tree" | "palette" => {
                common::key(if command == "new-tree" {
                    "ctrl+t"
                } else {
                    "ctrl+k"
                })?;
                common::wait("address editor focused", || {
                    common::chrome_focused(
                        &f.ui,
                        if command == "new-tree" {
                            "#start-search"
                        } else {
                            "#palette-input"
                        },
                    )
                })?;
                common::type_text(f.url("/slow-new").as_str())?;
                common::key(if command == "palette" {
                    "alt+Return"
                } else {
                    "Return"
                })?;
            }
            "tab" => {
                let (x, y, w, h) = common::rect(&f.ui, &format!(".tab[data-id='{other}']"))?;
                let (x, y) = common::screen_point(
                    &f.ui,
                    &f.ui.chrome.native(),
                    (x + w / 2.) as i32,
                    (y + h / 2.) as i32,
                )?;
                common::click_at(x, y)?;
            }
            "settle" => common::key("ctrl+w")?,
            "middle" => {
                let (x, y) = common::page_point(&f.ui, 50, 130)?;
                common::move_to(x, y)?;
                common::xdo(&["click", "2"])?;
            }
            _ => unreachable!(),
        }
        let result = common::wait("owner command lands during redirect loop", || {
            let snapshot = f.ui.snapshot().unwrap();
            if command == "middle" {
                snapshot.visits.iter().any(|v| v.url.path() == "/clicked")
            } else {
                snapshot.current_visit().is_some_and(|v| {
                    v.url.path()
                        == if command == "tab" || command == "settle" {
                            "/slow-other"
                        } else {
                            "/slow-new"
                        }
                })
            }
        });
        println!(
            "REDIRECT {command}: {}",
            if result.is_ok() { "landed" } else { "FAILED" }
        );
        result?;
        if command == "middle" {
            assert!(
                f.ui.snapshot()?
                    .current_visit()
                    .is_some_and(|v| v.url.path() == "/loop"),
                "middle-click queues without selecting its destination"
            );
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let download_home = tempfile::tempdir()?;
    std::fs::write(
        download_home.path().join("user-dirs.dirs"),
        format!(
            "XDG_DOWNLOAD_DIR=\"{}/Saved files\"\n",
            download_home.path().display()
        ),
    )?;
    std::env::set_var("XDG_CONFIG_HOME", download_home.path());

    let directory = tempfile::tempdir()?;
    common::start(directory.path())?;
    type Case = (&'static str, fn() -> Result<()>);
    if std::env::args().nth(1).as_deref() == Some("measurements") {
        return measurements::run();
    }
    if std::env::args().nth(1).as_deref() == Some("small-budget") {
        return small_budget_drops_hidden_page();
    }
    if std::env::args().nth(1).as_deref() == Some("redirect-loop") {
        return redirect_loop_commands();
    }
    let cases: &[Case] = &[
        ("redirect_loop_commands", redirect_loop_commands),
        (
            "a_fragment_navigation_does_not_protect_a_hidden_page_from_eviction",
            a_fragment_navigation_does_not_protect_a_hidden_page_from_eviction,
        ),
        (
            "commands_wait_for_a_settle_landing",
            commands_wait_for_a_settle_landing,
        ),
        (
            "hidden_download_uses_its_source_visit",
            hidden_download_uses_its_source_visit,
        ),
        (
            "a_new_tree_editor_keeps_focus_after_an_instant_live_return",
            a_new_tree_editor_keeps_focus_after_an_instant_live_return,
        ),
        (
            "a_candidate_hold_does_not_finish_another_pages_checkpoint",
            a_candidate_hold_does_not_finish_another_pages_checkpoint,
        ),
        (
            "settling_a_tree_cancels_its_loading_child_candidate",
            settling_a_tree_cancels_its_loading_child_candidate,
        ),
        (
            "a_link_parent_in_another_tree_gets_its_own_branch",
            a_link_parent_in_another_tree_gets_its_own_branch,
        ),
        (
            "windows_share_one_eviction_at_a_time",
            windows_share_one_eviction_at_a_time,
        ),
        (
            "selecting_a_live_page_keeps_its_inflight_navigation",
            selecting_a_live_page_keeps_its_inflight_navigation,
        ),
        (
            "settled_page_rejects_clicks_while_dormant_landing_loads",
            settled_page_rejects_clicks_while_dormant_landing_loads,
        ),
        (
            "audible_hidden_page_is_protected",
            audible_hidden_page_is_protected,
        ),
        (
            "live_switch_keeps_document_and_running_script",
            live_switch_keeps_document_and_running_script,
        ),
        (
            "hidden_navigation_stays_in_its_branch_and_alert_returns",
            hidden_navigation_stays_in_its_branch_and_alert_returns,
        ),
        (
            "incomplete_sample_drops_nothing_and_fresh_sample_drops_oldest",
            incomplete_sample_drops_nothing_and_fresh_sample_drops_oldest,
        ),
        (
            "selected_and_candidate_are_protected",
            selected_and_candidate_are_protected,
        ),
        (
            "settle_removes_live_pages_and_disables_old_input",
            settle_removes_live_pages_and_disables_old_input,
        ),
    ];
    for (name, case) in cases {
        case()?;
        println!("PASS {name}");
    }
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg("small-budget")
        .env("CANOPY_MEMORY_BUDGET_MB", "1")
        .status()?;
    assert!(status.success(), "small budget child");
    println!("PASS small_budget_drops_hidden_page");
    Ok(())
}
