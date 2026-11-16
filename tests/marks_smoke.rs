mod common;
#[path = "common/http.rs"]
mod http;
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
        // Give Xvfb a native chrome window so retained page windows cannot occlude captures.
        if let Some(window) = ui.chrome.native().window() {
            window.ensure_native();
        }
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let icon = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 16, 16)
            .unwrap();
        icon.fill(0x2050ffff);
        let favicon = icon.save_to_bufferv("png", &[])?;
        let counts = hits.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let counts = counts.clone();
                let favicon = favicon.clone();
                thread::spawn(move || {
                    let Ok(request) = http::request(&mut stream) else {
                        return;
                    };
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or("/");
                    *counts.lock().unwrap().entry(path.to_owned()).or_default() += 1;
                    if path == "/favicon.ico" {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", favicon.len());
                        let _ = stream.write_all(&favicon);
                        return;
                    }
                    let body = format!("<!doctype html><title>{path}</title><body style='background:#edf5ed;color:#234;font:24px sans-serif;padding:60px'><h1>{path}</h1><p>A local Canopy garden fixture.</p><a id=link href='/waiting' style='display:block;height:100px'>Open another garden</a>");
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

impl Fixture {
    fn queue(&self, path: &str) -> Result<canopy::browser::VisitId> {
        let count = self.ui.snapshot()?.visits.len();
        common::eval(
            &self.ui,
            &format!(
                "link.href={};JSON.stringify(true)",
                serde_json::to_string(path)?
            ),
        )?;
        let (x, y) = common::element_point(&self.ui, "link")?;
        common::xdo(&["mousemove", &x.to_string(), &y.to_string(), "click", "2"])?;
        common::wait("middle click queues visit", || {
            self.ui
                .snapshot()
                .is_ok_and(|s| s.visits.len() == count + 1)
        })?;
        Ok(self.ui.snapshot()?.visits.last().unwrap().id)
    }
    fn committed(&self, visit: canopy::browser::VisitId) -> Result<()> {
        common::wait("background committed", || {
            self.ui.pages.loaded_visits().contains(&visit)
                && self
                    .ui
                    .pages
                    .background_view(visit)
                    .is_some_and(|v| !v.is_loading())
        })
    }
}
fn marked(f: &Fixture, id: canopy::browser::VisitId) -> bool {
    common::chrome(&f.ui, &format!("S.frame.visits['{id}']?.live === true"))
        .is_ok_and(|v| v == true)
}
fn strip_marked(f: &Fixture, id: canopy::browser::VisitId) -> bool {
    common::visible(&f.ui, &format!(".node[data-id='{id}'] .live-dot"))
}
fn switching_marks_only_the_page_left_behind() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/orchard")?;
    f.root("/river")?;
    f.ui.pages.go(Go::Address {
        url: f.url("/orchard-branch"),
        target: canopy::browser::NavigationTarget::ChildOf(a),
    })?;
    common::wait("branch shown", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.current_visit()
                .is_some_and(|v| v.url.path() == "/orchard-branch")
        }) && !f.ui.pages.selected_page().native.is_loading()
    })?;
    let b = f.ui.snapshot()?.current.unwrap();
    common::wait("hidden page marked", || marked(&f, a))?;
    assert!(!marked(&f, b));
    f.select(a)?;
    common::open_strip(&f.ui)?;
    common::wait("strip marks outgoing page", || strip_marked(&f, b))?;
    assert!(!strip_marked(&f, a));
    Ok(())
}
fn pressure_removes_the_mark_and_reopening_restores_it() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/orchard")?;
    let b = f.root("/river")?;
    common::wait("hidden mark before pressure", || marked(&f, a))?;
    let start = std::time::Instant::now();
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    common::wait("dropped mark removed", || !marked(&f, a))?;
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    common::pump(1600);
    f.select(a)?;
    f.select(b)?;
    common::wait("reopened page marked again", || marked(&f, a))?;
    assert_eq!(f.hits("/orchard"), 2);
    Ok(())
}
fn hidden_navigation_moves_the_mark_to_the_current_document() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/orchard")?;
    let view = f.ui.pages.selected_page().native.clone();
    f.root("/river")?;
    common::wait("old visit marked", || marked(&f, a))?;
    eval_native(&view, "location.href='/new-orchard';JSON.stringify(true)")?;
    common::wait("mark moves to hidden commit", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.visits
                .iter()
                .any(|v| v.url.path() == "/new-orchard" && marked(&f, v.id))
        }) && !marked(&f, a)
    })?;
    Ok(())
}
fn settling_removes_the_trees_marks() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/orchard")?;
    f.root("/river")?;
    common::wait("live mark before settling", || marked(&f, a))?;
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(a)
        .unwrap();
    f.ui.pages.settle_tree(tree)?;
    common::wait("settled mark removed", || !marked(&f, a))?;
    Ok(())
}
fn never_seen_pages_keep_the_loaded_dot_until_opened_and_left() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/orchard")?;
    f.ui.pages.inject_recent_memory_sample(0);
    let queued = f.queue("/waiting")?;
    f.committed(queued)?;
    common::wait("loaded state delivered", || {
        common::chrome(&f.ui, &format!("S.frame.visits['{queued}'].loaded"))
            .is_ok_and(|v| v == true)
    })?;
    assert!(!marked(&f, queued));
    f.select(queued)?;
    f.select(a)?;
    common::open_strip(&f.ui)?;
    common::wait("seen page live in strip", || strip_marked(&f, queued))?;
    assert!(common::chrome(&f.ui,&format!("!document.querySelector('.node[data-id=\"{queued}\"] .pending-dot') && !S.frame.visits['{queued}'].queued"))? == true);
    Ok(())
}
fn marks_share_surfaces_without_moving_nodes() -> Result<()> {
    let f = Fixture::new()?;
    let root = f.root("/orchard")?;
    f.ui.pages.inject_recent_memory_sample(0);
    let branch = f.queue("/orchard-branch")?;
    f.committed(branch)?;
    f.root("/meadow")?;
    f.root("/river")?;
    f.select(branch)?;
    common::open_strip(&f.ui)?;
    common::wait("root marked", || strip_marked(&f, root))?;
    let selector = format!(".node[data-id='{root}']");
    let before = common::rect(&f.ui, &selector)?;
    common::chrome(
        &f.ui,
        &format!("(document.querySelector(\"{selector} .live-dot\").remove(),true)"),
    )?;
    assert_eq!(
        common::rect(&f.ui, &selector)?,
        before,
        "dot cannot move or resize a node"
    );
    common::chrome(&f.ui, "(canopy.present(S.frame),true)")?;
    for theme in ["dark", "light"] {
        common::chrome(
            &f.ui,
            &format!("(S.frame.theme='{theme}',canopy.present(S.frame),true)"),
        )?;
        common::pump(400);
        shot(&f, &format!("{theme}-strip"))?;
        common::chrome(&f.ui, "(canopy.toggleForest(),true)")?;
        common::wait("forest live mark", || {
            common::visible(&f.ui, &format!(".forest-node[data-id='{root}'] .live-dot"))
        })?;
        shot(&f, &format!("{theme}-forest"))?;
        common::chrome(&f.ui, "(canopy.toggleForest(),canopy.toggleTree(),true)")?;
        common::wait("sheet live mark", || {
            common::visible(&f.ui, &format!(".tree-tile[data-id='{root}'] .live-dot"))
        })?;
        shot(&f, &format!("{theme}-tree"))?;
        common::chrome(&f.ui, "(setTreeSide('list'),true)")?;
        assert!(common::visible(
            &f.ui,
            &format!(".tree-row[data-id='{root}'] .live-dot")
        ));
        assert!(
            common::text(&f.ui, &format!(".tree-row[data-id='{root}'] .menu-detail"))
                .contains("Open in memory")
        );
        shot(&f, &format!("{theme}-list"))?;
        common::chrome(&f.ui, "(setTreeSide('tree'),canopy.toggleTree(),true)")?;
    }
    Ok(())
}
fn shot(f: &Fixture, name: &str) -> Result<()> {
    if let Ok(folder) = std::env::var("CANOPY_MARK_SHOTS") {
        common::pump(300);
        common::save_window(&f.ui, &format!("{folder}/{name}.png"))?;
    }
    Ok(())
}
fn a_small_budget_removes_the_mark_when_the_page_is_dropped() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/orchard")?;
    let dropped = std::rc::Rc::new(std::cell::Cell::new(None));
    let observed = dropped.clone();
    f.ui.pages.observe_release(move |event| {
        if event.phase == canopy::pages::ReleasePhase::Checkpointing {
            observed.set(Some(std::time::Instant::now()));
        }
    });
    f.root("/river")?;
    common::wait("budget drops hidden page", || dropped.get().is_some())?;
    common::wait("budget drop clears chrome mark", || !marked(&f, a))?;
    assert!(dropped.get().unwrap().elapsed() < std::time::Duration::from_secs(1));
    Ok(())
}
fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    common::start(directory.path())?;
    if std::env::args().nth(1).as_deref() == Some("pictures") {
        return marks_share_surfaces_without_moving_nodes();
    }
    if std::env::args().nth(1).as_deref() == Some("small-budget") {
        a_small_budget_removes_the_mark_when_the_page_is_dropped()?;
        println!("PASS a_small_budget_removes_the_mark_when_the_page_is_dropped");
        return Ok(());
    }
    type Case = (&'static str, fn() -> Result<()>);
    let cases: &[Case] = &[
        (
            "switching_marks_only_the_page_left_behind",
            switching_marks_only_the_page_left_behind,
        ),
        (
            "pressure_removes_the_mark_and_reopening_restores_it",
            pressure_removes_the_mark_and_reopening_restores_it,
        ),
        (
            "hidden_navigation_moves_the_mark_to_the_current_document",
            hidden_navigation_moves_the_mark_to_the_current_document,
        ),
        (
            "settling_removes_the_trees_marks",
            settling_removes_the_trees_marks,
        ),
        (
            "never_seen_pages_keep_the_loaded_dot_until_opened_and_left",
            never_seen_pages_keep_the_loaded_dot_until_opened_and_left,
        ),
        (
            "marks_share_surfaces_without_moving_nodes",
            marks_share_surfaces_without_moving_nodes,
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
    assert!(status.success(), "small budget marks child");
    Ok(())
}
