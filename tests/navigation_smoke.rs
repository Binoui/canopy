use std::{
    cell::Cell,
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{NavigationTarget, VisitId},
    ui::Ui,
};
use gtk::prelude::*;
use serde_json::json;
use url::Url;
use webkit2gtk::{BackForwardListExt, BackForwardListItemExt, WebViewExt};

mod common;
use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let server = Server::start()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.engine
        .navigate(server.url("/long")?, NavigationTarget::Root)?;
    let a = arrived(&ui, "/long")?;
    eval(&ui, "window.marker = 'cached'; JSON.stringify(true)")?;
    ui.engine
        .navigate(server.url("/next")?, NavigationTarget::ChildOf(a))?;
    let b = arrived(&ui, "/next")?;
    ui.engine.native.go_back();
    assert_eq!(arrived(&ui, "/long")?, a);
    assert_eq!(server.hits("/long"), 1);
    assert_eq!(eval(&ui, "JSON.stringify(window.marker)")?, json!("cached"));
    ui.engine.native.go_forward();
    assert_eq!(arrived(&ui, "/next")?, b);
    ui.engine.native.go_back();
    assert_eq!(arrived(&ui, "/long")?, a);
    println!(
        "PASS native Back and Forward select their actual item, including page-cache restores."
    );
    if let Ok(case) = std::env::var("NAVIGATION_CASE") {
        regression(&case, &ui, &server)?;
        ui.window.close();
        return Ok(());
    }
    let count = ui.snapshot()?.visits.len();
    ui.engine
        .navigate(server.url("/third")?, NavigationTarget::ChildOf(a))?;
    let c = arrived(&ui, "/third")?;
    assert!(ui
        .engine
        .native
        .back_forward_list()
        .unwrap()
        .forward_list_with_limit(20)
        .is_empty());
    let fork_item = ui
        .engine
        .native
        .back_forward_list()
        .unwrap()
        .current_item()
        .unwrap();
    let before = server.hits("/next");
    ui.engine.restore(b)?;
    assert_eq!(arrived(&ui, "/next")?, b);
    assert_eq!(
        server.hits("/next"),
        before + 1,
        "a removed item must be fetched, never traversed"
    );
    let list = ui.engine.native.back_forward_list().unwrap();
    assert_eq!(
        list.back_item(),
        Some(fork_item),
        "stored restore must append after the fork"
    );
    assert_eq!(
        list.current_item().unwrap().uri().as_deref(),
        Some(server.url("/next")?.as_str())
    );
    assert_eq!(ui.snapshot()?.visits.len(), count + 1);
    assert_ne!(b, c);
    println!("PASS a fork removes the forward suffix; opening its stored visit fetches it again.");
    ui.engine.restore(a)?;
    assert_eq!(arrived(&ui, "/long")?, a);
    let count = ui.snapshot()?.visits.len();
    eval(&ui, "location.hash = 'hash'; JSON.stringify(true)")?;
    pump(250);
    eval(
        &ui,
        "history.replaceState({}, '', '#replaced'); JSON.stringify(true)",
    )?;
    pump(250);
    eval(
        &ui,
        "history.pushState({}, '', '#pushed'); JSON.stringify(true)",
    )?;
    pump(250);
    ui.engine.native.go_back();
    wait("pushState Back", || {
        ui.engine
            .native
            .uri()
            .is_some_and(|u| u.ends_with("#replaced"))
    })?;
    pump(250);
    assert_eq!(ui.snapshot()?.visits.len(), count);
    assert_eq!(ui.snapshot()?.current, Some(a));
    ui.engine.native.go_back();
    wait("same-document native Back", || {
        ui.engine.native.uri().is_some_and(|u| u.ends_with("/long"))
    })?;
    pump(300);
    ui.engine.native.go_forward();
    wait("same-document native Forward", || {
        ui.engine
            .native
            .uri()
            .is_some_and(|u| u.ends_with("#replaced"))
    })?;
    assert_eq!(ui.snapshot()?.current, Some(a));
    println!(
        "PASS hash, pushState and replaceState entries alias the visit through native traversal."
    );
    regression("fragment", &ui, &server)?;
    ui.engine.restore(a)?;
    assert_eq!(arrived(&ui, "/long")?, a);
    let count = ui.snapshot()?.visits.len();
    ui.engine
        .navigate(server.url("/slow")?, NavigationTarget::ChildOf(a))?;
    wait("slow attempt Started", || {
        server.hits("/slow") > 0 && ui.engine.native.is_loading()
    })?;
    ui.engine
        .navigate(server.url("/third")?, NavigationTarget::ChildOf(a))?;
    ui.engine
        .navigate(server.url("/third")?, NavigationTarget::ChildOf(a))?;
    arrived(&ui, "/third")?;
    pump(600);
    assert_eq!(ui.snapshot()?.visits.len(), count + 1);
    assert!(ui
        .snapshot()?
        .visits
        .iter()
        .all(|visit| visit.url.path() != "/slow"));
    println!("PASS replacing an active load drains its cancellation before issuing the latest equal-URL attempt.");
    for case in ["jank", "title", "adopt", "replace", "iframe"] {
        regression(case, &ui, &server)?;
    }
    ui.window.close();
    Ok(())
}

fn regression(case: &str, ui: &Rc<Ui>, server: &Server) -> Result<()> {
    ui.engine
        .navigate(server.url("/long")?, NavigationTarget::Root)?;
    let a = arrived(ui, "/long")?;
    match case {
        "fragment" => {
            ui.engine
                .navigate(server.url("/long#section")?, NavigationTarget::ChildOf(a))?;
            wait("explicit fragment visit", || {
                ui.snapshot().unwrap().current != Some(a)
            })?;
            let fragment = ui.snapshot()?.current.unwrap();
            let count = ui.snapshot()?.visits.len();
            ui.engine.native.go_back();
            wait("fragment back URI", || {
                ui.engine.native.uri().is_some_and(|u| u.ends_with("/long"))
            })?;
            pump(300);
            assert_eq!(ui.snapshot()?.current, Some(a));
            ui.engine.native.go_forward();
            wait("fragment forward URI", || {
                ui.engine
                    .native
                    .uri()
                    .is_some_and(|u| u.ends_with("#section"))
            })?;
            pump(300);
            assert_eq!(ui.snapshot()?.current, Some(fragment));
            assert_eq!(ui.snapshot()?.visits.len(), count);
        }
        "jank" => {
            let started = Rc::new(Cell::new(false));
            let observed = started.clone();
            let handler = ui.engine.native.connect_load_changed(move |_, event| {
                if event == webkit2gtk::LoadEvent::Started {
                    observed.set(true);
                }
            });
            eval(ui, "setTimeout(() => { const t = Date.now(); while (Date.now() - t < 1500) {} }, 0); JSON.stringify(true)")?;
            pump(50);
            ui.engine
                .navigate(server.url("/next")?, NavigationTarget::ChildOf(a))?;
            pump(300);
            assert!(ui.engine.native.is_loading());
            assert!(!started.get(), "replacement must precede Started");
            ui.engine
                .navigate(server.url("/third")?, NavigationTarget::Root)?;
            arrived(ui, "/third")?;
            assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
            ui.engine.native.disconnect(handler);
            ui.engine
                .navigate(server.url("/next")?, NavigationTarget::Root)?;
            arrived(ui, "/next")?;
        }
        "title" => {
            ui.engine
                .navigate(server.url("/spa")?, NavigationTarget::Root)?;
            let visit = arrived(ui, "/spa")?;
            eval(
                ui,
                "history.pushState({}, '', '/spa/route'); JSON.stringify(true)",
            )?;
            pump(400);
            eval(ui, "document.title = 'Route title'; JSON.stringify(true)")?;
            wait("route title delivered", || {
                ui.engine.native.title().as_deref() == Some("Route title")
            })?;
            pump(200);
            assert_eq!(
                ui.snapshot()?
                    .visits
                    .iter()
                    .find(|v| v.id == visit)
                    .unwrap()
                    .title,
                "SPA home"
            );
        }
        "adopt" => {
            let started = Rc::new(Cell::new(false));
            let observed = started.clone();
            let handler = ui.engine.native.connect_load_changed(move |_, event| {
                if event == webkit2gtk::LoadEvent::Started {
                    observed.set(true);
                }
            });
            let fired = Rc::new(Cell::new(false));
            let observed = fired.clone();
            let weak = Rc::downgrade(ui);
            let target = server.url("/next")?;
            let policy = ui
                .engine
                .native
                .connect_decide_policy(move |_, decision, kind| {
                    use webkit2gtk::{
                        NavigationPolicyDecision, NavigationPolicyDecisionExt, PolicyDecisionType,
                        URIRequestExt,
                    };
                    if kind != PolicyDecisionType::NavigationAction || observed.get() {
                        return false;
                    }
                    let Some(navigation) = decision.downcast_ref::<NavigationPolicyDecision>()
                    else {
                        return false;
                    };
                    let Some(action) = navigation.navigation_action() else {
                        return false;
                    };
                    if action
                        .request()
                        .and_then(|r| r.uri())
                        .is_some_and(|u| u.ends_with("/third"))
                    {
                        observed.set(true);
                        if let Some(ui) = weak.upgrade() {
                            started.set(false);
                            ui.engine
                                .navigate(target.clone(), NavigationTarget::Root)
                                .unwrap();
                            let started = started.clone();
                            let deadline = Instant::now() + Duration::from_secs(5);
                            // Delay application idles while WebKit IPC delivers the earlier Started.
                            let priority = unsafe { glib::translate::from_glib(120) };
                            glib::idle_add_local_full(priority, move || {
                                if started.get() || Instant::now() > deadline {
                                    glib::ControlFlow::Break
                                } else {
                                    thread::sleep(Duration::from_micros(200));
                                    glib::ControlFlow::Continue
                                }
                            });
                        }
                    }
                    false
                });
            eval(ui, "location.href = '/third'; JSON.stringify(true)")?;
            arrived(ui, "/next")?;
            assert!(fired.get(), "policy must schedule the user request");
            assert!(
                ui.snapshot()?.current_visit().unwrap().parent.is_none(),
                "user requested a new root"
            );
            ui.engine.native.disconnect(handler);
            ui.engine.native.disconnect(policy);
        }
        "replace" => {
            eval(
                ui,
                "history.pushState({}, '', '/long#x'); JSON.stringify(true)",
            )?;
            pump(300);
            eval(ui, "location.replace('/next'); JSON.stringify(true)")?;
            arrived(ui, "/next")?;
            let count = ui.snapshot()?.visits.len();
            eval(ui, "history.back(); JSON.stringify(true)")?;
            arrived(ui, "/long")?;
            assert_eq!(
                ui.snapshot()?.current,
                Some(a),
                "back must restore the original visit"
            );
            assert_eq!(ui.snapshot()?.visits.len(), count);
        }
        "iframe" => {
            ui.engine
                .navigate(server.url("/frame")?, NavigationTarget::Root)?;
            let visit = arrived(ui, "/frame")?;
            let before = server.hits("/inner");
            assert!(before > 0);
            eval(ui, "document.querySelector('iframe').contentWindow.location.reload(); JSON.stringify(true)")?;
            wait("iframe fetched again", || server.hits("/inner") > before)?;
            pump(500);
            assert!(
                ui.engine.remember_selection().is_ok(),
                "iframe reload must retain document writes"
            );
            eval(ui, "location.href = '/next'; JSON.stringify(true)")?;
            let next = arrived(ui, "/next")?;
            assert_ne!(next, visit);
            assert_eq!(
                ui.snapshot()?
                    .visits
                    .iter()
                    .find(|v| v.id == visit)
                    .unwrap()
                    .url
                    .path(),
                "/frame"
            );
        }
        _ => anyhow::bail!("unknown regression {case}"),
    }
    println!("PASS regression {case}");
    Ok(())
}

fn arrived(ui: &Ui, path: &str) -> Result<VisitId> {
    wait(&format!("{path} current and loaded"), || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|visit| visit.url.path() == path && !visit.title.is_empty())
            && ui
                .engine
                .native
                .uri()
                .is_some_and(|uri| uri.ends_with(path))
            && !ui.engine.native.is_loading()
    })?;
    Ok(ui.snapshot()?.current.unwrap())
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
                let mut request = [0; 4096];
                let Ok(size) = stream.read(&mut request) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&request[..size]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                *counted.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
                if path == "/slow" {
                    thread::sleep(std::time::Duration::from_millis(500));
                }
                let (status, body) = match path.as_str() {
                    "/spa" => ("200 OK", "<!doctype html><title>SPA home</title><p>SPA</p>"),
                    "/frame" => ("200 OK", "<!doctype html><title>Frame page</title><iframe src=\"/inner\"></iframe>"),
                    "/inner" => ("200 OK", "<!doctype html><title>Inner</title><p>Inner</p>"),
                    "/slow" => ("200 OK", "<!doctype html><title>Slow</title><p>Slow</p>"),
                    "/long" => ("200 OK", "<!doctype html><title>Long page</title><body style=\"margin:0\"><div style=\"height:6000px\">Long page</div>"),
                    "/third" => ("200 OK", "<!doctype html><title>Third page</title><p>Third</p>"),
                    "/next" => ("200 OK", "<!doctype html><title>Next page</title><p>Next page</p>"),
                    _ => ("404 Not Found", ""),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
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
