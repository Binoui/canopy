use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{NavigationTarget, VisitState},
    ui::Ui,
};
use gtk::prelude::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};
use webkit2gtk::WebViewExt;
mod common;
use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}", listener.local_addr()?);
    let icon =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 16, 16).unwrap();
    icon.fill(0x2050ffff);
    let favicon = icon.save_to_bufferv("png", &[])?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let favicon = favicon.clone();
            thread::spawn(move || -> std::io::Result<()> {
                let mut request = [0; 4096];
                let size = stream.read(&mut request)?;
                let request = String::from_utf8_lossy(&request[..size]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                if path == "/favicon.ico" {
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", favicon.len())?;
                    stream.write_all(&favicon)?;
                    return Ok(());
                }
                if path.starts_with("/slow") {
                    thread::sleep(Duration::from_millis(1500));
                }
                let links: String = (0..32).map(|i| format!("<p><a id='link{i}' href='/queued{path}/{i}'>Waiting article {i}</a></p>")).collect();
                let body = format!("<!doctype html><link rel='icon' href='/favicon.ico'><title>Waiting fixture {path}</title><input id='focus'><main>{links}<p><a id='slow' href='/slow'>Slow page</a></p></main>");
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
                Ok(())
            });
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+Tab")?;
    wait("empty forest toast", || {
        toast_text(&ui).as_deref() == Some("No waiting pages in this tree")
    })?;
    assert!(!visible(&ui, "#waiting-pin"));
    ui.pages
        .selected_page()
        .navigate(web_url(&format!("{base}/source"))?, NavigationTarget::Root)?;
    loaded(&ui, "/source")?;
    prove_delivery(&ui)?;
    assert!(!visible(&ui, "#waiting-pin"));
    for i in 0..3 {
        queue(&ui, &format!("link{i}"))?;
    }
    let branch = ui
        .snapshot()?
        .visits
        .iter()
        .find(|v| v.state == VisitState::Queued)
        .unwrap()
        .id;
    ui.pages.selected_page().restore(branch)?;
    wait("branch loaded", || {
        ui.snapshot().unwrap().current == Some(branch)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    for i in 0..14 {
        ui.pages.selected_page().navigate(
            web_url(&format!("{base}/path{i}"))?,
            NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap()),
        )?;
        loaded(&ui, &format!("/path{i}"))?;
    }
    for i in 0..24 {
        queue(&ui, &format!("link{i}"))?;
    }
    wait("counter", || text(&ui, "#waiting-pin .count") == "26")?;
    let snapshot = ui.snapshot()?;
    let mut queued: Vec<_> = snapshot
        .visits
        .iter()
        .filter(|v| v.state == VisitState::Queued)
        .collect();
    queued.sort_by_key(|v| std::cmp::Reverse(v.created_at));
    assert_eq!(
        waiting(&ui)?,
        queued.iter().map(|v| v.id.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(text(&ui, "#tab-row .tab.on .count"), "26");
    assert!(visible(&ui, "#waiting-pin .pending-dot:not(.loaded)"));
    assert!(chrome(&ui, "document.querySelector('#waiting-pin img').getAttribute('src').includes(encodeURIComponent(S.frame.visits[canopy.state().waiting[0]].url))")? == true);
    assert!(chrome(&ui, "[...S.strip.open.nodes].some(n => n.hidden.some(id => canopy.state().waiting.includes(id)))")? == true);
    println!(
        "PASS counter, next icon, state dot, tab count, newest order and hidden waiting pages."
    );
    for width in [1240, 800, 1440] {
        ui.window.resize(width, 800);
        pump(400);
        settle(&ui)?;
        room_check(&ui)?;
        open_strip(&ui)?;
        room_check(&ui)?;
        leave_strip(&ui)?;
    }
    ui.window.resize(800, 800);
    pump(400);
    settle(&ui)?;
    let pin = rect(&ui, "#waiting-pin")?;
    let before = chrome(
        &ui,
        "document.getElementById('strip-canvas').style.transform",
    )?;
    let (x, y) = page_point(&ui, 400, 68)?;
    move_to(x, y)?;
    xdo(&["click", "4"])?;
    pump(80);
    assert_ne!(
        chrome(
            &ui,
            "document.getElementById('strip-canvas').style.transform"
        )?,
        before
    );
    assert_eq!(rect(&ui, "#waiting-pin")?, pin);
    leave_strip(&ui)?;
    println!("PASS pinned counter stays still while the strip pans; current pill fits reserved room at three widths.");
    ui.window.resize(1240, 800);
    pump(400);
    for theme in ["dark", "light"] {
        chrome(
            &ui,
            &format!("(S.frame.theme = '{theme}', canopy.present(S.frame), true)"),
        )?;
        shots(&ui, &format!("{theme}-counter"))?;
        hover_waiting(&ui)?;
        wait("waiting list", || visible(&ui, "#popover.waiting"))?;
        assert!(text(&ui, ".pop-head").contains("Waiting in this tree · 26"));
        assert!(text(&ui, ".pop-head").contains("newest first"));
        assert_eq!(
            chrome(
                &ui,
                "[...document.querySelectorAll('#popover .menu-row')].map(r => r.dataset.id)"
            )?,
            serde_json::json!(waiting(&ui)?)
        );
        assert_eq!(
            text(&ui, "#popover .menu-row:first-of-type kbd"),
            "Ctrl Tab"
        );
        assert!(chrome(&ui, "[...document.querySelectorAll('#popover .menu-row .menu-detail')].every(e => e.textContent === 'not loaded')")? == true);
        assert_eq!(strip_height(&ui), 52);
        assert!(chrome(&ui, "(() => {const d=document.querySelector('#popover .mk .pending-dot').getBoundingClientRect(), i=document.querySelector('#popover .mk .fav').getBoundingClientRect();return d.x+d.width/2 > i.x+i.width/2 && d.y+d.height/2 < i.y+i.height/2})()")? == true);
        shots(&ui, &format!("{theme}-list"))?;
        chrome(&ui, "(canopy.present(S.frame), true)")?;
        assert!(visible(&ui, "#popover.waiting"));
        chrome(&ui, "(() => { window.unloadedFrame = structuredClone(S.frame); const v=S.frame.visits[S.frame.waiting.ids[0]]; v.loaded=true; v.when='loaded'; canopy.present(S.frame); return true; })()")?;
        assert!(visible(&ui, "#waiting-pin .pending-dot.loaded"));
        assert!(chrome(&ui, "getComputedStyle(document.querySelector('.node.bud.pending.loaded')).borderTopStyle === 'none'")? == true);
        chrome(&ui, "(canopy.toggleForest(), true)")?;
        assert!(chrome(&ui, "getComputedStyle(document.querySelector('.forest-node.pending.loaded')).borderTopStyle === 'solid'")? == true);
        assert!(chrome(&ui, "getComputedStyle(document.querySelector('.forest-node.pending:not(.loaded)')).borderTopStyle === 'dashed'")? == true);
        assert!(chrome(&ui, "getComputedStyle(document.querySelector('.forest-node.pending.loaded .fav')).opacity === '1'")? == true);
        assert!(chrome(&ui, "getComputedStyle(document.querySelector('.forest-node.pending:not(.loaded) .fav')).opacity === '0.5'")? == true);
        chrome(&ui, "(canopy.toggleForest(), true)")?;
        assert_eq!(text(&ui, "#popover .next .menu-detail"), "loaded");
        shots(&ui, &format!("{theme}-loaded"))?;
        chrome(&ui, "(canopy.present(window.unloadedFrame), true)")?;
        key("Escape")?;
    }
    hover_waiting(&ui)?;
    wait("list before empty frame", || {
        visible(&ui, "#popover.waiting")
    })?;
    chrome(&ui, "(() => {window.waitingFrame = structuredClone(S.frame); const frame = structuredClone(S.frame); frame.waiting.ids=[]; frame.strip.pins=[]; frame.strip.room=innerWidth; canopy.present(frame); return true})()")?;
    assert_eq!(
        chrome(&ui, "canopy.state().popover")?,
        serde_json::Value::Null
    );
    chrome(&ui, "(canopy.present(window.waitingFrame), true)")?;
    hover_waiting(&ui)?;
    chrome(&ui, "(openPopover('waiting', $('waiting-pin'), null), leaveWaitingPin(), openPopover('settings', $('settings-button'), null), true)")?;
    pump(300);
    assert_eq!(chrome(&ui, "canopy.state().popover")?, "settings");
    key("Escape")?;
    println!(
        "PASS empty frame closes waiting list and stale hover timer preserves another popover."
    );
    println!("PASS hover heading, all ordered rows, first-row state and hint, live frame refresh, loaded marks and top-right dot.");
    hover_waiting(&ui)?;
    wait("row list", || visible(&ui, "#popover.waiting"))?;
    let second = waiting(&ui)?[1].clone();
    click_chrome(&ui, "#popover .menu-row:nth-of-type(2)")?;
    activated(&ui, &second)?;
    let next = waiting(&ui)?[0].clone();
    click_chrome(&ui, "#waiting-pin")?;
    activated(&ui, &next)?;
    for context in ["page", "strip", "palette", "start", "forest"] {
        match context {
            "page" => ui.pages.selected_page().focus(),
            "strip" => {
                key("F6")?;
                wait("node focus", || chrome_focused(&ui, ".node"))?;
            }
            "palette" => {
                key("ctrl+k")?;
                wait("palette focus", || chrome_focused(&ui, "#palette-input"))?;
            }
            "start" => {
                key("ctrl+t")?;
                wait("start page", || {
                    chrome(&ui, "canopy.state().surface").is_ok_and(|v| v == "start")
                })?;
            }
            _ => {
                key("ctrl+e")?;
                wait("forest", || visible(&ui, "#forest"))?;
            }
        }
        let next = waiting(&ui)?[0].clone();
        key("ctrl+Tab")?;
        activated(&ui, &next)?;
        assert_eq!(chrome(&ui, "canopy.state().palette")?, false);
        assert_eq!(chrome(&ui, "canopy.state().surface")?, "reading");
        assert!(!visible(&ui, "#forest"));
        println!(
            "PASS Ctrl+Tab opens next waiting page from {context}, returning focus to the page."
        );
    }
    queue(&ui, "slow")?;
    let next = waiting(&ui)?[0].clone();
    let count = waiting(&ui)?.len();
    key("ctrl+Tab")?;
    pump(50);
    key("ctrl+Tab")?;
    activated(&ui, &next)?;
    assert_eq!(waiting(&ui)?.len(), count - 1);
    println!("PASS repeated Ctrl+Tab during a slow load asks for the same page.");
    while let Some(next) = waiting(&ui)?.first().cloned() {
        key("ctrl+Tab")?;
        activated(&ui, &next)?;
    }
    assert!(!visible(&ui, "#waiting-pin"));
    assert_eq!(
        chrome(&ui, "canopy.state().room")?,
        ui.window.allocated_width()
    );
    key("ctrl+Tab")?;
    wait("nothing waiting toast", || {
        toast_text(&ui).as_deref() == Some("No waiting pages in this tree")
    })?;
    for theme in ["dark", "light"] {
        chrome(
            &ui,
            &format!("(S.frame.theme = '{theme}', canopy.present(S.frame), true)"),
        )?;
        shots(&ui, &format!("{theme}-empty"))?;
    }
    println!("PASS counter hides and full strip room returns at zero; empty shortcut shows toast.");
    ui.window.close();
    Ok(())
}

fn loaded(ui: &Ui, path: &str) -> Result<()> {
    wait("fixture load", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == path)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    pump(100);
    Ok(())
}
fn waiting(ui: &Ui) -> Result<Vec<String>> {
    Ok(serde_json::from_value(chrome(
        ui,
        "canopy.state().waiting",
    )?)?)
}
fn queue(ui: &Ui, link: &str) -> Result<()> {
    let before = ui.snapshot()?.visits.len();
    eval(
        ui,
        &format!(
            "document.getElementById({}).scrollIntoView({{block:'center'}}); JSON.stringify(true)",
            serde_json::to_string(link)?
        ),
    )?;
    pump(30);
    let (x, y) = element_point(ui, link)?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    wait(
        &format!("queued {link} at {x},{y}, visit count {before}"),
        || ui.snapshot().unwrap().visits.len() == before + 1,
    )?;
    pump(80);
    Ok(())
}
fn activated(ui: &Ui, id: &str) -> Result<()> {
    wait("waiting page committed", || {
        ui.snapshot()
            .unwrap()
            .current
            .is_some_and(|v| v.to_string() == id)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    wait("count refreshed", || {
        waiting(ui).is_ok_and(|ids| !ids.iter().any(|v| v == id))
    })?;
    wait("page keyboard", || {
        ui.pages.selected_page().native.has_focus()
    })?;
    Ok(())
}
fn room_check(ui: &Ui) -> Result<()> {
    let (x, _, w, _) = rect(ui, "#waiting-pin")?;
    assert!(x + w <= f64::from(ui.window.allocated_width()) - 7.5);
    assert!(chrome(ui, "(() => {const p=document.querySelector('.node.pill').getBoundingClientRect(), r=canopy.state().room;return p.left>=-0.5 && p.right<=r+0.5})()")? == true);
    Ok(())
}
fn hover_waiting(ui: &Ui) -> Result<()> {
    let (x, y) = page_point(ui, 400, 700)?;
    move_to(x, y)?;
    pump(50);
    hover_chrome(ui, "#waiting-pin")
}
fn shots(ui: &Ui, name: &str) -> Result<()> {
    if let Ok(folder) = std::env::var("CANOPY_WAITING_SHOTS") {
        std::fs::create_dir_all(&folder)?;
        save_window(ui, &format!("{folder}/{name}.png"))?;
    }
    Ok(())
}

fn prove_delivery(ui: &Ui) -> Result<()> {
    for context in ["page", "strip", "palette"] {
        match context {
            "page" => ui.pages.selected_page().focus(),
            "strip" => {
                key("F6")?;
                wait("strip focus", || chrome_focused(ui, ".node"))?;
            }
            _ => {
                key("ctrl+k")?;
                wait("palette focus", || chrome_focused(ui, "#palette-input"))?;
            }
        }
        let focused = ui.window.focused_widget();
        let element = chrome(ui, "[document.activeElement?.tagName, document.activeElement?.id, document.activeElement?.dataset.id]")?;
        chrome(ui, "document.getElementById('toast').hidden = true")?;
        key("ctrl+Tab")?;
        wait("Ctrl+Tab toast", || {
            toast_text(ui).as_deref() == Some("No waiting pages in this tree")
        })?;
        assert_eq!(
            ui.window.focused_widget(),
            focused,
            "{context} native focus"
        );
        assert_eq!(chrome(ui, "[document.activeElement?.tagName, document.activeElement?.id, document.activeElement?.dataset.id]")?, element, "{context} DOM focus");
        println!("PASS Ctrl+Tab delivery and unchanged focus in {context}.");
        if context == "palette" {
            key("Escape")?;
        }
    }
    Ok(())
}
