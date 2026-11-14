use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
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
use webkit2gtk::{BackForwardListExt, WebViewExt};

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
    let before = server.hits("/next");
    ui.engine.restore(b)?;
    assert_eq!(arrived(&ui, "/next")?, b);
    assert_eq!(
        server.hits("/next"),
        before + 1,
        "a removed item must be fetched, never traversed"
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
    ui.window.close();
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
