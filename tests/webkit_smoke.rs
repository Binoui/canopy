use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use canopy::{
    address::web_url,
    browser::{NavigationTarget, Snapshot, VisitState},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

mod common;

use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let fixture = Fixture::start()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    pump(500);

    key("ctrl+k")?;
    let entry = named(&ui.window, "palette-entry")
        .downcast::<gtk::Entry>()
        .unwrap();
    wait("palette focus", || entry.has_focus())?;
    type_text(&fixture.url("/a"))?;
    key("Return")?;
    loaded(&ui, "/a", 1)?;
    let a = ui.snapshot()?.current.unwrap();
    assert!(!named(&ui.window, "palette").is_visible());
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    link(&ui, "b")?;
    loaded(&ui, "/b", 2)?;
    let b = ui.snapshot()?.current.unwrap();
    println!("PASS the palette plants a tree from a typed address; a real link adds B under A.");

    open_strip(&ui)?;
    click(&ui, &node(&ui, a))?;
    loaded(&ui, "/a", 2)?;
    link(&ui, "c")?;
    loaded(&ui, "/c", 3)?;
    let c = ui.snapshot()?.current.unwrap();
    let snapshot = ui.snapshot()?;
    assert_eq!(
        snapshot
            .visits
            .iter()
            .filter(|v| v.parent == Some(a))
            .map(|v| v.id)
            .collect::<Vec<_>>(),
        vec![b, c]
    );
    println!(
        "PASS hovering opens the strip, clicking node A restores it, and C becomes B's sibling."
    );

    leave_strip(&ui)?;
    key("F6")?;
    wait("current node focus", || node(&ui, c).has_focus())?;
    key("Left")?;
    wait("parent node focus", || node(&ui, a).has_focus())?;
    key("Return")?;
    loaded(&ui, "/a", 3)?;
    link(&ui, "c")?;
    loaded(&ui, "/c", 4)?;
    let c = ui.snapshot()?.current.unwrap();
    println!(
        "PASS F6 focuses the current node, Left walks the trunk and Enter restores the parent."
    );

    ui.engine.native.grab_focus();
    pump(100);
    key("ctrl+l")?;
    wait("Ctrl+L from the focused web view", || entry.has_focus())?;
    assert_eq!(entry.text().as_str(), fixture.url("/c"));
    assert_eq!(
        entry.selection_bounds(),
        Some((0, fixture.url("/c").chars().count() as i32))
    );
    key("Escape")?;
    wait("palette closed", || {
        !named(&ui.window, "palette").is_visible()
    })?;
    assert!(ui.engine.native.has_focus());
    println!("PASS Ctrl+L opens the palette on the current address from the focused web view; Escape returns to the page.");

    let (x, y) = element_point(&ui, "q")?;
    click_at(x, y)?;
    type_text("alpha")?;
    key("ctrl+k")?;
    wait("palette over a focused input", || entry.has_focus())?;
    type_text("zzz")?;
    key("Escape")?;
    type_text("omega")?;
    assert_eq!(
        eval(&ui, "JSON.stringify(document.getElementById('q').value)")?,
        "alphaomega"
    );
    println!("PASS Ctrl+K then Escape gives keyboard focus back to the page input that had it.");

    eval(
        &ui,
        "window.clicks = 0; addEventListener('click', () => clicks++); JSON.stringify(true)",
    )?;
    let clicks = |ui: &Ui| {
        eval(ui, "JSON.stringify(window.clicks)")
            .unwrap()
            .as_i64()
            .unwrap()
    };
    open_strip(&ui)?;
    let (x, y) = page_point(&ui, 900, 42 + 120 + 40)?;
    click_at(x, y)?;
    assert_eq!(
        clicks(&ui),
        1,
        "a click below the open strip reaches the page"
    );
    open_strip(&ui)?;
    move_to_widget(&ui, &node(&ui, c))?;
    let card = named(&ui.window, "card");
    wait("hover card", || card.is_visible())?;
    pump(150);
    move_to_widget(&ui, &card)?;
    pump(500);
    assert!(
        card.is_visible() && strip_height(&ui) == 120,
        "the card holds the strip open"
    );
    let bounds = card.allocation();
    let (x, y) = screen_point(&ui, &card, bounds.width() + 180, bounds.height() / 2)?;
    click_at(x, y)?;
    assert_eq!(clicks(&ui), 2, "a click beside the card reaches the page");
    open_strip(&ui)?;
    let (away_x, away_y) = page_point(&ui, 900, 600)?;
    move_to(away_x, away_y)?;
    pump(430);
    assert_eq!(
        strip_height(&ui),
        120,
        "the closing strip keeps its open allocation"
    );
    let (x, y) = page_point(&ui, 900, 140)?;
    click_at(x, y)?;
    assert_eq!(
        clicks(&ui),
        2,
        "the closing band swallows a click during the close"
    );
    move_to(away_x, away_y)?;
    pump(900);
    assert_eq!(strip_height(&ui), 52);
    click_at(x, y)?;
    assert_eq!(
        clicks(&ui),
        3,
        "the same point reaches the page once the strip is closed"
    );
    println!("PASS page clicks pass below the open strip and beside the card; the 220 ms closing band is swallowed, then the page is reachable again.");

    save_window(&ui, "/tmp/canopy-chrome-webkit.png")?;

    ui.engine.reload()?;
    wait("reload start", || ui.engine.native.is_loading())?;
    loaded(&ui, "/c", 4)?;
    assert_eq!(ui.snapshot()?.current, Some(c));
    println!("PASS reloading adds no node.");

    ui.engine.navigate(
        web_url(&fixture.url("/redirect"))?,
        NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap()),
    )?;
    loaded(&ui, "/final", 5)?;
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url.path(), "/final");
    println!("PASS an HTTP 302 is recorded once, under its final URL.");

    let before_failure = ui.snapshot()?;
    let unused_port = TcpListener::bind("127.0.0.1:0")?;
    let port = unused_port.local_addr()?.port();
    drop(unused_port);
    let failed_url = format!("http://127.0.0.1:{port}/unreachable");
    ui.engine.navigate(
        web_url(&failed_url)?,
        NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap()),
    )?;
    wait("connection failure", || {
        toast_text(&ui).is_some_and(|text| text.contains("Could not load"))
            && !ui.engine.native.is_loading()
    })?;
    pump(150);
    assert_eq!(ui.snapshot()?, before_failure);
    let memory = popover_of(&ui, "memory-menu");
    assert!(label_texts(&named(&memory, "memory-errors"))
        .iter()
        .any(|text| text.contains("Could not load")));
    println!("PASS a network failure shows a toast, stays listed in the Memory menu, and changes nothing.");

    ui.engine.restore(a)?;
    loaded(&ui, "/a", 5)?;
    trusted_click(&ui, "blank")?;
    wait("target=_blank queued", || {
        ui.snapshot().unwrap().visits.len() == 6
    })?;
    let pending = ui
        .snapshot()?
        .visits
        .into_iter()
        .find(|visit| visit.state == VisitState::Queued)
        .context("queued target=_blank")?;
    assert_eq!(pending.parent, Some(a));
    assert_eq!(ui.snapshot()?.current, Some(a));
    open_strip(&ui)?;
    move_to_widget(&ui, &node(&ui, pending.id))?;
    let card = named(&ui.window, "card");
    wait("card of the queued page", || card.is_visible())?;
    pump(150);
    click(&ui, &named(&ui.window, "card-branch"))?;
    loaded(&ui, "/d", 6)?;
    assert_eq!(ui.snapshot()?.current, Some(pending.id));
    wait("palette anchored after the commit", || {
        named(&ui.window, "palette").is_visible() && entry.has_focus()
    })?;
    assert!(label_texts(&named(&ui.window, "palette"))
        .iter()
        .any(|text| text == "Link opened in another window"));
    key("Escape")?;
    println!("PASS a target=_blank link waits in the tree; New branch here activates it without a duplicate, then anchors the palette on it.");

    ui.engine.navigate(
        web_url(&fixture.url("/b"))?,
        NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap()),
    )?;
    ui.engine
        .navigate(web_url(&fixture.url("/c"))?, NavigationTarget::Root)?;
    loaded(&ui, "/c", 7)?;
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    println!("PASS close requests coalesce; the last new tree stays a root.");

    pump(300);
    click(&ui, &named(&ui.window, &format!("tab-{a}")))?;
    loaded(&ui, "/d", 7)?;
    assert_eq!(ui.snapshot()?.current, Some(pending.id));
    println!("PASS a tree tab restores its tree's last page.");

    for n in 1..=8 {
        ui.engine.navigate(
            web_url(&fixture.url(&format!("/topic-{n}")))?,
            NavigationTarget::Root,
        )?;
        loaded(&ui, &format!("/topic-{n}"), 7 + n)?;
    }
    pump(400);
    let roots: Vec<_> = ui
        .snapshot()?
        .visits
        .into_iter()
        .filter(|v| v.parent.is_none())
        .collect();
    let hidden: Vec<String> = roots
        .iter()
        .filter(|root| !named(&ui.window, &format!("tab-{}", root.id)).is_visible())
        .map(|root| root.label().to_owned())
        .collect();
    assert!(!hidden.is_empty(), "ten trees overflow a 1240 px tab row");
    assert!(named(&ui.window, &format!("tab-{}", roots.last().unwrap().id)).is_visible());
    click(&ui, &named(&ui.window, "tab-overflow"))?;
    let list = popover_of(&ui, "tab-overflow");
    wait("overflow list", || list.is_visible())?;
    let mut rows = Vec::new();
    find_all(list.upcast_ref(), "overflow-tree", &mut rows);
    let listed: Vec<String> = rows.iter().map(|row| label_texts(row)[0].clone()).collect();
    assert_eq!(listed, hidden);
    let first_hidden = rows[0].clone();
    click(&ui, &first_hidden)?;
    let hidden_root = roots
        .iter()
        .find(|root| root.label() == hidden[0])
        .unwrap()
        .id;
    wait("hidden tree selected", || {
        ui.snapshot().unwrap().current == Some(hidden_root) && !ui.engine.native.is_loading()
    })?;
    println!(
        "PASS the overflow arrow lists exactly the hidden trees, in creation order, and opens one."
    );

    let expected: Snapshot = ui.snapshot()?;
    ui.window.close();
    drop(ui);
    pump(100);
    let reopened = Ui::new(directory.path())?;
    reopened.window.show_all();
    assert_eq!(reopened.snapshot()?, expected);
    reopened.resume()?;
    wait("restart reload", || {
        !reopened.engine.native.is_loading()
            && reopened
                .engine
                .native
                .uri()
                .is_some_and(|uri| uri.starts_with("http"))
    })?;
    assert_eq!(reopened.snapshot()?, expected);
    reopened.window.close();
    println!("PASS reopening SQLite reloads the URL with titles and branches unchanged.");
    Ok(())
}

fn loaded(ui: &Ui, path: &str, count: usize) -> Result<()> {
    wait(&format!("{path} loaded with {count} visits"), || {
        let snapshot = ui.snapshot().unwrap();
        snapshot.visits.len() == count
            && snapshot
                .current_visit()
                .is_some_and(|visit| visit.url.path() == path && !visit.title.is_empty())
            && !ui.engine.native.is_loading()
    })
    .with_context(|| {
        format!(
            "observed {:?}, native URI {:?}",
            ui.snapshot(),
            ui.engine.native.uri()
        )
    })
}

fn link(ui: &Ui, id: &str) -> Result<()> {
    eval(
        ui,
        &format!("document.getElementById('{id}').click(); JSON.stringify(true)"),
    )?;
    Ok(())
}

fn trusted_click(ui: &Ui, id: &str) -> Result<()> {
    let (x, y) = element_point(ui, id)?;
    focus_window()?;
    click_at(x, y)
}

struct Fixture {
    address: std::net::SocketAddr,
    stopped: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let handle = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buffer = [0; 8192];
                let size = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..size]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let response = if path == "/redirect" {
                    "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                } else {
                    let topic;
                    let title = match path {
                        "/a" => "Research starting point",
                        "/b" => "Trail B",
                        "/c" => "Trail C",
                        "/d" => "Link opened in another window",
                        "/final" => "Destination after a redirect",
                        _ if path.starts_with("/topic-") => {
                            topic = format!("A tree about topic {}", &path[7..]);
                            &topic
                        }
                        _ => "Fixture",
                    };
                    let body = format!("<!doctype html><html lang='en'><meta charset='utf-8'><title>{title}</title><style>body{{font:18px system-ui;max-width:720px;margin:64px auto;color:#294331}}a{{display:inline-block;margin:8px 12px 8px 0;color:#356a48}}h1{{font-weight:550}}p{{line-height:1.7;color:#667568}}</style><h1>{title}</h1><p>A research page to explore several trails and find the way back.</p><input id='q' aria-label='Notes'><a id='b' href='/b'>Explore trail B</a><a id='c' href='/c'>Explore trail C</a><a id='blank' href='/d' target='_blank'>Open another trail</a><a id='same' href='/a'>Revisit this URL</a></html>");
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Ok(Self {
            address,
            stopped,
            thread: Some(handle),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
