mod common;
#[path = "common/http.rs"]
mod http;

use std::{io::Write, net::TcpListener, thread};

use anyhow::Result;
use canopy::{address::web_url, browser::NavigationTarget, ui::Ui};
use gtk::prelude::*;
use serde_json::json;
use webkit2gtk::WebViewExt;

use common::*;

const OPENER: &str = r#"<!doctype html><title>Opener</title>
<button id="sized" style="font-size:24px" onclick="popup = window.open('/popup', 'auth', 'width=480,height=360')">Sized</button>
<button id="plain" style="font-size:24px" onclick="window.open('/child')">Plain</button>
<script>let popup; addEventListener('message', e => { document.title = 'Opener got ' + e.data; });</script>"#;
const POPUP: &str = r#"<!doctype html><title>Popup</title><p>Popup</p>
<script>
window.opener.postMessage('hello', '*');
addEventListener('message', e => { if (e.data === 'close') window.close(); });
</script>"#;
const CHILD: &str = "<!doctype html><title>Child</title><p>Child</p>";

fn popups(ui: &Ui) -> Vec<gtk::Window> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .filter(|window| window != &ui.window && window.is_visible())
        .filter(|window| window.title().as_deref() == Some("Popup"))
        .collect()
}

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = format!("http://{}", listener.local_addr()?);
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let Ok(request) = http::request(&mut stream) else {
                continue;
            };
            let line = String::from_utf8_lossy(&request);
            let path = line.split_whitespace().nth(1).unwrap_or("/");
            let body = match path {
                "/popup" => POPUP,
                "/child" => CHILD,
                _ => OPENER,
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });

    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages
        .selected_page()
        .navigate(web_url(&origin)?, NavigationTarget::Root)?;
    let view = ui.pages.selected_page().native;
    wait("opener loaded", || {
        view.title().as_deref() == Some("Opener") && !view.is_loading()
    })?;
    let before = ui.snapshot()?.visits.len();

    let (x, y) = element_point(&ui, "sized")?;
    click_at(x, y)?;
    wait("sized window.open shows a popup window", || {
        popups(&ui).len() == 1
    })?;
    wait("the popup reaches its opener", || {
        view.title().as_deref() == Some("Opener got hello")
    })?;
    let popup = popups(&ui).remove(0);
    assert_eq!(popup.transient_for().as_ref(), Some(&ui.window));
    let (width, height) = popup.size();
    assert_eq!(width, 480, "popup width");
    assert!(
        height > 360,
        "popup height {height} leaves room for the address line"
    );
    eval(&ui, "popup.postMessage('close', '*'); JSON.stringify(true)")?;
    wait("window.close in the popup closes its window", || {
        popups(&ui).is_empty()
    })?;
    assert_eq!(
        ui.snapshot()?.visits.len(),
        before,
        "a popup is not stored in the tree"
    );
    assert_eq!(eval(&ui, "JSON.stringify(popup.closed)")?, json!(true));
    println!("PASS a sized window.open opens a popup window that keeps window.opener, closes itself, and stays out of the tree.");

    let opener = ui
        .snapshot()?
        .visits
        .iter()
        .find(|visit| visit.url.path() == "/")
        .map(|visit| visit.id)
        .unwrap();
    let (x, y) = element_point(&ui, "plain")?;
    click_at(x, y)?;
    wait("plain window.open queues a waiting child", || {
        ui.snapshot().is_ok_and(|snapshot| {
            snapshot.visits.iter().any(|visit| {
                visit.url.path() == "/child"
                    && visit.parent == Some(opener)
                    && visit.opened_at.is_none()
            })
        })
    })?;
    pump(500);
    assert!(popups(&ui).is_empty());
    assert!(
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .all(|window| window == ui.window || !window.is_visible()),
        "a plain window.open opens no window"
    );
    assert_eq!(
        ui.pages.selected_page().native,
        view,
        "the opener stays selected"
    );
    println!("PASS a window.open without a size becomes a waiting child page, like a middle-clicked link.");
    Ok(())
}
