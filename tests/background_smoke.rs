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
                    if path.starts_with("/hang") {
                        let mut byte = [0];
                        use std::io::Read;
                        let _ = stream.read(&mut byte);
                        return;
                    }
                    if path.starts_with("/image-page") {
                        let body =
                            "<!doctype html><title>image waiting</title><img src='/hang-image'>";
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        return;
                    }
                    if path == "/favicon.ico" {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",favicon.len());
                        let _ = stream.write_all(&favicon);
                        return;
                    }
                    if path.starts_with("/slow") {
                        thread::sleep(std::time::Duration::from_secs(
                            if matches!(path, "/slow-two" | "/slow-three") {
                                7
                            } else {
                                2
                            },
                        ));
                    }
                    if path == "/slow-fail"
                        && counts.lock().unwrap().get(path).copied().unwrap_or(0) > 1
                    {
                        return;
                    }
                    if path == "/slow-drop" || path == "/fail" {
                        return;
                    }
                    if path == "/empty" {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
                        );
                        return;
                    }
                    if path == "/redirect" {
                        let _ = write!(stream, "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        return;
                    }
                    if path == "/attachment" {
                        thread::sleep(std::time::Duration::from_millis(700));
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=hidden.txt\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfile");
                        return;
                    }
                    let body = "<!doctype html><link rel='icon' href='/favicon.ico'><title>live</title><body style='height:10000px'><input id=field><a id=link href='/clicked' style='display:block;height:100px'>click</a><script>window.counter=0;setInterval(()=>counter++,100)</script>";
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
fn middle_click_loads_silently_and_opening_reuses_the_muted_page() -> Result<()> {
    let f = Fixture::new()?;
    let source = f.root("/source")?;
    let view = f.ui.pages.selected_page().native.clone();
    view.grab_focus();
    let address = common::chrome(
        &f.ui,
        "[S.frame.current,S.frame.visits[S.frame.current].url]",
    )?;
    let trees = f.ui.pages.read(|b| b.tree_states())?;
    let database = rusqlite::Connection::open(f._directory.path().join("forest.sqlite3"))?;
    let arrivals: i64 =
        database.query_row("SELECT count(*) FROM visit_events", [], |r| r.get(0))?;
    let queued = f.queue("/redirect")?;
    f.committed(queued)?;
    assert_eq!(f.hits("/redirect"), 1);
    assert_eq!(f.hits("/final"), 1);
    assert_eq!(f.ui.snapshot()?.current, Some(source));
    assert_eq!(
        common::chrome(
            &f.ui,
            "[S.frame.current,S.frame.visits[S.frame.current].url]"
        )?,
        address
    );
    assert_eq!(f.ui.pages.read(|b| b.tree_states())?, trees);
    assert_eq!(
        database.query_row("SELECT count(*) FROM visit_events", [], |r| r
            .get::<_, i64>(0))?,
        arrivals
    );
    assert!(view.is_focus());
    let visit = f.ui.pages.read(|b| b.visit(queued))?;
    assert!(visit.opened_at.is_none());
    assert_eq!(visit.title, "live");
    assert_eq!(visit.url.path(), "/final");
    common::wait("background favicon and loaded frame", || {
        f.ui.pages.favicons().png(&visit.url, true).is_some()
            && common::chrome(&f.ui, &format!("S.frame.visits['{queued}'].loaded"))
                .is_ok_and(|v| v == true)
    })?;
    let background = f.ui.pages.background_view(queued).unwrap();
    assert!(background.is_muted());
    f.select(queued)?;
    assert_eq!(f.hits("/final"), 1);
    assert_eq!(f.ui.pages.selected_page().native, background);
    assert!(!background.is_muted());
    assert!(f.ui.pages.read(|b| b.visit(queued))?.opened_at.is_some());
    Ok(())
}
fn self_navigation_keeps_one_never_seen_visit() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let q = f.queue("/plain")?;
    f.committed(q)?;
    eval_native(
        &f.ui.pages.background_view(q).unwrap(),
        "location.href='/after';JSON.stringify(true)",
    )?;
    common::wait("self navigation updates same visit", || {
        f.ui.pages
            .read(|b| b.visit(q))
            .is_ok_and(|v| v.url.path() == "/after")
    })?;
    assert_eq!(f.ui.snapshot()?.visits.len(), 2);
    assert!(f.ui.pages.read(|b| b.visit(q))?.opened_at.is_none());
    let view = f.ui.pages.background_view(q).unwrap();
    assert_eq!(
        eval_native(&view, "JSON.stringify(confirm('hidden'))")?,
        false
    );
    eval_native(
        &view,
        "history.pushState({},'', '/script-route');JSON.stringify(true)",
    )?;
    common::wait("same document route updates queued URL", || {
        f.ui.pages
            .read(|b| b.visit(q))
            .is_ok_and(|v| v.url.path() == "/script-route")
    })?;
    assert_eq!(f.ui.snapshot()?.visits.len(), 2);
    Ok(())
}
fn failed_and_empty_background_loads_leave_no_loaded_page() -> Result<()> {
    let f = Fixture::new()?;
    let source = f.root("/source")?;
    for path in ["/fail", "/empty"] {
        let q = f.queue(path)?;
        common::wait("failed background view destroyed", || {
            f.hits(path) > 0 && f.ui.pages.background_view(q).is_none()
        })?;
        assert!(!f.ui.pages.loaded_visits().contains(&q));
        assert_eq!(f.ui.snapshot()?.current, Some(source));
        assert!(common::toast_text(&f.ui).is_none_or(|t| t.is_empty()));
    }
    Ok(())
}
fn a_background_download_is_refused_but_opening_downloads() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let q = f.queue("/attachment")?;
    common::wait("download view destroyed", || {
        f.hits("/attachment") == 1 && f.ui.pages.background_view(q).is_none()
    })?;
    assert!(!f.ui.pages.loaded_visits().contains(&q));
    assert!(f.ui.downloads()?.is_empty());
    assert!(!std::path::Path::new(&std::env::var("XDG_CONFIG_HOME")?)
        .join("saved")
        .exists());
    f.ui.pages.go(Go::Visit(q))?;
    common::wait("explicit open saves download", || {
        f.ui.downloads().is_ok_and(|v| v.len() == 1)
    })?;
    assert_eq!(f.hits("/attachment"), 2);
    Ok(())
}
fn opening_an_uncommitted_background_page_adopts_its_request() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let q = f.queue("/slow-adopt")?;
    common::wait("background request started", || f.hits("/slow-adopt") == 1)?;
    let view = f.ui.pages.background_view(q).unwrap();
    f.select(q)?;
    assert_eq!(f.hits("/slow-adopt"), 1);
    assert_eq!(view, f.ui.pages.selected_page().native);
    Ok(())
}
fn opening_pending_attachment_saves_once_with_one_request() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let saved = std::path::Path::new(&std::env::var("XDG_CONFIG_HOME")?).join("saved");
    let before = if saved.exists() {
        std::fs::read_dir(&saved)?.count()
    } else {
        0
    };
    let q = f.queue("/attachment")?;
    common::wait("attachment request pending", || f.hits("/attachment") == 1)?;
    f.ui.pages.go(Go::Visit(q))?;
    common::wait("adopted attachment recorded", || {
        f.ui.downloads().is_ok_and(|d| d.len() == 1)
    })?;
    common::pump(800);
    let downloads = f.ui.downloads()?;
    assert_eq!(downloads.len(), 1);
    assert_eq!(f.hits("/attachment"), 1);
    assert_eq!(std::fs::read_dir(saved)?.count(), before + 1);
    Ok(())
}
fn opening_pending_dropped_connection_reports_error_with_one_request() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let q = f.queue("/slow-drop")?;
    common::wait("failing request pending", || f.hits("/slow-drop") == 1)?;
    f.ui.pages.go(Go::Visit(q))?;
    common::wait("adopted failure reported", || {
        common::toast_text(&f.ui).is_some_and(|t| t.contains("Could not load"))
    })?;
    assert_eq!(f.hits("/slow-drop"), 1);
    Ok(())
}
fn unanswered_loads_expire_and_the_next_link_loads() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    f.ui.pages
        .set_background_timeout(std::time::Duration::from_secs(3));
    let mut stalled = Vec::new();
    for path in ["/hang-a", "/hang-b", "/hang-c"] {
        stalled.push(f.queue(path)?);
    }
    common::wait("three unanswered requests", || f.hits("/hang-c") == 1)?;
    let next = f.queue("/plain-next")?;
    assert_eq!(f.hits("/plain-next"), 0);
    common::wait("waiting pin is hollow for real unloaded visit", || {
        common::chrome(
            &f.ui,
            "!!document.querySelector('#waiting-pin .pending-dot:not(.loaded)')",
        )
        .is_ok_and(|v| v == true)
    })?;
    f.committed(next)?;
    common::wait("waiting pin is full after commit", || {
        common::chrome(
            &f.ui,
            "!!document.querySelector('#waiting-pin .pending-dot.loaded')",
        )
        .is_ok_and(|v| v == true)
    })?;
    common::wait("expired views removed", || {
        stalled
            .iter()
            .all(|q| f.ui.pages.background_view(*q).is_none())
    })?;
    for q in stalled {
        assert!(!f.ui.pages.loaded_visits().contains(&q));
    }
    assert_eq!(f.hits("/hang-a"), 1);
    f.ui.pages
        .set_background_timeout(std::time::Duration::from_secs(20));
    Ok(())
}
fn committed_documents_with_pending_images_free_their_slots() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let mut images = Vec::new();
    for path in ["/image-page-a", "/image-page-b", "/image-page-c"] {
        images.push(f.queue(path)?);
    }
    common::wait("documents committed while images wait", || {
        images
            .iter()
            .all(|q| f.ui.pages.loaded_visits().contains(q))
    })?;
    assert!(images
        .iter()
        .all(|q| f.ui.pages.background_view(*q).unwrap().is_loading()));
    let next = f.queue("/plain-next")?;
    f.committed(next)?;
    assert!(images
        .iter()
        .all(|q| f.ui.pages.loaded_visits().contains(q)));
    Ok(())
}
fn a_burst_admits_only_reserved_pages_and_drops_none() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let budget = canopy::budget::Budget::of_machine()?.bytes();
    f.ui.pages
        .inject_recent_memory_sample(budget - 2 * 256 * 1024 * 1024);
    let mut visits = Vec::new();
    for i in 0..5 {
        visits.push(f.queue(&format!("/slow-budget-{i}"))?);
    }
    assert_eq!(
        (0..5)
            .map(|i| f.hits(&format!("/slow-budget-{i}")))
            .sum::<usize>(),
        2
    );
    assert_eq!(
        visits
            .iter()
            .filter(|q| f.ui.pages.background_view(**q).is_some())
            .count(),
        2
    );
    f.ui.pages
        .inject_recent_memory_sample(budget - 256 * 1024 * 1024);
    common::wait("newest waiting link admitted by next sample", || {
        f.hits("/slow-budget-4") == 1
    })?;
    assert_eq!(f.hits("/slow-budget-2"), 0);
    assert_eq!(f.hits("/slow-budget-3"), 0);
    assert!(visits[..2]
        .iter()
        .all(|q| f.ui.pages.background_view(*q).is_some()));
    Ok(())
}
fn switching_elsewhere_keeps_an_adopted_background_load() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let q = f.queue("/slow-switch")?;
    common::wait("request started", || f.hits("/slow-switch") == 1)?;
    f.ui.pages.go(Go::Visit(q))?;
    f.root("/other")?;
    f.committed(q)?;
    assert_eq!(f.hits("/slow-switch"), 1);
    f.select(q)?;
    assert_eq!(f.hits("/slow-switch"), 1);
    Ok(())
}
fn three_loads_run_and_newest_pending_starts_first() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    for path in ["/slow-one", "/slow-two", "/slow-three"] {
        f.queue(path)?;
    }
    common::wait("three requests started", || f.hits("/slow-three") == 1)?;
    let old = f.queue("/slow-old")?;
    let new = f.queue("/slow-new")?;
    assert_eq!(f.hits("/slow-old"), 0);
    assert_eq!(f.hits("/slow-new"), 0);
    common::wait("newest starts before oldest", || f.hits("/slow-new") == 1)?;
    assert_eq!(
        f.hits("/slow-old"),
        0,
        "newest pending starts in the first freed slot"
    );
    f.committed(new)?;
    f.committed(old)?;
    Ok(())
}
fn windows_share_three_background_load_slots() -> Result<()> {
    let first = Fixture::new()?;
    first.root("/source")?;
    for path in ["/slow-one", "/slow-two", "/slow-three"] {
        first.queue(path)?;
    }
    common::wait("first window fills slots", || {
        first.hits("/slow-three") == 1
    })?;
    let second = Fixture::new()?;
    second.root("/source")?;
    let q = second.queue("/plain")?;
    assert_eq!(
        second.hits("/plain"),
        0,
        "other window waits for a shared slot"
    );
    second.committed(q)?;
    assert_eq!(second.hits("/plain"), 1);
    Ok(())
}
fn pressure_drops_oldest_never_seen_before_seen_and_reopening_fetches() -> Result<()> {
    let f = Fixture::new()?;
    let seen = f.root("/seen")?;
    f.root("/source")?;
    let old = f.queue("/old")?;
    let new = f.queue("/new")?;
    f.committed(old)?;
    f.committed(new)?;
    common::pump(1200);
    f.ui.pages.inject_memory_sample(Some(u64::MAX));
    assert!(f.ui.pages.background_view(old).is_none());
    assert!(!f.ui.pages.loaded_visits().contains(&old));
    assert!(f.ui.pages.background_view(new).is_some());
    assert!(f.ui.pages.live_visits().contains(&seen));
    common::pump(1100);
    f.select(old)?;
    assert_eq!(f.hits("/old"), 2);
    Ok(())
}
fn settling_destroys_never_seen_and_pending_loads() -> Result<()> {
    let f = Fixture::new()?;
    let source = f.root("/source")?;
    let q = f.queue("/plain")?;
    f.committed(q)?;
    for path in ["/slow-a", "/slow-b", "/slow-c", "/pending"] {
        f.queue(path)?;
    }
    let tree = f
        .ui
        .pages
        .read(|b| canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?
        .tree_of(source)
        .unwrap();
    f.ui.pages.settle_tree(tree)?;
    common::pump(3000);
    assert!(f.ui.pages.loaded_visits().is_empty());
    assert!(f.ui.pages.background_view(q).is_none());
    assert_eq!(f.hits("/pending"), 0);
    Ok(())
}
fn over_budget_a_new_link_stays_unloaded() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/source")?;
    let q = f.queue("/plain")?;
    common::pump(2500);
    assert_eq!(f.hits("/plain"), 0);
    assert!(f.ui.pages.background_view(q).is_none());
    Ok(())
}
fn pixel(f: &Fixture) -> [u8; 3] {
    let pix =
        f.ui.window
            .window()
            .unwrap()
            .pixbuf(400, 400, 1, 1)
            .unwrap();
    let bytes = pix.read_pixel_bytes();
    [bytes[0], bytes[1], bytes[2]]
}
fn first_frame(f: &Fixture, q: canopy::browser::VisitId) -> Result<f64> {
    let start = std::time::Instant::now();
    common::key("ctrl+Tab")?;
    common::wait("blue first shown frame", || pixel(f) == [0, 0, 255])?;
    assert_eq!(f.ui.snapshot()?.current, Some(q));
    Ok(start.elapsed().as_secs_f64() * 1000.)
}
fn cpu_ticks() -> Result<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    Ok(fields[11].parse::<u64>()? + fields[12].parse::<u64>()?)
}
fn measurements() -> Result<()> {
    let mut loaded = Vec::new();
    let mut cold = Vec::new();
    for _ in 0..20 {
        let f = Fixture::new()?;
        f.root("/red")?;
        let q = f.queue("/blue")?;
        f.committed(q)?;
        loaded.push(first_frame(&f, q)?);
        f.root("/red-cold")?;
        let q = f.queue("/blue-cold")?;
        f.committed(q)?;
        common::pump(1200);
        f.ui.pages.inject_memory_sample(Some(u64::MAX));
        assert!(f.ui.pages.background_view(q).is_none());
        common::pump(1100);
        cold.push(first_frame(&f, q)?);
    }
    loaded.sort_by(f64::total_cmp);
    cold.sort_by(f64::total_cmp);
    println!(
        "MEASURE input_to_first_pixels_median20_ms loaded={:.3} unloaded={:.3}",
        (loaded[9] + loaded[10]) / 2.,
        (cold[9] + cold[10]) / 2.
    );
    assert!(std::process::Command::new(std::env::current_exe()?)
        .arg("resources")
        .status()?
        .success());
    Ok(())
}
fn resources() -> Result<()> {
    let f = Fixture::new()?;
    f.root("/plain-source")?;
    common::pump(1500);
    let baseline = canopy::budget::measure()?;
    common::eval(&f.ui, "link.href='/plain';JSON.stringify(true)")?;
    let (x, y) = common::element_point(&f.ui, "link")?;
    common::xdo(&["mousemove", &x.to_string(), &y.to_string()])?;
    let ticks = cpu_ticks()?;
    let start = std::time::Instant::now();
    let child = std::cell::RefCell::new(
        std::process::Command::new("xdotool")
            .args(["click", "--repeat", "10", "--delay", "90", "2"])
            .spawn()?,
    );
    common::wait("ten middle clicks delivered", || {
        child.borrow_mut().try_wait().unwrap().is_some()
    })?;
    let input_ticks = cpu_ticks()? - ticks;
    let input_seconds = start.elapsed().as_secs_f64();
    common::wait("ten never seen pages committed", || {
        f.ui.pages.loaded_visits().len() == 10
    })?;
    common::pump(1000);
    let seconds = start.elapsed().as_secs_f64();
    let ticks = cpu_ticks()? - ticks;
    let used = canopy::budget::measure()?;
    println!("MEASURE ten_plain_pages_sampler_bytes baseline={baseline} total={used} delta={} input_seconds={input_seconds:.3} elapsed_seconds={seconds:.3} own_cpu_ticks={ticks} input_cpu_ticks={input_ticks}",used.saturating_sub(baseline));
    Ok(())
}
fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    std::fs::write(
        directory.path().join("user-dirs.dirs"),
        format!(
            "XDG_DOWNLOAD_DIR=\"{}/saved\"\n",
            directory.path().display()
        ),
    )?;
    std::env::set_var("XDG_CONFIG_HOME", directory.path());
    common::start(directory.path())?;
    if std::env::args().nth(1).as_deref() == Some("measurements") {
        return measurements();
    }
    if std::env::args().nth(1).as_deref() == Some("resources") {
        return resources();
    }
    if std::env::args().nth(1).as_deref() == Some("over-budget") {
        return over_budget_a_new_link_stays_unloaded();
    }
    type Case = (&'static str, fn() -> Result<()>);
    let cases: &[Case] = &[
        (
            "windows_share_three_background_load_slots",
            windows_share_three_background_load_slots,
        ),
        (
            "switching_elsewhere_keeps_an_adopted_background_load",
            switching_elsewhere_keeps_an_adopted_background_load,
        ),
        (
            "middle_click_loads_silently_and_opening_reuses_the_muted_page",
            middle_click_loads_silently_and_opening_reuses_the_muted_page,
        ),
        (
            "self_navigation_keeps_one_never_seen_visit",
            self_navigation_keeps_one_never_seen_visit,
        ),
        (
            "failed_and_empty_background_loads_leave_no_loaded_page",
            failed_and_empty_background_loads_leave_no_loaded_page,
        ),
        (
            "a_background_download_is_refused_but_opening_downloads",
            a_background_download_is_refused_but_opening_downloads,
        ),
        (
            "opening_an_uncommitted_background_page_adopts_its_request",
            opening_an_uncommitted_background_page_adopts_its_request,
        ),
        (
            "three_loads_run_and_newest_pending_starts_first",
            three_loads_run_and_newest_pending_starts_first,
        ),
        (
            "pressure_drops_oldest_never_seen_before_seen_and_reopening_fetches",
            pressure_drops_oldest_never_seen_before_seen_and_reopening_fetches,
        ),
        (
            "settling_destroys_never_seen_and_pending_loads",
            settling_destroys_never_seen_and_pending_loads,
        ),
        (
            "opening_pending_attachment_saves_once_with_one_request",
            opening_pending_attachment_saves_once_with_one_request,
        ),
        (
            "opening_pending_dropped_connection_reports_error_with_one_request",
            opening_pending_dropped_connection_reports_error_with_one_request,
        ),
        (
            "unanswered_loads_expire_and_the_next_link_loads",
            unanswered_loads_expire_and_the_next_link_loads,
        ),
        (
            "committed_documents_with_pending_images_free_their_slots",
            committed_documents_with_pending_images_free_their_slots,
        ),
        (
            "a_burst_admits_only_reserved_pages_and_drops_none",
            a_burst_admits_only_reserved_pages_and_drops_none,
        ),
    ];
    for (name, case) in cases {
        case()?;
        println!("PASS {name}");
    }
    assert!(std::process::Command::new(std::env::current_exe()?)
        .arg("over-budget")
        .env("CANOPY_MEMORY_BUDGET_MB", "1")
        .status()?
        .success());
    println!("PASS over_budget_a_new_link_stays_unloaded");
    Ok(())
}
