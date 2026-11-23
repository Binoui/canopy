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
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    let xdg = tempfile::tempdir()?;
    std::env::set_var("XDG_DATA_HOME", xdg.path().join("data"));
    std::env::set_var("XDG_CACHE_HOME", xdg.path().join("cache"));
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}", listener.local_addr()?);
    let icon =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 16, 16).unwrap();
    icon.fill(0x2050ffff);
    let favicon = icon.save_to_bufferv("png", &[])?;
    let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_changed = changed.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let favicon = favicon.clone();
            let changed = server_changed.clone();
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
                use std::sync::atomic::Ordering;
                if path == "/film-change" || path == "/film-mutated" {
                    if path == "/film-mutated" {
                        FILM_MUTATED.store(true, Ordering::Relaxed);
                    }
                    let response = if changed.load(Ordering::Relaxed) {
                        "yes"
                    } else {
                        "no"
                    };
                    write!(stream, "HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len())?;
                    return Ok(());
                }
                if let Some(index) = path
                    .strip_prefix("/colour/")
                    .and_then(|i| i.parse::<usize>().ok())
                {
                    let [r, g, b] = FILM_COLOURS[index];
                    let script = if index == 11 {
                        r#"<script>const timer=setInterval(async()=>{if(await (await fetch('/film-change')).text()==='yes'){document.body.style.background='rgb(220,96,141)';clearInterval(timer);fetch('/film-mutated')}},150)</script>"#
                    } else {
                        ""
                    };
                    let body = format!("<!doctype html><title>Filmstrip colour {index}</title><body style='margin:0;background:rgb({r},{g},{b})'>{script}");
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
                    return Ok(());
                }
                if path == "/film" {
                    let links: String = (0..FILM_COLOURS.len())
                        .map(|i| {
                            format!(
                                "<p><a id='film{i}' href='/colour/{i}'>Filmstrip page {i}</a></p>"
                            )
                        })
                        .collect();
                    let body = format!("<!doctype html><title>Filmstrip source</title>{links}<p><a id='film-slow' href='/slow-film'>Slow fixture</a>");
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
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
    let picture_directory = tempfile::tempdir()?;
    prove_filmstrip(picture_directory.path(), &base, &changed)?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+Tab")?;
    wait("empty forest toast", || {
        toast_text(&ui).as_deref() == Some("No other pages in this tree")
    })?;
    assert!(!visible(&ui, ".node.tab:not([inert])"));
    key("ctrl+shift+e")?;
    pump(200);
    assert_eq!(
        chrome(&ui, "canopy.state().overlay")?,
        serde_json::Value::Null
    );
    assert!(!visible(&ui, "#zoom-pin"));
    ui.pages
        .selected_page()
        .navigate(web_url(&format!("{base}/source"))?, NavigationTarget::Root)?;
    loaded(&ui, "/source")?;
    key("ctrl+shift+e")?;
    wait("zero-waiting sheet", || visible(&ui, "#tree-sheet"))?;
    assert_eq!(text(&ui, "#tree-detail"), "1 page · 0 waiting");
    key("Escape")?;
    prove_delivery(&ui)?;
    key("F6")?;
    assert!(!visible(&ui, ".node.tab:not([inert])"));
    for i in 0..3 {
        queue(&ui, &format!("link{i}"))?;
        if i == 0 {
            assert_eq!(
                chrome(
                    &ui,
                    "getComputedStyle(document.querySelector('.node.tab')).animationDuration"
                )?,
                "0.32s"
            );
        }
        let expected = if i == 0 {
            "Waiting · Waiting article 0".to_owned()
        } else {
            format!("{} waiting from this page", i + 1)
        };
        wait("queued link feedback", || {
            toast_text(&ui).as_deref() == Some(&expected)
        })?;
        assert_eq!(chrome(&ui, "$('toast-status').textContent")?, expected);
        assert_eq!(
            chrome(&ui, "document.querySelectorAll('.node.tab').length")?,
            i + 1
        );
    }
    composed_waiting_toast(&ui)?;
    println!("PASS middle-click announces link text, batches three waiting links and shows each queued link as a tab.");
    let branch = ui
        .snapshot()?
        .visits
        .iter()
        .find(|v| v.state == VisitState::Queued)
        .unwrap()
        .id;
    ui.pages.go(canopy::pages::Go::Visit(branch))?;
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
    wait("ordered waiting pages", || {
        waiting(&ui).is_ok_and(|ids| ids.len() == 26)
    })?;
    assert!(chrome(&ui, "!document.getElementById('waiting-pin')")? == true);
    let snapshot = ui.snapshot()?;
    let mut queued: Vec<_> = snapshot
        .visits
        .iter()
        .filter(|v| v.state == VisitState::Queued)
        .collect();
    queued.sort_by_key(|v| {
        (
            v.parent != snapshot.current,
            std::cmp::Reverse(v.created_at),
        )
    });
    assert_eq!(
        waiting(&ui)?,
        queued.iter().map(|v| v.id.to_string()).collect::<Vec<_>>()
    );
    assert!(visible(
        &ui,
        "#tab-row .tab[aria-selected=true] .unread-dot"
    ));
    assert!(
        chrome(&ui, "document.querySelector('#tab-row .tab[aria-selected=true]').title.includes('26 waiting')")? == true
    );
    wait("newest waiting page loaded", || {
        chrome(&ui, "strip.elements.get(S.frame.waiting.ids[0])?.querySelector('.pending-dot.loaded') != null").is_ok_and(|v| v == true)
    })?;
    assert!(chrome(&ui, "strip.elements.get(S.frame.waiting.ids[0]).querySelector('img').getAttribute('src').includes(encodeURIComponent(S.frame.visits[canopy.state().waiting[0]].url))")? == true);
    assert!(chrome(&ui, "S.strip.nodes.some(n => n.kind === 'tab' && n.divider) || S.strip.nodes.some(n => n.kind === 'overflow')")? == true);
    println!("PASS first tab icon and loaded mark, tab-row badge and exact waiting order.");
    for width in [1240, 800, 1440, 390, 712] {
        ui.window.resize(width, 800);
        pump(400);
        settle(&ui)?;
        room_check(&ui)?;
        ui.pages.selected_page().focus();
        key("F6")?;
        wait(&format!("current list at {width}px"), || {
            visible(&ui, "#strip-list")
        })?;
        room_check(&ui)?;
        key("F6")?;
        leave_strip(&ui)?;
    }
    ui.window.resize(800, 800);
    pump(400);
    settle(&ui)?;
    open_list(&ui)?;
    let pin = rect(&ui, ".node.overflow")?;
    let before = chrome(
        &ui,
        "document.getElementById('strip-canvas').style.transform",
    )?;
    let (x, y) = page_point(&ui, 400, 68)?;
    move_to(x, y)?;
    xdo(&["click", "4"])?;
    pump(80);
    assert_eq!(
        chrome(
            &ui,
            "document.getElementById('strip-canvas').style.transform"
        )?,
        before
    );
    assert_eq!(rect(&ui, ".node.overflow")?, pin);
    leave_strip(&ui)?;
    println!("PASS the fitted strip stays inside its clip at five widths and keeps its position while scrolling.");
    ui.window.resize(1240, 800);
    pump(400);
    key("ctrl+shift+e")?;
    wait("sheet from resting chrome", || visible(&ui, "#tree-sheet"))?;
    assert_eq!(chrome(&ui, "S.treeSide")?, "list");
    assert_eq!(chrome(&ui, "(() => {const a=document.querySelector('.tree-row[aria-current=page]').getBoundingClientRect(),b=$('tree-list').getBoundingClientRect();return a.top >= b.top && a.bottom <= b.bottom;})()")?, true);
    chrome(&ui, "(setTreeSide('tree'), true)")?;
    wait("current tile revealed", || {
        chrome(&ui, "(() => {const a=document.querySelector('.tree-tile[aria-current=page]').getBoundingClientRect(),b=$('tree-board').getBoundingClientRect();return $('tree-board').scrollHeight > $('tree-board').clientHeight && a.left >= b.left && a.right <= b.right && a.top >= b.top && a.bottom <= b.bottom;})()").is_ok_and(|v| v == true)
    })?;
    println!("PASS first sheet open selects List for a tall tree and reveals the current page on both sides.");
    key("Escape")?;
    prove_tree_sheet(&ui)?;
    for theme in ["dark", "light"] {
        chrome(
            &ui,
            &format!("(S.frame.theme = '{theme}', canopy.present(S.frame), true)"),
        )?;
        shots(&ui, &format!("{theme}-tabs"))?;
        hover_waiting(&ui)?;
        wait("waiting list", || visible(&ui, "#popover.waiting"))?;
        assert!(text(&ui, ".pop-head").contains("Waiting in this tree · 26"));
        assert!(
            chrome(
                &ui,
                "document.querySelector('.node.overflow').getAttribute('aria-expanded') === 'true'"
            )? == true
        );
        assert!(chrome(&ui, "[...document.querySelectorAll('#popover .menu-detail')].some(e => e.textContent.startsWith('Waiting article'))")? == true);
        assert!(text(&ui, ".pop-head").contains("this page first, then newest"));
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
        assert!(chrome(&ui, "[...document.querySelectorAll('#popover .menu-row .menu-detail')].every(e => e.textContent === S.frame.visits[e.closest('.menu-row').dataset.id].detail)")? == true);
        assert_eq!(strip_height(&ui), 52);
        assert!(chrome(&ui, "(() => {const d=document.querySelector('#popover .mk .pending-dot').getBoundingClientRect(), i=document.querySelector('#popover .mk .fav').getBoundingClientRect();return d.x+d.width/2 > i.x+i.width/2 && d.y+d.height/2 < i.y+i.height/2})()")? == true);
        shots(&ui, &format!("{theme}-list"))?;
        chrome(&ui, "(canopy.present(S.frame), true)")?;
        assert!(visible(&ui, "#popover.waiting"));
        assert!(
            chrome(
                &ui,
                "document.querySelector('.node.overflow').getAttribute('aria-expanded') === 'true'"
            )? == true
        );
        chrome(&ui, "(() => { window.unloadedFrame = structuredClone(S.frame); const v=S.frame.visits[S.frame.waiting.ids[0]]; v.loaded=true; v.when='loaded'; v.detail='loaded'; const u=S.frame.visits[S.frame.waiting.ids[1]]; u.loaded=false; u.when=''; u.detail=''; canopy.present(S.frame); return true; })()")?;
        assert!(chrome(&ui, "strip.elements.get(S.frame.waiting.ids[0])?.querySelector('.pending-dot.loaded') != null").is_ok_and(|v| v == true));
        assert!(
            chrome(
                &ui,
                "!!document.querySelector('#popover .mk .pending-dot.loaded')"
            )? == true
        );
        chrome(&ui, "(canopy.toggleForest(), true)")?;
        wait("Canopy waiting marks", || {
            visible(&ui, "#forest-canopy [data-st=waiting]")
        })?;
        assert!(visible(
            &ui,
            "#forest-canopy [data-current=true][data-page]"
        ));
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
    hover_chrome(&ui, "#popover.waiting [data-id]")?;
    xdo(&["click", "3"])?;
    wait("nested waiting-row menu", || {
        visible(&ui, "#popover [data-node-action]")
    })?;
    chrome(&ui, "(() => {window.waitingFrame = structuredClone(S.frame); const frame = structuredClone(S.frame); frame.waiting.ids=[]; frame.strip.nodes=frame.strip.nodes.filter(n=>!['tab','overflow'].includes(n.kind)); frame.strip.trunk=frame.strip.nodes.map(n=>n.id); canopy.present(frame); return true})()")?;
    assert_eq!(
        chrome(&ui, "canopy.state().popover")?,
        serde_json::Value::Null
    );
    chrome(&ui, "(canopy.present(window.waitingFrame), true)")?;
    hover_waiting(&ui)?;
    chrome(&ui, "(openQueue(document.querySelector('.node.overflow')), leaveQueue(), openPopover('settings', $('settings-button'), null), true)")?;
    pump(300);
    assert_eq!(chrome(&ui, "canopy.state().popover")?, "settings");
    key("Escape")?;
    println!(
        "PASS losing overflow closes its nested page menu and stale hover does not close another popover."
    );
    println!("PASS hover heading, all ordered rows, first-row state and hint, live frame refresh, loaded marks and top-right dot.");
    hover_waiting(&ui)?;
    wait("row list", || visible(&ui, "#popover.waiting"))?;
    let second = waiting(&ui)?[1].clone();
    click_chrome(&ui, "#popover .menu-row:nth-of-type(2)")?;
    activated(&ui, &second)?;
    let next = waiting(&ui)?[0].clone();
    let tab = format!("#strip-nodes [data-id='{next}']");
    chrome(
        &ui,
        &format!(
            "(window.pressedTab=document.querySelector({}),true)",
            serde_json::to_string(&tab)?
        ),
    )?;
    let (x, y) = chrome_point(&ui, &tab)?;
    move_to(x, y)?;
    xdo(&["mousedown", "1"])?;
    wait("waiting tab press reaches the chrome", || {
        chrome(&ui, "canopy.state().buttons").is_ok_and(|value| value == 1)
    })?;
    chrome(&ui, "(canopy.present(structuredClone(S.frame)), true)")?;
    assert!(chrome(&ui, "canopy.state().deferred.includes('present')")? == true);
    xdo(&["mouseup", "1"])?;
    activated(&ui, &next)?;
    assert!(
        chrome(
            &ui,
            "document.querySelector('#strip-nodes .pill') === window.pressedTab"
        )? == true
    );
    println!(
        "PASS a real click spans a held frame and keeps the same tab element as the current pill."
    );
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
    let before = waiting(&ui)?;
    let opening = &before[0];
    key("ctrl+Tab")?;
    wait("slow page becomes current", || {
        chrome(&ui, "S.frame.current").is_ok_and(|id| id == opening.as_str())
    })?;
    let next = waiting(&ui)?[0].clone();
    assert_ne!(&next, opening);
    key("ctrl+Tab")?;
    activated(&ui, &next)?;
    assert!(waiting(&ui)?.contains(opening));
    println!("PASS repeated Ctrl+Tab during a slow load opens the next waiting page.");
    let mut read = Vec::new();
    while let Some(next) = waiting(&ui)?.first().cloned() {
        key("ctrl+Tab")?;
        activated(&ui, &next)?;
        read.push(next);
    }
    assert!(!visible(&ui, ".node.tab:not([inert])"));
    assert_eq!(
        chrome(&ui, "canopy.state().pins")?,
        serde_json::json!(["zoom"])
    );
    assert!(
        chrome(&ui, "canopy.state().room")?.as_f64().unwrap()
            < f64::from(ui.window.allocated_width())
    );
    switch_read_pages(&ui, &read)?;
    dismiss_tabs(&ui, &base)?;
    for theme in ["dark", "light"] {
        chrome(
            &ui,
            &format!("(S.frame.theme = '{theme}', canopy.present(S.frame), true)"),
        )?;
        shots(&ui, &format!("{theme}-empty"))?;
    }
    println!("PASS counter hides and only the zoom pin reserves room at zero; empty shortcut shows toast.");
    small_tree_shots(&ui, &base)?;
    ui.window.close();
    Ok(())
}

/// With nothing waiting, Ctrl+Tab goes through read pages: a quick press flips between the
/// last two, a held Ctrl steps down the list and opens the row selected on release.
fn switch_read_pages(ui: &Ui, read: &[String]) -> Result<()> {
    let [.., previous, last] = read else {
        anyhow::bail!("the test reads at least two waiting pages");
    };
    assert!(!visible(ui, "#switcher"));
    key("ctrl+Tab")?;
    activated(ui, previous)?;
    key("ctrl+Tab")?;
    activated(ui, last)?;
    assert!(
        !visible(ui, "#switcher"),
        "a quick press never shows the list"
    );
    println!("PASS with nothing waiting, a quick Ctrl+Tab flips between the last two read pages.");
    let rows = |ui: &Ui| -> Result<Vec<String>> {
        Ok(serde_json::from_value(chrome(
            ui,
            "[...document.querySelectorAll('#switcher [data-id]')].map((row) => row.dataset.id)",
        )?)?)
    };
    let selected = |ui: &Ui, row: &str, why: &str| {
        let row = serde_json::json!(row);
        wait(why, || {
            chrome(
                ui,
                "document.querySelector('#switcher [aria-selected=\"true\"]')?.dataset.id ?? null",
            )
            .is_ok_and(|id| id == row)
        })
    };
    focus_window()?;
    xdo(&["keydown", "ctrl"])?;
    xdo(&["key", "Tab"])?;
    pump(60);
    xdo(&["key", "Tab"])?;
    wait("list shown", || visible(ui, "#switcher"))?;
    let listed = rows(ui)?;
    assert_eq!(listed.first(), Some(last), "the current page is first");
    assert_eq!(
        listed.get(1),
        Some(previous),
        "most recently read follows Here"
    );
    selected(ui, &listed[2], "second Tab selects the second read page")?;
    shots(ui, "filmstrip")?;
    xdo(&["key", "shift+Tab"])?;
    selected(ui, &listed[1], "Shift+Tab steps back")?;
    xdo(&["key", "Left"])?;
    selected(ui, &listed[0], "Left steps to Here")?;
    xdo(&["keyup", "ctrl"])?;
    pump(200);
    assert_eq!(
        ui.snapshot()?.current.map(|id| id.to_string()).as_ref(),
        Some(last)
    );
    xdo(&["keydown", "ctrl"])?;
    xdo(&["key", "Tab"])?;
    wait("filmstrip shown after a single Tab", || {
        visible(ui, "#switcher")
    })?;
    selected(ui, &listed[1], "the hold starts on the first read page")?;
    xdo(&["key", "shift+Tab"])?;
    selected(ui, &listed[0], "Shift+Tab steps to Here")?;
    xdo(&["key", "shift+Tab"])?;
    selected(ui, listed.last().unwrap(), "Shift+Tab wraps")?;
    xdo(&["key", "Right"])?;
    selected(ui, &listed[0], "Right wraps to the first card")?;
    xdo(&["key", "Tab"])?;
    selected(ui, &listed[1], "Tab steps to the first read page")?;
    xdo(&["keyup", "ctrl"])?;
    activated(ui, &listed[1])?;
    wait("release closes filmstrip", || !visible(ui, "#switcher"))?;
    println!("PASS held Ctrl+Tab lists Here then read pages newest first, arrows and Tab step and wrap, release opens the card.");
    let here = ui.snapshot()?.current;
    xdo(&["keydown", "ctrl"])?;
    xdo(&["key", "Tab"])?;
    pump(30);
    xdo(&["key", "Tab"])?;
    wait("list shown again", || visible(ui, "#switcher"))?;
    xdo(&["key", "Escape"])?;
    wait("Escape drops the list", || !visible(ui, "#switcher"))?;
    xdo(&["keyup", "ctrl"])?;
    pump(200);
    assert_eq!(
        ui.snapshot()?.current,
        here,
        "release after Escape opens nothing"
    );
    println!("PASS Escape with Ctrl held drops the list and the release opens nothing.");
    queue(ui, "link5")?;
    let queued = waiting(ui)?;
    assert_eq!(queued.len(), 1);
    let cards = |kind: &str| -> Result<Vec<String>> {
        Ok(serde_json::from_value(chrome(
            ui,
            &format!("[...document.querySelectorAll('#switcher .switcher-option[data-kind=\"{kind}\"]')].map((row) => row.dataset.id)"),
        )?)?)
    };
    focus_window()?;
    xdo(&["keydown", "ctrl"])?;
    xdo(&["key", "Tab"])?;
    pump(60);
    xdo(&["key", "Tab"])?;
    wait("list with a waiting page", || visible(ui, "#switcher"))?;
    assert_eq!(cards("waiting")?, queued, "waiting pages follow Here");
    let read = cards("read")?;
    assert!(read.len() >= 2, "read pages follow waiting");
    assert_eq!(
        rows(ui)?,
        [cards("here")?, queued.clone(), read.clone()].concat()
    );
    selected(ui, &read[0], "the second Tab crosses to the read cards")?;
    shots(ui, "filmstrip-waiting-and-read")?;
    click_chrome(ui, &format!("#switcher [data-id=\"{}\"]", read[1]))?;
    activated(ui, &read[1])?;
    assert!(!visible(ui, "#switcher"));
    xdo(&["keyup", "ctrl"])?;
    pump(200);
    assert_eq!(
        ui.snapshot()?.current.map(|id| id.to_string()).as_ref(),
        Some(&read[1]),
        "releasing Ctrl after a click opens nothing more"
    );
    println!("PASS the filmstrip shows waiting before read pages, Tab crosses the divider, a click opens a card.");
    key("ctrl+Tab")?;
    activated(ui, &queued[0])?;
    println!("PASS a quick Ctrl+Tab still opens the waiting page first.");
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
fn dismiss_tabs(ui: &Ui, base: &str) -> Result<()> {
    ui.pages
        .selected_page()
        .navigate(web_url(&format!("{base}/dismiss"))?, NavigationTarget::Root)?;
    loaded(ui, "/dismiss")?;
    for i in 0..3 {
        queue(ui, &format!("link{i}"))?;
    }
    settle(ui)?;
    let ids = waiting(ui)?;
    let tab = |id: &str| format!("#strip-nodes .node.tab[data-id='{id}']");
    ui.pages.selected_page().focus();
    key("F6")?;
    key("Right")?;
    assert!(chrome_focused(ui, &tab(&ids[0])));
    assert!(text(ui, "#strip-list").contains("queued from"));
    assert!(visible(ui, "#strip-list [data-act=dismiss]"));
    key("shift+F10")?;
    wait("keyboard page menu", || {
        visible(ui, "#popover [data-node-action=dismiss]")
    })?;
    key("Escape")?;
    key("Delete")?;
    wait("Delete transfers focus", || {
        chrome_focused(ui, &tab(&ids[1]))
    })?;
    assert!(!waiting(ui)?.contains(&ids[0]));
    settle(ui)?;
    hover_chrome(ui, &tab(&ids[2]))?;
    xdo(&["click", "2"])?;
    wait("middle click dismisses", || {
        waiting(ui).is_ok_and(|pages| !pages.contains(&ids[2]))
    })?;
    settle(ui)?;
    hover_chrome(ui, &tab(&ids[1]))?;
    let cross = format!("{} .x", tab(&ids[1]));
    wait("waiting tab cross is visible", || visible(ui, &cross))?;
    let (x, y) = chrome_point(ui, &cross)?;
    move_to(x, y)?;
    xdo(&["mousedown", "1"])?;
    let pressed = format!("crossPress?.closest('.node')?.dataset.id === '{}'", ids[1]);
    wait("press lands on the visible cross", || {
        chrome(ui, &pressed).is_ok_and(|value| value == true)
    })?;
    xdo(&["mouseup", "1"])?;
    wait("cross dismisses", || {
        waiting(ui).is_ok_and(|pages| pages.is_empty())
    })?;
    queue(ui, "link3")?;
    settle(ui)?;
    ui.pages.selected_page().focus();
    key("F6")?;
    key("Right")?;
    key("Delete")?;
    wait("last Delete returns focus to the pill", || {
        chrome_focused(ui, "#strip-nodes .pill") && waiting(ui).is_ok_and(|pages| pages.is_empty())
    })?;
    let snapshot = ui.snapshot()?;
    assert!(ids.iter().all(|id| snapshot
        .visits
        .iter()
        .any(|visit| visit.id.to_string() == *id && visit.dismissed_at.is_some())));
    println!("PASS real keys and pointer open the tab card and menu, dismiss tabs, and return focus after Delete.");
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
    let (x, _, w, _) = rect(ui, "#zoom-pin")?;
    assert!(x + w <= f64::from(ui.window.allocated_width()) - 7.5);
    assert!(
        chrome(
            ui,
            r#"(() => {
        const clip = $('strip-clip').getBoundingClientRect();
        return S.strip.nodes.every(n => {
            const r = strip.elements.get(n.id).getBoundingClientRect();
            return r.left >= clip.left - 0.5 && r.right <= clip.right + 0.5;
        });
    })()"#
        )? == true
    );
    assert!(
        chrome(
            ui,
            r#"(() => {
        const tabs = S.strip.nodes.filter(n => n.kind === 'tab');
        const hidden = S.strip.nodes.find(n => n.kind === 'overflow');
        return tabs.length + Number(hidden?.label ?? 0) === S.frame.waiting.ids.length
            && tabs.every((n, i) => n.page === S.frame.waiting.ids[i]);
    })()"#
        )? == true
    );
    Ok(())
}
fn hover_waiting(ui: &Ui) -> Result<()> {
    let (x, y) = page_point(ui, 400, 700)?;
    move_to(x, y)?;
    pump(50);
    hover_chrome(ui, ".node.overflow")
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
            toast_text(ui).as_deref() == Some("No other pages in this tree")
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

fn prove_tree_sheet(ui: &Ui) -> Result<()> {
    let original = ui.snapshot()?.current.unwrap();
    let pin = rect(ui, "#zoom-pin")?;
    assert!(pin.0 + pin.2 <= f64::from(ui.window.allocated_width()) - 7.5);
    key("ctrl+t")?;
    key("ctrl+shift+e")?;
    wait("tree sheet from start", || visible(ui, "#tree-sheet"))?;
    assert_eq!(chrome(ui, "canopy.state().surface")?, "reading");
    assert_eq!(chrome(ui, "canopy.state().overlay")?, "tree");
    assert_eq!(rect(ui, "#zoom-pin")?, pin);
    assert!(visible(ui, "#zoom-pin[aria-pressed=true]"));
    assert_eq!(rect(ui, ".sheet-box")?.1, 42.);
    assert!(rect(ui, ".sheet-box")?.3 > 52.);
    assert_eq!(cover(ui), None);
    assert!(visible(ui, "#tree-sheet > .scrim"));
    assert!(chrome(ui, "document.querySelector('#tree-title').textContent === S.frame.treeView.title && document.querySelector('#tree-detail').textContent === S.frame.treeView.detail")? == true);
    assert!(chrome(ui, "document.querySelectorAll('.tree-tile').length === S.frame.treeView.tiles.length && document.querySelectorAll('.tree-tile[aria-current=page]').length === 1 && !!document.querySelector('.tree-tile.pending .pending-dot')")? == true);
    assert!(chrome(ui, "(() => {const a=document.querySelector('.tree-tile[aria-current=page]').getBoundingClientRect(),b=document.querySelector('#tree-board').getBoundingClientRect();return a.left>=b.left && a.right<=b.right && a.top>=b.top && a.bottom<=b.bottom})()")? == true);
    hover_chrome(ui, ".sheet-head")?;
    pump(400);
    assert_eq!(strip_height(ui), 52);
    assert!(chrome(ui, "canopy.state().list")?.is_null());
    assert!(!visible(ui, "#strip-list"));
    for theme in ["dark", "light"] {
        chrome(ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
        wait("sheet theme", || {
            chrome(ui, "canopy.state().dark").is_ok_and(|v| v == (theme == "dark"))
        })?;
        chrome(ui, "(setTreeSide('tree'), true)")?;
        pump(100);
        tree_shot(ui, &format!("tall-{theme}-tree"))?;
        click_chrome(ui, "[data-side=list]")?;
        assert!(chrome(ui, "document.querySelectorAll('.tree-row').length === S.frame.treeView.rows.length && !!document.querySelector('.tree-row[aria-current=page]') && [...document.querySelectorAll('.tree-row.pending')].every(r=>r.querySelector('.menu-detail').textContent === S.frame.visits[r.dataset.id].detail)")? == true);
        assert!(chrome(ui, "document.querySelector('#tree-list').scrollHeight > document.querySelector('#tree-list').clientHeight")? == true);
        tree_shot(ui, &format!("tall-{theme}-list"))?;
    }
    key("Escape")?;
    wait("sheet escape", || !visible(ui, "#tree-sheet"))?;
    key("ctrl+shift+e")?;
    wait("remember list", || visible(ui, "#tree-list"))?;
    assert_eq!(chrome(ui, "canopy.state().treeSide")?, "list");
    key("ctrl+shift+e")?;
    wait("sheet shortcut closes", || !visible(ui, "#tree-sheet"))?;
    click_chrome(ui, "#zoom-pin")?;
    wait("sheet button opens", || visible(ui, "#tree-sheet"))?;
    click_chrome(ui, "#zoom-pin")?;
    wait("sheet button closes", || !visible(ui, "#tree-sheet"))?;
    key("ctrl+shift+e")?;
    wait("sheet for scrim", || visible(ui, "#tree-sheet"))?;
    let (x, y) = chrome_point(ui, "#tree-sheet > .scrim")?;
    let (_, top, _, h) = rect(ui, ".sheet-box")?;
    click_at(x, y.max((top + h + 20.) as i32))?;
    wait("sheet scrim closes", || !visible(ui, "#tree-sheet"))?;
    key("ctrl+shift+e")?;
    click_chrome(ui, "[data-side=tree]")?;
    click_chrome(ui, ".tree-tile[aria-current=page]")?;
    wait("current tile closes", || !visible(ui, "#tree-sheet"))?;
    let target = chrome(ui, "S.frame.treeView.rows[0].id")?
        .as_str()
        .unwrap()
        .to_owned();
    key("ctrl+shift+e")?;
    click_chrome(ui, "[data-side=list]")?;
    chrome(
        ui,
        "(document.querySelector('#tree-list').scrollTop=0,true)",
    )?;
    click_chrome(ui, &format!(".tree-row[data-id='{target}']"))?;
    wait("list opens selected page", || {
        ui.snapshot().unwrap().current.unwrap().to_string() == target && !visible(ui, "#tree-sheet")
    })?;
    ui.pages.selected_page().restore(original)?;
    wait("restore sheet fixture", || {
        ui.snapshot().unwrap().current == Some(original)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    println!("PASS tree sheet pins, header, marks, native scrolling, side memory, start-page shortcut, tile/list activation, all close paths and hover suppression.");
    Ok(())
}
fn tree_shot(ui: &Ui, name: &str) -> Result<()> {
    if let Ok(folder) = std::env::var("CANOPY_TREE_SHOTS") {
        std::fs::create_dir_all(&folder)?;
        save_window(ui, &format!("{folder}/{name}.png"))?;
        let status = std::process::Command::new("import")
            .args(["-window", "root", &format!("{folder}/{name}-composed.png")])
            .status()?;
        assert!(status.success());
    }
    Ok(())
}

fn small_tree_shots(ui: &Ui, base: &str) -> Result<()> {
    ui.pages.selected_page().navigate(
        web_url(&format!("{base}/sheet-root"))?,
        NavigationTarget::Root,
    )?;
    loaded(ui, "/sheet-root")?;
    let root = ui.snapshot()?.current.unwrap();
    ui.pages.selected_page().navigate(
        web_url(&format!("{base}/sheet-branch"))?,
        NavigationTarget::ChildOf(root),
    )?;
    loaded(ui, "/sheet-branch")?;
    queue(ui, "link0")?;
    queue(ui, "link1")?;
    let branch = ui.snapshot()?.current.unwrap();
    ui.pages.selected_page().restore(root)?;
    loaded(ui, "/sheet-root")?;
    queue(ui, "link2")?;
    ui.pages.go(canopy::pages::Go::Visit(branch))?;
    loaded(ui, "/sheet-branch")?;
    ui.pages.selected_page().navigate(
        web_url(&format!("{base}/sheet-current"))?,
        NavigationTarget::ChildOf(branch),
    )?;
    loaded(ui, "/sheet-current")?;
    key("ctrl+shift+e")?;
    wait("small screenshot sheet", || visible(ui, "#tree-sheet"))?;
    wait("old toast gone", || !visible(ui, "#toast"))?;
    for theme in ["dark", "light"] {
        chrome(ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
        wait("sheet theme", || {
            chrome(ui, "canopy.state().dark").is_ok_and(|v| v == (theme == "dark"))
        })?;
        chrome(ui, "(setTreeSide('tree'), true)")?;
        pump(100);
        tree_shot(ui, &format!("{theme}-tree"))?;
        click_chrome(ui, "[data-side=list]")?;
        tree_shot(ui, &format!("{theme}-list"))?;
    }
    key("Escape")?;
    println!(
        "PASS screenshots use a short path, side branches at two path pages and waiting pages."
    );
    Ok(())
}

fn composed_waiting_toast(ui: &Ui) -> Result<()> {
    key("ctrl+k")?;
    wait("palette over queued page", || visible(ui, "#palette"))?;
    let (x, y, _, _) = rect(ui, "#toast")?;
    let (x, y) = screen_point(ui, &ui.chrome.native(), x as i32 + 12, y as i32 + 8)?;
    let expected = chrome(
        ui,
        "getComputedStyle($('toast')).backgroundColor.match(/\\d+/g).slice(0,3).map(Number)",
    )?;
    wait("waiting toast paints over the palette", || {
        let Some(image) = gtk::gdk::Screen::default()
            .and_then(|s| s.root_window())
            .and_then(|w| w.pixbuf(x, y, 1, 1))
        else {
            return false;
        };
        let pixels = image.read_pixel_bytes();
        (0..3).all(|i| (pixels[i] as i64 - expected[i].as_i64().unwrap()).abs() < 12)
    })?;
    key("Escape")?;
    println!("PASS waiting feedback paints in the composed window with the palette open.");
    Ok(())
}

fn prove_filmstrip(
    directory: &std::path::Path,
    base: &str,
    changed: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    use std::{cell::Cell, rc::Rc, sync::atomic::Ordering, time::Instant};

    let ui = Ui::new(directory)?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages
        .selected_page()
        .navigate(web_url(&format!("{base}/film"))?, NavigationTarget::Root)?;
    loaded(&ui, "/film")?;
    let here = ui.snapshot()?.current.unwrap();
    let film_choices = || -> Result<Vec<String>> {
        Ok(serde_json::from_value(chrome(
            &ui,
            "[...$('switcher-row').querySelectorAll('[role=option]')].map(e => e.dataset.id)",
        )?)?)
    };
    let selection = || -> Result<String> {
        Ok(serde_json::from_value(chrome(
            &ui,
            "S.switcher?.selected ?? ''",
        )?)?)
    };
    let expect_selection = |id: &str| {
        wait("selected filmstrip card", || {
            selection().is_ok_and(|s| s == id)
        })
    };
    let held = || -> Result<()> {
        focus_window()?;
        xdo(&["keydown", "ctrl"])?;
        xdo(&["key", "Tab"])?;
        wait("timed filmstrip reveal", || visible(&ui, "#switcher"))
    };
    let cancel = || -> Result<()> {
        xdo(&["key", "Escape"])?;
        xdo(&["keyup", "ctrl"])?;
        wait("filmstrip closes on Escape", || !visible(&ui, "#switcher"))
    };
    let mut recent = Vec::new();
    for i in 0..7 {
        ui.pages.inject_recent_memory_sample(0);
        queue(&ui, &format!("film{i}"))?;
        let id = ui
            .snapshot()?
            .visits
            .into_iter()
            .find(|v| v.url.path() == format!("/colour/{i}"))
            .unwrap()
            .id;
        wait("read fixture loaded while waiting", || {
            ui.pages.loaded_visits().contains(&id)
        })?;
        let before = ui.chrome.switcher_snapshot_count();
        key("ctrl+Tab")?;
        activated(&ui, &id.to_string())?;
        assert!(!visible(&ui, "#switcher"));
        assert_eq!(ui.chrome.switcher_snapshot_count(), before);
        recent.insert(0, id.to_string());
        key("alt+Left")?;
        activated(&ui, &here.to_string())?;
    }
    for i in 7..12 {
        ui.pages.inject_recent_memory_sample(0);
        queue(&ui, &format!("film{i}"))?;
    }
    wait(
        "five committed background pages and eight retained read pages",
        || ui.pages.loaded_visits().len() == 5 && ui.pages.live_visits().len() == 8,
    )?;
    let queued = waiting(&ui)?;
    let order = [vec![here.to_string()], queued.clone(), recent.clone()].concat();
    assert_eq!(order.len(), 13);
    wait("page hold has ended", || {
        !ui.pages.selected_page().native.is_loading()
    })?;
    pump(1500);
    let mut roots = vec![directory.to_path_buf()];
    roots.extend(
        ["XDG_DATA_HOME", "XDG_CACHE_HOME"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(std::path::PathBuf::from),
    );
    for view in [ui.chrome.native(), ui.pages.selected_page().native.clone()] {
        use webkit2gtk::{WebContextExt, WebsiteDataManagerExt};
        let manager = view.context().unwrap().website_data_manager().unwrap();
        roots.extend(
            [
                manager.base_cache_directory(),
                manager.base_data_directory(),
            ]
            .into_iter()
            .flatten()
            .map(|path| std::path::PathBuf::from(path.as_str())),
        );
    }
    let files = film_inventory(&roots)?;
    let control = directory.join("inventory-control");
    std::fs::write(&control, "visible to the inventory")?;
    assert!(film_inventory(&roots)?.contains(&control));
    std::fs::remove_file(&control)?;

    let mut samples = Vec::new();
    for _ in 0..5 {
        let previous = Rc::new(Cell::new(Instant::now()));
        let longest = Rc::new(Cell::new(Duration::ZERO));
        let tick = glib::timeout_add_local(Duration::from_millis(2), {
            let longest = longest.clone();
            move || {
                let gap = previous.replace(Instant::now()).elapsed();
                longest.set(gap.max(longest.get()));
                glib::ControlFlow::Continue
            }
        });
        held()?;
        let mut painted_ms = 0.;
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            if let Some(ms) = chrome(&ui, "S.switcher?.picturesPainted ?? null")?.as_f64() {
                painted_ms = ms;
                break;
            }
            pump(4);
        }
        tick.remove();
        assert!(painted_ms > 0., "ready pictures: {}", chrome(&ui,
            "[...S.switcher.rows.values()].map(r => [r.v.pictureStatus, r.el.dataset.picture, r.v.label])")?);
        samples.push((painted_ms, longest.get().as_secs_f64() * 1000.));
        assert_eq!(film_choices()?, order);
        expect_selection(&queued[0])?;
        let v = ui
            .snapshot()?
            .visits
            .into_iter()
            .find(|v| v.id.to_string() == queued[0])
            .unwrap();
        assert!(v.opened_at.is_none());
        film_pixel(&ui, FILM_COLOURS[11])?;
        assert_eq!(chrome(&ui, "document.activeElement.id")?, "switcher-row");
        assert_eq!(chrome(&ui, "$('switcher-row').getAttribute('aria-activedescendant') === $('switcher-row').querySelector('[aria-selected=true]').id")?, true);
        cancel()?;
    }
    println!("FILMSTRIP 1240x800, 13 retained, five openings: (cards to pictures ms, largest GTK 2 ms timer gap ms) {samples:?}");
    assert!(
        samples.iter().all(|(elapsed, _)| *elapsed <= 300.),
        "{samples:?}"
    );
    assert_eq!(
        film_inventory(&roots)?,
        files,
        "filmstrip must create no files"
    );
    assert_eq!(ui.snapshot()?.current, Some(here));
    println!("PASS filmstrip paints 13 retained pages within 300 ms and creates no files in the private data and cache folders.");

    held()?;
    wait("large snapshot visible", || {
        visible(&ui, "#switcher-stage > img")
    })?;
    let (stage_x, stage_y, stage_w, stage_h) = rect(&ui, "#switcher-stage")?;
    let dimensions = chrome(&ui, "[$('switcher-stage').firstChild.naturalWidth, $('switcher-stage').firstChild.naturalHeight]")?;
    assert_eq!(
        dimensions,
        serde_json::json!([stage_w as i32, stage_h as i32])
    );
    assert!(stage_x > 0. && stage_y > 94.);
    assert!(text(&ui, "#switcher-caption").contains("Waiting, from Filmstrip source"));
    ui.window.resize(720, 800);
    wait("filmstrip at 720 px", || {
        rect(&ui, "#switcher-row").is_ok_and(|(_, _, w, _)| w < 710.)
    })?;
    assert_eq!(chrome(&ui, "[$('switcher-stage').firstChild.naturalWidth, $('switcher-stage').firstChild.naturalHeight]")?, dimensions);
    xdo(&["key", "Left"])?;
    xdo(&["key", "Left"])?;
    expect_selection(order.last().unwrap())?;
    assert!(chrome(&ui, "(() => { const row=$('switcher-row'), card=document.getElementById(row.getAttribute('aria-activedescendant')).getBoundingClientRect(), frame=row.getBoundingClientRect(); return row.scrollLeft > 0 && card.left >= frame.left && card.right <= frame.right; })()")? == true);
    ui.window.resize(1240, 300);
    wait("short filmstrip", || !visible(&ui, "#switcher-stage"))?;
    assert!(visible(&ui, "#switcher-row"));
    assert!(visible(&ui, "#switcher-keys"));
    cancel()?;
    ui.window.resize(1240, 800);
    pump(100);
    println!("PASS the filmstrip scrolls its selected card into view at 720 px, preserves picture pixels on resize and retains the row in a short window.");

    changed.store(true, Ordering::Relaxed);
    wait(
        "hidden page's own script applied the updated colour",
        || FILM_MUTATED.load(Ordering::Relaxed),
    )?;
    pump(200);
    held()?;
    wait("updated hidden picture delivered", || {
        chrome(
            &ui,
            "S.switcher?.rows.get(S.switcher.selected).el.dataset.picture",
        )
        .is_ok_and(|v| v == "ready")
    })?;
    film_pixel(&ui, FILM_CHANGED)?;
    for _ in 0..5 {
        xdo(&["key", "Right"])?;
    }
    expect_selection(&recent[0])?;
    wait("large picture has been mounted", || {
        visible(&ui, "#switcher-stage > img")
    })?;
    film_pixel(&ui, FILM_COLOURS[6])?;
    let started = Instant::now();
    xdo(&["keyup", "ctrl"])?;
    wait("retained read page paints after Ctrl-up", || {
        let Some(window) = ui.window.window() else {
            return false;
        };
        let Some(pixel) = window.pixbuf(1200, 500, 1, 1) else {
            return false;
        };
        pixel.read_pixel_bytes()[..3] == FILM_COLOURS[6]
            && ui
                .snapshot()
                .is_ok_and(|s| s.current.is_some_and(|id| id.to_string() == recent[0]))
    })?;
    println!(
        "FILMSTRIP Ctrl-up to selected page painted: {:.1} ms, including xdotool and polling",
        started.elapsed().as_secs_f64() * 1000.
    );
    activated(&ui, &recent[0])?;
    key("alt+Left")?;
    activated(&ui, &here.to_string())?;

    ui.pages.inject_recent_memory_sample(0);
    queue(&ui, "film12")?;
    let extra = waiting(&ui)?[0].parse()?;
    wait("eighth read fixture preloaded", || {
        ui.pages.loaded_visits().contains(&extra)
    })?;
    key("ctrl+Tab")?;
    activated(&ui, &extra.to_string())?;
    key("alt+Left")?;
    activated(&ui, &here.to_string())?;
    held()?;
    wait("all fourteen cards and large images fill", || {
        chrome(&ui, "S.switcher?.picturesPainted >= 0").is_ok_and(|v| v == true)
    })?;
    assert_eq!(film_choices()?.len(), 14);
    let old_url = chrome(&ui, "$('switcher-stage').firstChild.src")?;
    cancel()?;
    chrome(&ui, &format!("(() => {{ const img = new Image(); img.onload = () => window.oldFilmstripImage = 'loaded'; img.onerror = () => window.oldFilmstripImage = 'expired'; img.src = {old_url}; return true; }})()"))?;
    wait("the closed opening no longer serves its picture", || {
        chrome(&ui, "window.oldFilmstripImage ?? ''").is_ok_and(|v| v == "expired")
    })?;
    println!("PASS the maximum fourteen-page opening fills and its picture URL expires on close.");

    queue(&ui, "film-slow")?;
    let slow = waiting(&ui)?[0].clone();
    held()?;
    expect_selection(&slow)?;
    assert_eq!(
        text(&ui, "#switcher-stage .switcher-fallback > span:last-child"),
        "Not loaded"
    );
    assert_eq!(
        chrome(
            &ui,
            "$('switcher-stage').querySelector('img')?.closest('.fav') !== null"
        )?,
        true
    );
    xdo(&["keyup", "ctrl"])?;
    activated(&ui, &slow)?;
    key("alt+Left")?;
    activated(&ui, &here.to_string())?;
    println!("PASS composed colours match a never-shown waiting page, its scripted update, and a retained read page. An uncommitted page shows a card.");
    let new_waiting = waiting(&ui)?;
    held()?;
    let read = recent[0].clone();
    let (x, y, w, h) = rect(
        &ui,
        &format!(".switcher-option[data-id='{}']", new_waiting[1]),
    )?;
    let (x, y) = screen_point(
        &ui,
        &ui.chrome.native(),
        (x + w / 2.) as i32,
        (y + h / 2.) as i32,
    )?;
    move_to(x, y)?;
    expect_selection(&new_waiting[1])?;
    click_chrome(&ui, "#switcher-stage")?;
    activated(&ui, &new_waiting[1])?;
    xdo(&["keyup", "ctrl"])?;
    assert_eq!(ui.snapshot()?.current.unwrap().to_string(), new_waiting[1]);
    key("alt+Left")?;
    activated(&ui, &here.to_string())?;
    println!("PASS moving onto a thumbnail selects it and clicking the large preview opens it.");

    let pending = waiting(&ui)?;
    let head = pending[0].parse::<canopy::browser::VisitId>()?;
    let mut browser = canopy::browser::Browser::open(&directory.join("forest.sqlite3"))?;
    let parent_url = browser.visit(here)?.url;
    let children: Vec<_> = (0..3)
        .map(|i| {
            browser
                .queue_link(
                    here,
                    web_url(&format!("{base}/descendant/{i}")).unwrap(),
                    canopy::browser::SourceContext {
                        page_url: parent_url.clone(),
                        link_text: format!("Descendant {i}"),
                        citation_url: None,
                        excerpt: String::new(),
                    },
                )
                .unwrap()
        })
        .collect();
    drop(browser);
    let db = rusqlite::Connection::open(directory.join("forest.sqlite3"))?;
    for child in &children {
        db.execute(
            "UPDATE visits SET parent_id=?1 WHERE id=?2",
            [head.to_string(), child.to_string()],
        )?;
    }
    drop(db);
    queue(&ui, "film-slow")?;
    let before_delete = waiting(&ui)?;
    held()?;
    let shown = film_choices()?;
    assert_eq!(shown.len(), 14);
    assert_eq!(
        text(&ui, "#switcher-more"),
        format!("{} more waiting", before_delete.len() - 5)
    );
    let index = shown.iter().position(|id| id == &head.to_string()).unwrap();
    for _ in 1..index {
        xdo(&["key", "Right"])?;
    }
    expect_selection(&head.to_string())?;
    xdo(&["key", "Delete"])?;
    wait("waiting branch disappears from the filmstrip", || {
        film_choices().is_ok_and(|ids| !ids.contains(&head.to_string()))
    })?;
    let after_delete = film_choices()?;
    for id in &children {
        assert!(!after_delete.contains(&id.to_string()));
    }
    let successor = shown[index..]
        .iter()
        .chain(&shown[..index])
        .find(|id| after_delete.contains(id))
        .unwrap();
    expect_selection(successor)?;
    let state = ui.snapshot()?;
    for id in std::iter::once(&head).chain(&children) {
        assert!(state
            .visits
            .iter()
            .any(|v| v.id == *id && v.dismissed_at.is_some()));
    }
    assert_eq!(
        chrome(
            &ui,
            "S.switcher.rows.get(S.switcher.selected).el.getAttribute('aria-selected')"
        )?,
        "true"
    );
    cancel()?;
    held()?;
    xdo(&["key", "Left"])?;
    expect_selection(&here.to_string())?;
    xdo(&["key", "Delete"])?;
    expect_selection(&here.to_string())?;
    wait("current card remains after Delete", || {
        film_choices().is_ok_and(|ids| ids.first() == Some(&here.to_string()))
    })?;
    for _ in 0..10 {
        if selection()? == read {
            break;
        }
        xdo(&["key", "Right"])?;
    }
    expect_selection(&read)?;
    xdo(&["key", "Delete"])?;
    expect_selection(&read)?;
    cancel()?;
    println!("PASS Delete dismisses a waiting branch and its descendants, selects a survivor and leaves current and read cards alone.");
    ui.window.close();
    Ok(())
}

const FILM_COLOURS: [[u8; 3]; 13] = [
    [192, 50, 38],
    [41, 144, 185],
    [149, 70, 160],
    [172, 143, 77],
    [216, 115, 14],
    [52, 69, 197],
    [160, 55, 71],
    [107, 71, 51],
    [156, 117, 187],
    [166, 208, 81],
    [180, 111, 100],
    [26, 142, 72],
    [65, 141, 181],
];
const FILM_CHANGED: [u8; 3] = [220, 96, 141];
static FILM_MUTATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn film_pixel(ui: &Ui, expected: [u8; 3]) -> Result<()> {
    let (x, y, w, h) = rect(ui, "#switcher-stage")?;
    let (sx, sy) = screen_point(
        ui,
        &ui.chrome.native(),
        (x + w / 2.).round() as i32,
        (y + h / 2.).round() as i32,
    )?;
    let (_, ox, oy) = ui.window.window().unwrap().origin();
    let sample = || -> Result<()> {
        let pixbuf = ui
            .window
            .window()
            .unwrap()
            .pixbuf(sx - ox - 2, sy - oy - 2, 5, 5)
            .unwrap();
        let bytes = pixbuf.read_pixel_bytes();
        for y in 0..5 {
            for x in 0..5 {
                let offset = (y * pixbuf.rowstride() + x * pixbuf.n_channels()) as usize;
                assert!(
                    expected
                        .iter()
                        .zip(&bytes[offset..offset + 3])
                        .all(|(a, b)| a.abs_diff(*b) <= 4),
                    "expected {expected:?}, got {:?}",
                    &bytes[offset..offset + 3]
                );
            }
        }
        Ok(())
    };
    pump(34);
    sample()
}

fn film_inventory(
    roots: &[std::path::PathBuf],
) -> Result<std::collections::BTreeSet<std::path::PathBuf>> {
    fn walk(
        root: &std::path::Path,
        files: &mut std::collections::BTreeSet<std::path::PathBuf>,
    ) -> Result<()> {
        if !root.is_dir() {
            return Ok(());
        }
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(&entry.path(), files)?;
            }
            if kind.is_file() {
                files.insert(entry.path());
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeSet::new();
    for root in roots {
        walk(root, &mut files)?;
    }
    Ok(files)
}
