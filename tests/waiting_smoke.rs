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
    assert!(!visible(&ui, "#waiting-pin"));
    for i in 0..3 {
        queue(&ui, &format!("link{i}"))?;
        if i == 0 {
            assert_eq!(
                chrome(&ui, "getComputedStyle($('waiting-pin')).animationDuration")?,
                "0.32s"
            );
            assert_eq!(
                chrome(
                    &ui,
                    "getComputedStyle(document.querySelector('.node.after')).animationDuration"
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
        assert!(text(&ui, ".node.after").contains(&(i + 1).to_string()));
    }
    composed_waiting_toast(&ui)?;
    println!("PASS middle-click announces link text, batches three waiting links and updates the After count.");
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
    assert_eq!(text(&ui, "#tab-row .tab[aria-selected=true] .count"), "26");
    wait("newest waiting page loaded", || {
        visible(&ui, "#waiting-pin .pending-dot.loaded")
    })?;
    assert!(chrome(&ui, "document.querySelector('#waiting-pin img').getAttribute('src').includes(encodeURIComponent(S.frame.visits[canopy.state().waiting[0]].url))")? == true);
    assert!(chrome(&ui, "S.strip.nodes.some(n => n.kind === 'after' && n.waiting === 24 && n.rows.length === 6 && n.more.includes('18 more waiting'))")? == true);
    println!(
        "PASS counter, next icon, state dot, tab count, newest order and hidden waiting pages."
    );
    for width in [1240, 800, 1440] {
        ui.window.resize(width, 800);
        pump(400);
        settle(&ui)?;
        room_check(&ui)?;
        open_list(&ui)?;
        room_check(&ui)?;
        leave_strip(&ui)?;
    }
    ui.window.resize(800, 800);
    pump(400);
    settle(&ui)?;
    open_list(&ui)?;
    let pin = rect(&ui, "#waiting-pin")?;
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
    assert_eq!(rect(&ui, "#waiting-pin")?, pin);
    leave_strip(&ui)?;
    println!("PASS pinned counter and fitted strip stay still while scrolling; current pill fits reserved room at three widths.");
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
        shots(&ui, &format!("{theme}-counter"))?;
        hover_waiting(&ui)?;
        wait("waiting list", || visible(&ui, "#popover.waiting"))?;
        assert!(text(&ui, ".pop-head").contains("Waiting in this tree · 26"));
        assert!(chrome(&ui, "!$('waiting-pin').hasAttribute('title')")? == true);
        assert!(chrome(&ui, "[...document.querySelectorAll('#popover .menu-detail')].some(e => e.textContent.startsWith('Waiting article'))")? == true);
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
        assert!(chrome(&ui, "[...document.querySelectorAll('#popover .menu-row .menu-detail')].every(e => e.textContent === S.frame.visits[e.closest('.menu-row').dataset.id].detail)")? == true);
        assert_eq!(strip_height(&ui), 52);
        assert!(chrome(&ui, "(() => {const d=document.querySelector('#popover .mk .pending-dot').getBoundingClientRect(), i=document.querySelector('#popover .mk .fav').getBoundingClientRect();return d.x+d.width/2 > i.x+i.width/2 && d.y+d.height/2 < i.y+i.height/2})()")? == true);
        shots(&ui, &format!("{theme}-list"))?;
        chrome(&ui, "(canopy.present(S.frame), true)")?;
        assert!(visible(&ui, "#popover.waiting"));
        assert!(chrome(&ui, "!$('waiting-pin').hasAttribute('title')")? == true);
        chrome(&ui, "(() => { window.unloadedFrame = structuredClone(S.frame); const v=S.frame.visits[S.frame.waiting.ids[0]]; v.loaded=true; v.when='loaded'; v.detail='loaded'; const u=S.frame.visits[S.frame.waiting.ids[1]]; u.loaded=false; u.when=''; u.detail=''; canopy.present(S.frame); return true; })()")?;
        assert!(visible(&ui, "#waiting-pin .pending-dot.loaded"));
        assert!(
            chrome(
                &ui,
                "!!document.querySelector('#popover .mk .pending-dot.loaded')"
            )? == true
        );
        chrome(&ui, "(canopy.toggleForest(), true)")?;
        wait("forest data", || visible(&ui, ".forest-node.pending"))?;
        chrome(&ui, "(() => { const v=S.frame.visits[S.frame.waiting.ids[0]]; v.loaded=true; v.when='loaded'; const u=S.frame.visits[S.frame.waiting.ids[1]]; u.loaded=false; u.when='';u.detail=''; canopy.present(S.frame); return true; })()")?;
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
        chrome(&ui, "canopy.state().pins")?,
        serde_json::json!(["zoom"])
    );
    assert!(
        chrome(&ui, "canopy.state().room")?.as_f64().unwrap()
            < f64::from(ui.window.allocated_width())
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
    println!("PASS counter hides and only the zoom pin reserves room at zero; empty shortcut shows toast.");
    small_tree_shots(&ui, &base)?;
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

fn prove_tree_sheet(ui: &Ui) -> Result<()> {
    let original = ui.snapshot()?.current.unwrap();
    let pin = rect(ui, "#zoom-pin")?;
    let waiting = rect(ui, "#waiting-pin")?;
    assert!(pin.0 > waiting.0 + waiting.2);
    assert!(pin.0 + pin.2 <= f64::from(ui.window.allocated_width()) - 7.5);
    key("ctrl+t")?;
    key("ctrl+shift+e")?;
    wait("tree sheet from start", || visible(ui, "#tree-sheet"))?;
    assert_eq!(chrome(ui, "canopy.state().surface")?, "reading");
    assert_eq!(chrome(ui, "canopy.state().overlay")?, "tree");
    assert_eq!(rect(ui, "#zoom-pin")?, pin);
    assert!(visible(ui, "#zoom-pin[aria-pressed=true]"));
    assert!(!visible(ui, "#waiting-pin"));
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
