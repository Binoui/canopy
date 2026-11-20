mod common;
#[path = "common/http.rs"]
mod http;
use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{NavigationTarget, VisitId},
    pages::Go,
    ui::Ui,
};
use common::*;
use gtk::prelude::*;
use serde_json::json;
use std::time::Duration;
use std::{
    collections::BTreeMap,
    io::Write,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};
use url::Url;
use webkit2gtk::{SettingsExt, WebViewExt};

fn arrived(ui: &Ui, path: &str) -> Result<VisitId> {
    wait(path, || {
        ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == path))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    Ok(ui.snapshot()?.current.unwrap())
}
fn page_eval(ui: &Ui, script: &str) -> Result<serde_json::Value> {
    let output = std::rc::Rc::new(std::cell::RefCell::new(None));
    let received = output.clone();
    ui.pages.selected_page().native.evaluate_javascript(
        script,
        None,
        None,
        None::<&gio::Cancellable>,
        move |v| {
            use javascriptcore::ValueExt;
            *received.borrow_mut() = Some(v.map(|v| v.to_str().to_string()));
        },
    );
    wait("page script", || output.borrow().is_some())?;
    let text = output.borrow_mut().take().unwrap()?;
    Ok(serde_json::from_str(&text)?)
}
fn question(ui: &Ui) -> Result<()> {
    wait("return to a form result question", || {
        visible(ui, "#form-return")
    })
}
fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let server = Server::start()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages.go(Go::Address {
        url: server.url("/long")?,
        target: NavigationTarget::Root,
    })?;
    let root = arrived(&ui, "/long")?;
    page_eval(&ui,"const f=document.createElement('form');f.method='POST';f.action='/post';document.body.append(f);f.submit();JSON.stringify(true)")?;
    let post = arrived(&ui, "/post")?;
    ui.pages.go(Go::Address {
        url: server.url("/next")?,
        target: NavigationTarget::ChildOf(post),
    })?;
    let next = arrived(&ui, "/next")?;
    let posts = server.hits("POST /post");
    let visits = ui.snapshot()?.visits.len();
    let can_back = ui.pages.selected_page().native.can_go_back();
    let displayed = chrome(&ui, "[document.getElementById('search-pill').innerText, current(), document.querySelector('.node.pill')?.dataset.id]")?;
    let window_title = ui.window.title();
    ui.pages.go(Go::Parent)?;
    question(&ui)?;
    pump(200);
    if std::env::var_os("RESEND_SHOTS").is_some() {
        save_window(
            &ui,
            &format!("{}/.tmp/form-return-dark.png", env!("CARGO_MANIFEST_DIR")),
        )?;
        ui.chrome
            .evaluate("document.documentElement.classList.remove('dark')")?;
        pump(200);
        save_window(
            &ui,
            &format!("{}/.tmp/form-return-light.png", env!("CARGO_MANIFEST_DIR")),
        )?;
    }
    assert_eq!(chrome(&ui, "[document.getElementById('search-pill').innerText, current(), document.querySelector('.node.pill')?.dataset.id]")?, displayed);
    assert_eq!(ui.window.title(), window_title);
    assert_eq!(server.hits("POST /post"), posts);
    assert_eq!(ui.snapshot()?.current, Some(next));
    assert_eq!(
        chrome(&ui, "document.activeElement.id")?,
        json!("form-return-cancel")
    );
    println!("PASS Back holds the form result and focuses Cancel without sending or recording");
    key("Return")?;
    pump(200);
    assert!(!visible(&ui, "#form-return"));
    assert_eq!(ui.snapshot()?.current, Some(next));
    assert_eq!(server.hits("POST /post"), posts);
    assert_eq!(
        ui.pages.selected_page().native.uri().as_deref(),
        Some(server.url("/next")?.as_str())
    );
    assert_eq!(ui.pages.selected_page().native.can_go_back(), can_back);
    assert_eq!(ui.snapshot()?.visits.len(), visits);
    page_eval(&ui, "(()=>{const a=document.createElement('a');a.id='usable';a.href='/third';a.textContent='usable link';document.body.append(a);return JSON.stringify(true)})()")?;
    pump(100);
    let point=page_eval(&ui,"JSON.stringify((()=>{const r=document.getElementById('usable').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})())")?;
    let (x, y) = screen_point(
        &ui,
        &ui.pages.selected_page().native,
        point[0].as_f64().unwrap() as i32,
        point[1].as_f64().unwrap() as i32,
    )?;
    click_at(x, y)?;
    arrived(&ui, "/third")?;
    ui.pages.go(Go::Visit(next))?;
    arrived(&ui, "/next")?;
    ui.pages.go(Go::Parent)?;
    question(&ui)?;
    click_chrome(&ui, "#form-return-continue")?;
    arrived(&ui, "/post")?;
    assert_eq!(server.hits("POST /post"), posts);
    assert_eq!(ui.snapshot()?.visits.len(), visits + 1);
    println!("PASS Cancel preserves the visit, address and Back; a real link works; cached Continue sends nothing");
    ui.pages.selected_page().native.go_back();
    arrived(&ui, "/long")?;
    ui.pages.selected_page().native.go_forward();
    question(&ui)?;
    key("Escape")?;
    pump(100);
    assert_eq!(server.hits("POST /post"), posts);
    println!("PASS Forward to a form result asks and Escape cancels");
    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    click_at(700, 700)?;
    pump(100);
    assert!(!visible(&ui, "#form-return"));
    println!("PASS an older node asks and a click outside cancels");
    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    // The chrome remains actionable above the scrim.
    assert_eq!(ui.snapshot()?.current, Some(root));
    click_chrome(&ui, "#zoom-pin")?;
    wait("chrome input closes question", || {
        !visible(&ui, "#form-return")
    })?;
    wait("continuation tree", || visible(&ui, "#tree-sheet"))?;
    click_chrome(&ui, "[data-side=list]")?;
    click_chrome(&ui, &format!(".tree-row[data-id='{next}']"))?;
    assert_eq!(arrived(&ui, "/next")?, next);
    assert_eq!(server.hits("POST /post"), posts);
    println!("PASS a link in the chrome cancels the held decision and navigates");
    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    key("ctrl+l")?;
    type_text(server.url("/next")?.as_str())?;
    key("Return")?;
    arrived(&ui, "/next")?;
    assert!(!visible(&ui, "#form-return"));
    println!("PASS an address replaces the held decision without a POST");
    assert_eq!(server.hits("POST /post"), posts);
    webkit2gtk::WebViewExt::settings(&ui.pages.selected_page().native)
        .unwrap()
        .set_enable_page_cache(false);
    page_eval(&ui,"(()=>{const f=document.createElement('form');f.method='POST';f.action='/post';document.body.append(f);f.submit();return JSON.stringify(true)})()")?;
    let post = arrived(&ui, "/post")?;
    let sent = server.hits("POST /post");
    ui.pages.go(Go::Address {
        url: server.url("/next")?,
        target: NavigationTarget::ChildOf(post),
    })?;
    let next = arrived(&ui, "/next")?;
    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    click_chrome(&ui, "#form-return-continue")?;
    assert_eq!(arrived(&ui, "/post")?, post);
    assert_eq!(server.hits("POST /post"), sent + 1);
    println!(
        "PASS Continue on an uncached form result sends exactly one POST and reuses the visit"
    );
    ui.pages.go(Go::Visit(next))?;
    arrived(&ui, "/next")?;
    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    ui.pages.go(Go::Address {
        url: server.url("/long")?,
        target: NavigationTarget::Root,
    })?;
    let other = arrived(&ui, "/long")?;
    assert!(!visible(&ui, "#form-return"));
    ui.pages.go(Go::Visit(next))?;
    arrived(&ui, "/next")?;
    assert_eq!(server.hits("POST /post"), sent + 1);
    println!("PASS switching trees cancels the held decision and returning preserves the page");
    let hidden = ui.pages.selected_page().native.clone();
    ui.pages.go(Go::Visit(other))?;
    arrived(&ui, "/long")?;
    hidden.evaluate_javascript(
        "history.back()",
        None,
        None,
        None::<&gio::Cancellable>,
        |_| {},
    );
    pump(350);
    assert_eq!(server.hits("POST /post"), sent + 1);
    assert!(!visible(&ui, "#form-return"));
    ui.pages.go(Go::Visit(next))?;
    arrived(&ui, "/next")?;
    println!("PASS a hidden page refuses a return to a form result silently");
    ui.pages.go(Go::Visit(other))?;
    arrived(&ui, "/long")?;
    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    assert_eq!(ui.snapshot()?.current, Some(next));
    assert_eq!(
        page_eval(&ui, "JSON.stringify(location.pathname)")?,
        json!("/next")
    );
    assert_eq!(server.hits("POST /post"), sent + 1);
    click_chrome(&ui, "#form-return-cancel")?;
    pump(150);
    assert_eq!(arrived(&ui, "/next")?, next);
    ui.pages.go(Go::Visit(other))?;
    arrived(&ui, "/long")?;
    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    click_chrome(&ui, "#form-return-continue")?;
    assert_eq!(arrived(&ui, "/post")?, post);
    assert_eq!(server.hits("POST /post"), sent + 2);
    println!("PASS another tree's older form result shows its current page and asks; Cancel permits another click and Continue sends once");
    // Keep the remaining original assertions relative to this deliberate resend.
    let sent = sent + 1;
    ui.pages.go(Go::Visit(next))?;
    arrived(&ui, "/next")?;
    ui.pages.go(Go::Parent)?;
    question(&ui)?;
    key("alt+Left")?;
    pump(600);
    question(&ui)?;
    assert_eq!(ui.snapshot()?.current, Some(next));
    assert_eq!(server.hits("POST /post"), sent + 1);
    println!("PASS Back while asked asks again for the same entry without sending or skipping");
    for _ in 0..2 {
        key("shift+Tab")?;
        pump(100);
        assert_eq!(
            chrome(&ui, "!!document.activeElement.closest('.form-return-box')")?,
            json!(true)
        );
    }
    println!("PASS Shift+Tab keeps focus inside the question");
    click_chrome(&ui, "#search-pill")?;
    pump(200);
    type_text("abc")?;
    pump(150);
    assert_eq!(
        chrome(&ui, "document.getElementById('palette-input').value")?,
        json!("abc")
    );
    assert!(!visible(&ui, "#form-return"));
    key("Escape")?;
    println!("PASS typing after clicking the search pill reaches the palette");

    page_eval(&ui, "(()=>{const f=document.createElement('iframe');document.body.append(f);let n=0;window.rot=setInterval(()=>{f.src='/inner?'+(++n)},700);return JSON.stringify(true)})()")?;
    ui.pages.go(Go::Parent)?;
    question(&ui)?;
    pump(2200);
    assert!(
        visible(&ui, "#form-return"),
        "iframe navigation must leave question open"
    );
    assert_eq!(server.hits("POST /post"), sent + 1);
    click_chrome(&ui, "#form-return-cancel")?;
    page_eval(&ui, "JSON.stringify(clearInterval(window.rot) ?? true)")?;
    println!("PASS iframe navigation every 700 ms keeps the question open until answered");

    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    click_chrome(&ui, "#form-return-continue")?;
    arrived(&ui, "/post")?;
    let sent = sent + 1;
    page_eval(&ui, "(()=>{const a=document.createElement('a');a.id='branch-child';a.href='/third';a.textContent='child';document.body.append(a);return JSON.stringify(true)})()")?;
    pump(150);
    let point = page_eval(&ui, "JSON.stringify((()=>{const r=document.getElementById('branch-child').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})())")?;
    let (x, y) = screen_point(
        &ui,
        &ui.pages.selected_page().native,
        point[0].as_f64().unwrap() as i32,
        point[1].as_f64().unwrap() as i32,
    )?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    pump(700);
    ui.pages.go(Go::Visit(next))?;
    arrived(&ui, "/next")?;
    key("ctrl+Tab")?;
    arrived(&ui, "/third")?;
    ui.pages.go(Go::Parent)?;
    question(&ui)?;
    assert_eq!(ui.snapshot()?.current, Some(next));
    assert_eq!(server.hits("POST /post"), sent + 1);
    click_chrome(&ui, "#form-return-cancel")?;
    arrived(&ui, "/next")?;
    println!("PASS Back from a middle-clicked branch asks at its parent's older form result");

    ui.pages.go(Go::Visit(post))?;
    question(&ui)?;
    ui.pages.reload()?;
    arrived(&ui, "/next")?;
    assert!(!visible(&ui, "#form-return"));
    assert_eq!(server.hits("POST /post"), sent + 1);
    println!("PASS Canopy Reload cancels a pending question without sending a form");
    let fixed = ui
        .pages
        .selected_page()
        .native
        .parent()
        .unwrap()
        .downcast::<gtk::Fixed>()
        .unwrap();
    ui.pages.release_selected_page();
    wait("checkpoint captured", || {
        ui.pages.checkpoint_size().is_some()
    })?;
    let gets = server.hits("GET /post");
    assert!(!ui.pages.revive_selected_page(&fixed, post)?);
    arrived(&ui, "/post")?;
    assert!(!visible(&ui, "#form-return"));
    assert_eq!(server.hits("GET /post"), gets + 1);
    assert_eq!(server.hits("POST /post"), sent + 1);
    println!("PASS a checkpoint form result is fetched by GET without asking");
    // Build a fresh live POST after the GET revival for settling.
    let before = server.hits("POST /post");
    page_eval(&ui,"(()=>{const f=document.createElement('form');f.method='POST';f.action='/post';document.body.append(f);f.submit();return JSON.stringify(true)})()")?;
    wait("fresh POST received", || server.hits("POST /post") > before)?;
    wait("fresh POST committed", || {
        ui.snapshot().is_ok_and(|s| s.current != Some(post))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    let fresh = arrived(&ui, "/post")?;
    ui.pages.selected_page().native.reload();
    question(&ui)?;
    key("Escape")?;
    let sent = server.hits("POST /post") - 1;
    ui.pages.go(Go::Address {
        url: server.url("/next")?,
        target: NavigationTarget::ChildOf(fresh),
    })?;
    arrived(&ui, "/next")?;
    ui.pages.go(Go::Visit(fresh))?;
    question(&ui)?;
    let tree = ui.chrome.current_tree().unwrap();
    ui.pages.place_tree(tree, canopy::browser::Move::History)?;
    pump(700);
    assert!(!visible(&ui, "#form-return"));
    assert_eq!(server.hits("POST /post"), sent + 1);
    println!("PASS settling the tree cancels the held decision without a POST");
    ui.window.close();
    Ok(())
}
struct Server {
    origin: String,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
}

impl Server {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(BTreeMap::new()));
        let counted = hits.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let counted = counted.clone();
                thread::spawn(move || {
                    let Ok(request) = http::request(&mut stream) else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&request);
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let method = request.split_whitespace().next().unwrap_or("GET");
                    *counted
                        .lock()
                        .unwrap()
                        .entry(format!("{method} {path}"))
                        .or_insert(0) += 1;
                    *counted.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
                    if path == "/slow" {
                        thread::sleep(std::time::Duration::from_millis(1000));
                    }
                    if path == "/slowimg" {
                        thread::sleep(Duration::from_secs(3));
                    }
                    let (status, body) = match path.as_str() {
                    "/post" => ("200 OK", "<!doctype html><title>Post result</title>result"),
                    "/heavy" => ("200 OK", "<!doctype html><title>Heavy</title><script>addEventListener('beforeunload', () => { const t=Date.now(); while(Date.now()-t<1000) {} });</script>"),
                    "/imgpage" => ("200 OK", "<!doctype html><title>Image page</title><img src='/slowimg'>"),
                    "/slowimg" => ("200 OK", ""),
                    "/spa" => ("200 OK", "<!doctype html><title>SPA home</title><p>SPA</p>"),
                    "/frame" => ("200 OK", "<!doctype html><title>Frame page</title><iframe src=\"/inner\"></iframe>"),
                    "/inner" => ("200 OK", "<!doctype html><title>Inner</title><p>Inner</p>"),
                    "/slow" => ("200 OK", "<!doctype html><title>Slow</title><p>Slow</p>"),
                    "/long" => ("200 OK", "<!doctype html><title>Long page</title><body style=\"margin:0\"><div style=\"height:6000px\">Long page</div>"),
                    "/third" => ("200 OK", "<!doctype html><title>Third page</title><p>Third</p>"),
                    "/next" | "/mid" => ("200 OK", "<!doctype html><title>Next page</title><p>Next page</p>"),
                    _ => ("404 Not Found", ""),
                };
                    let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                });
            }
        });
        Ok(Self { origin, hits })
    }

    fn url(&self, path: &str) -> Result<Url> {
        web_url(&format!("{}{path}", self.origin))
    }

    fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}
