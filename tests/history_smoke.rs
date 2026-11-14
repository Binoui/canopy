//! Returning to a page: through WebKit's history while it still has the page, so scroll and
//! live state survive; from the network otherwise, scrolled back to where it was left.

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
use webkit2gtk::WebViewExt;

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
    let long = arrived(&ui, "/long")?;
    eval(
        &ui,
        "window.marker = 'kept'; scrollTo(0, 1800); JSON.stringify(scrollY)",
    )?;
    pump(600);
    ui.engine
        .navigate(server.url("/next")?, NavigationTarget::ChildOf(long))?;
    let next = arrived(&ui, "/next")?;

    ui.engine.back()?;
    assert_eq!(arrived(&ui, "/long")?, long);
    assert_eq!(ui.snapshot()?.visits.len(), 2, "going back adds no visit");
    assert_eq!(
        server.hits("/long"),
        1,
        "going back does not fetch the page again"
    );
    wait("scroll kept on Back", || {
        eval(&ui, "JSON.stringify(scrollY)").is_ok_and(|y| y == json!(1800))
    })?;
    assert_eq!(
        eval(&ui, "JSON.stringify(window.marker ?? null)")?,
        json!("kept"),
        "the page comes back from WebKit's page cache with its script state"
    );
    println!("PASS Back returns through WebKit's history: no fetch, no new visit, scroll and script state kept.");

    ui.engine.restore(next)?;
    assert_eq!(arrived(&ui, "/next")?, next);
    assert_eq!(server.hits("/next"), 1);
    assert_eq!(ui.snapshot()?.visits.len(), 2);
    println!("PASS opening the page just left from the tree goes forward in WebKit's history.");

    ui.engine
        .navigate(server.url("/long#end")?, NavigationTarget::ChildOf(next))?;
    wait("fragment child", || {
        ui.snapshot().unwrap().visits.len() == 3
            && ui
                .snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.url.fragment() == Some("end"))
            && !ui.engine.native.is_loading()
    })?;
    let fragment = ui.snapshot()?.current.unwrap();
    ui.engine.navigate(
        server.url("/long#top")?,
        NavigationTarget::ChildOf(fragment),
    )?;
    wait("second fragment child", || {
        ui.snapshot().unwrap().visits.len() == 4 && !ui.engine.native.is_loading()
    })?;
    let hits = server.hits("/long");
    ui.engine.back()?;
    wait("same-document Back", || {
        ui.snapshot().unwrap().current == Some(fragment)
            && ui
                .engine
                .native
                .uri()
                .is_some_and(|uri| uri.ends_with("#end"))
            && !ui.engine.native.is_loading()
    })?;
    pump(300);
    assert_eq!(ui.snapshot()?.current, Some(fragment));
    assert_eq!(ui.snapshot()?.visits.len(), 4);
    assert_eq!(
        server.hits("/long"),
        hits,
        "a same-document return fetches nothing"
    );
    println!("PASS Back between two fragments of one page stays in the document and keeps the tree in step.");

    ui.window.close();
    drop(ui);
    pump(100);
    let reopened = Ui::new(directory.path())?;
    reopened.window.show_all();
    ready(&reopened)?;
    pump(500);
    wait("startup settled", || !reopened.engine.native.is_loading())?;
    let before = server.hits("/long");
    reopened.engine.restore(long)?;
    assert_eq!(arrived(&reopened, "/long")?, long);
    assert_eq!(
        server.hits("/long"),
        before + 1,
        "a cold visit loads from the network"
    );
    wait("scroll restored on a cold visit", || {
        eval(&reopened, "JSON.stringify(scrollY)").is_ok_and(|y| y == json!(1800))
    })?;
    println!(
        "PASS after a restart, a page reopened from the tree scrolls back to where it was left."
    );

    eval(&reopened, "scrollTo(0, 2400); JSON.stringify(scrollY)")?;
    pump(600);
    reopened.engine.reload()?;
    wait("reload finished", || {
        server.hits("/long") == before + 2 && !reopened.engine.native.is_loading()
    })?;
    wait("scroll kept on reload", || {
        eval(&reopened, "JSON.stringify(scrollY)").is_ok_and(|y| y == json!(2400))
    })?;
    assert_eq!(reopened.snapshot()?.visits.len(), 4);
    println!("PASS reloading keeps the latest scroll position.");
    reopened.window.close();
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
                let (status, body) = match path.as_str() {
                    "/long" => ("200 OK", "<!doctype html><title>Long page</title><body style=\"margin:0\"><div style=\"height:6000px\">Long page</div>"),
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
