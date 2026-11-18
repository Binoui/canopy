use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{Browser, Navigation, NavigationTarget, SourceContext, Visibility},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
#[path = "common/http.rs"]
mod http;
use common::*;

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let data = std::env::var_os("CANOPY_TAB_FIXTURE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().to_owned());
    if let Ok(seed) = std::env::var("CANOPY_TAB_SEED") {
        drop(Browser::open(&data.join("forest.sqlite3"))?);
        assert!(std::process::Command::new("python3")
            .args([
                seed,
                data.join("forest.sqlite3").to_string_lossy().into_owned(),
                std::env::var("CANOPY_TAB_PORT")?
            ])
            .status()?
            .success());
    } else if std::env::var_os("CANOPY_TAB_FIXTURE").is_none() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        std::thread::spawn(move || {
            use std::io::Write;
            for mut stream in listener.incoming().flatten() {
                if http::request(&mut stream).is_err() {
                    continue;
                }
                let body =
                    "<!doctype html><title>A tree title long enough to shrink</title><p>A tree</p>";
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        let mut b = Browser::open(&data.join("forest.sqlite3"))?;
        let mut n = Navigation::default();
        for i in 0..16 {
            let url = web_url(&format!("http://{address}/tree-{i}"))?;
            b.navigate(&mut n, url.clone(), NavigationTarget::Root)?;
            let ticket = n.pending_ticket().unwrap();
            n.started(ticket);
            b.commit_load(&mut n, ticket, url, Visibility::Selected)?;
        }
    }
    if std::env::var_os("CANOPY_TAB_FIXTURE").is_none()
        && std::env::var_os("CANOPY_TAB_SEED").is_none()
    {
        let mut b = Browser::open(&data.join("forest.sqlite3"))?;
        for tree in b.tree_states()? {
            b.rename_tree(tree.root, "A tree title long enough to shrink")?;
            let page = b
                .snapshot()?
                .visits
                .into_iter()
                .find(|v| v.id == tree.last_visit)
                .unwrap();
            b.queue_link(
                page.id,
                page.url.clone(),
                SourceContext {
                    page_url: page.url,
                    citation_url: None,
                    link_text: "Waiting page".into(),
                    excerpt: String::new(),
                },
            )?;
        }
    }
    let ui = Ui::new(&data)?;
    ui.window.show_all();
    ready(&ui)?;
    wait("sixteen tabs", || {
        chrome(&ui, "document.querySelectorAll('#tab-row .tab').length").is_ok_and(|v| v == 16)
    })?;
    let ui = tab_reorder(ui, &data)?;
    settle(&ui)?;
    assert_eq!(chrome(&ui, "(() => { const el = document.getElementById('memory-dot'); const r = el.getBoundingClientRect(); return [-11, 11].every(dx => [-11, 11].every(dy => el.contains(document.elementFromPoint(r.x+r.width/2+dx, r.y+r.height/2+dy)))) })()")?, true);
    assert_eq!(chrome(&ui, "[...document.querySelectorAll('.tab:not([hidden]):not(.icon-only) .tab-settle')].every(el => el.offsetWidth >= 24 && el.offsetHeight >= 24)")?, true);
    println!("PASS saved-text dot and tab settle have 24 by 24 hit areas.");
    for width in [1240, 480] {
        ui.window.resize(width, 800);
        pump(300);
        key("ctrl+t")?;
        wait("New tree stays outside overflow", || {
            visible(&ui, "#new-tree-tab[aria-selected=true]")
        })?;
        pump(300);
        assert_eq!(chrome(&ui, "(() => { const tab=document.getElementById('new-tree-tab'); const plus=document.getElementById('new-tree'); const r=tab.getBoundingClientRect(); const trees=[...document.querySelectorAll('#tab-row .tab')]; return r.width >= 110 && r.right <= plus.getBoundingClientRect().left && plus.getBoundingClientRect().right <= window.innerWidth && trees.every(t=>t.hidden || t.getBoundingClientRect().right <= r.left) && !trees.some(t=>t.getAttribute('aria-selected')==='true') && trees.filter(t=>!t.hidden && t.tabIndex===0).length===1; })()")?, true);
        if width == 480 {
            assert_eq!(chrome(&ui, "[...document.querySelectorAll('#tab-row .tab')].every(t=>t.hidden || t.classList.contains('icon-only'))")?, true);
        }
        key("ctrl+w")?;
        wait("reading tab selected again", || {
            visible(&ui, "#tab-row [aria-selected=true]") && !visible(&ui, "#new-tree-tab")
        })?;
    }
    println!("PASS New tree remains visible at 1240 and 480 pixels while tree tabs collapse and retain a keyboard stop.");
    review_regressions(&ui)?;
    let original = ui.snapshot()?.current;
    key("ctrl+2")?;
    wait("second tree selected", || {
        ui.snapshot().unwrap().current != original
    })?;
    key("ctrl+shift+Tab")?;
    wait("previous tree", || {
        ui.snapshot().unwrap().current == original
    })?;
    key("ctrl+shift+Tab")?;
    wait("previous tree toggles", || {
        ui.snapshot().unwrap().current != original
    })?;
    println!("PASS Ctrl+Shift+Tab alternates between previously shown trees.");
    chrome(
        &ui,
        "(treeAct(S.frame.tabs[S.frame.tabs.length-1].id,'open'), true)",
    )?;
    wait("last tree active", || {
        chrome(&ui, "S.frame.tabs[S.frame.tabs.length-1].active").is_ok_and(|v| v == true)
    })?;
    for theme in ["dark", "light"] {
        chrome(&ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
        pump(250);
        for width in [1240, 820, 560, 480] {
            ui.window.resize(width, 800);
            move_to(1100, 900)?;
            pump(500);
            assert_eq!(chrome(&ui, "window.innerWidth")?, width);
            assert_eq!(
                chrome(
                    &ui,
                    r#"(() => {
                const tabs = [...document.querySelectorAll('.tab')].filter(t => !t.hidden);
                const active = document.querySelector('.tab[aria-selected=true]');
                const arrow = document.getElementById('tab-overflow');
                const plus = document.getElementById('new-tree');
                return !active.hidden && tabs.every(t => t.getBoundingClientRect().right <= (arrow.hidden ? plus : arrow).getBoundingClientRect().left)
                    && plus.getBoundingClientRect().right <= window.innerWidth
                    && tabs.every((t,i) => !i || tabs[i-1].getBoundingClientRect().right <= t.getBoundingClientRect().left);
            })()"#
                )?,
                true
            );
            assert_eq!(chrome(&ui, "(() => { const tabs=[...document.querySelectorAll('#tab-row .tab')];const stops=tabs.filter(t=>t.tabIndex===0);return stops.length===1 && !stops[0].hidden && tabs.every(t=>t.getAttribute('role')==='tab' && !!t.getAttribute('aria-label')); })()")?, true);
            if let Ok(folder) = std::env::var("CANOPY_TAB_SHOTS") {
                std::fs::create_dir_all(&folder)?;
                std::process::Command::new("import")
                    .args(["-window", "root", &format!("{folder}/{width}-{theme}.png")])
                    .status()?;
            }
            if chrome(&ui, "document.getElementById('tab-overflow').hidden")? == true {
                assert_eq!(
                    chrome(
                        &ui,
                        "[...document.querySelectorAll('.tab')].filter(t => !t.hidden).length"
                    )?,
                    16
                );
                println!("PASS {width}px {theme} all sixteen trees fit before overflow.");
                continue;
            }
            click_chrome(&ui, "#tab-overflow")?;
            wait("overflow rows", || {
                chrome(
                    &ui,
                    "document.querySelectorAll('.overflow-settle').length > 0",
                )
                .is_ok_and(|v| v == true)
            })?;
            assert_eq!(chrome(&ui, "[...document.querySelectorAll('.overflow-settle')].every(b => b.offsetWidth >= 24 && b.offsetHeight >= 24)")?, true);
            assert_eq!(chrome(&ui, "(() => { const buttons = [...document.querySelectorAll('.overflow-tree button')]; const tab = document.querySelector('.tab[aria-selected=true]'); const cross = tab.querySelector('[data-settle]'); renderTabs(); refreshPopover(); return buttons.every(b => b.isConnected) && tab.isConnected && cross.isConnected; })()")?, true);
            assert_eq!(chrome(&ui, "[...document.querySelectorAll('.overflow-tree')].every(row => { const tree = S.frame.tabs.find(t => t.id === row.dataset.tree); return !!row.querySelector('.count') === !!tree.pending && !!row.querySelector('.tab-pinned-mark') === tree.pinned; })")?, true);
            if let Ok(folder) = std::env::var("CANOPY_TAB_SHOTS") {
                std::process::Command::new("import")
                    .args([
                        "-window",
                        "root",
                        &format!("{folder}/{width}-{theme}-overflow.png"),
                    ])
                    .status()?;
            }
            key("Escape")?;
            println!("PASS {width}px {theme} active tab, overflow and new-tree bounds.");
        }
    }
    click_chrome(&ui, "#tab-overflow")?;
    click_chrome(&ui, ".overflow-settle")?;
    wait("overflow settle removes one tree", || {
        chrome(&ui, "document.querySelectorAll('#tab-row .tab').length").is_ok_and(|v| v == 15)
    })?;
    println!("PASS settle in the overflow removes its tree.");
    ui.window.resize(820, 800);
    chrome(&ui, "(() => { const keep = S.frame.tabs.slice(0,4).map(t => t.id); for (const t of S.frame.tabs.slice(4)) treeAct(t.id,'settle'); intent('rename-tree', {id:keep[0], name:'GTK documentation and application development'}); treeAct(keep[0],'open'); return true; })()")?;
    wait("four trees", || {
        chrome(&ui, "S.frame.tabs.length").is_ok_and(|v| v == 4)
    })?;
    pump(400);
    assert_eq!(chrome(&ui, "[...document.querySelectorAll('#tab-row .tab')].every(t => !t.hidden && !t.classList.contains('icon-only'))")?, true);
    assert_eq!(
        chrome(
            &ui,
            "(() => { const on = document.querySelector('.tab[aria-selected=true]').offsetWidth; return on >= 200 && on <= 280 && [...document.querySelectorAll('.tab')].every(t => t.offsetWidth <= on); })()"
        )?,
        true
    );
    println!(
        "PASS 2 four trees at 820px keep all titles and active gets 200 to 280px, the widest."
    );
    if let Ok(folder) = std::env::var("CANOPY_TAB_SHOTS") {
        for theme in ["dark", "light"] {
            chrome(&ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
            move_to(1100, 900)?;
            pump(250);
            std::process::Command::new("import")
                .args(["-window", "root", &format!("{folder}/four-820-{theme}.png")])
                .status()?;
        }
    }
    click_chrome(&ui, ".tab[aria-selected=true] .tab-settle")?;
    wait("visible settle cross removes one tree", || {
        chrome(&ui, "S.frame.tabs.length").is_ok_and(|v| v == 3)
    })?;
    println!("PASS 1 single click on a visible settle cross settles its tree.");
    ui.window.close();
    Ok(())
}

fn review_regressions(ui: &Ui) -> Result<()> {
    ui.window.resize(820, 800);
    pump(400);
    chrome(ui, "(treeAct(S.frame.tabs[0].id,'open'), true)")?;
    pump(400);
    chrome(ui, "(treeAct(S.frame.tabs[1].id,'open'), true)")?;
    pump(500);
    chrome(ui, "(treeAct(S.frame.tabs[0].id,'open'), true)")?;
    pump(500);
    chrome(ui, "(() => { window.reviewFrames = []; const present = canopy.present; canopy.present = (...args) => { present(...args); const on = document.querySelector('.tab[aria-selected=true]'); window.reviewFrames.push(!on.classList.contains('icon-only') && on.offsetWidth >= 110 && [...document.querySelectorAll('.tab:not([aria-selected=true])')].every(t => t.hidden || t.classList.contains('icon-only'))); }; return true; })()")?;
    let target = chrome(
        ui,
        "document.querySelector('.tab[aria-selected=true] + .tab.icon-only')?.dataset.id",
    )?;
    let target = target.as_str().expect("icon immediately right of active");
    let selector = format!(".tab[data-id='{target}']");
    let (x, y) = chrome_point(ui, &selector)?;
    move_to(x, y)?;
    xdo(&["click", "--repeat", "2", "--delay", "100", "1"])?;
    pump(500);
    assert_eq!(
        chrome(ui, "S.frame.tabs.length")?,
        16,
        "double-click must not settle a tree"
    );
    assert_eq!(
        chrome(
            ui,
            "document.querySelector('.tab[aria-selected=true]').dataset.id"
        )?,
        target
    );
    println!("PASS 1 double-click on the icon right of active keeps all sixteen trees.");
    assert_eq!(
        chrome(
            ui,
            "window.reviewFrames.length > 0 && window.reviewFrames.every(Boolean)"
        )?,
        true
    );
    println!("PASS 7 active class and titled fit change in the same presentation call.");
    assert_eq!(chrome(ui, "(() => { const cross = document.querySelector('.tab[aria-selected=true] .tab-settle'); const original = window.ipc.postMessage; let settled = false; window.ipc.postMessage = (message) => { settled ||= JSON.parse(message).act === 'settle'; }; const r = cross.getBoundingClientRect(); const event = (type, detail, x = r.x+12) => cross.dispatchEvent(new MouseEvent(type, {bubbles:true, button:0, detail, clientX:x, clientY:r.y+12})); cross.style.opacity = '0'; event('mousedown',1); cross.style.opacity = '1'; event('mouseup',1); event('click',1); event('mousedown',2); event('mouseup',2); event('click',2); event('mousedown',1,r.x-1); event('mouseup',1); event('click',1); window.ipc.postMessage = original; cross.style.opacity = ''; return !settled; })()")?, true);
    println!("PASS 1 invisible-at-press cross and second double-click cannot settle.");
    assert_eq!(
        chrome(
            ui,
            "getComputedStyle(document.querySelector('.tab-title')).textAlign"
        )?,
        "left"
    );
    println!("PASS 3 short tab titles align left.");
    hover_chrome(ui, ".tab[aria-selected=true] .tab-settle")?;
    assert_eq!(chrome(ui, "(() => { const el = document.querySelector('.tab[aria-selected=true] .tab-settle'); const inner = getComputedStyle(el, '::before'); return el.offsetWidth >= 24 && el.offsetHeight >= 24 && parseFloat(inner.width) >= 14 && parseFloat(inner.width) <= 16 && parseFloat(inner.height) >= 14 && parseFloat(inner.height) <= 16 && getComputedStyle(el).backgroundColor === 'rgba(0, 0, 0, 0)'; })()")?, true);
    println!("PASS 4 settle hover draws a 16px square inside its 24px hit area.");
    move_to(1000, 900)?;
    for width in [820, 560, 480] {
        ui.window.resize(width, 800);
        pump(400);
        assert_eq!(chrome(ui, "(() => { const tab = document.querySelector('.tab[aria-selected=true]'); return tab.offsetWidth >= 200 && tab.offsetWidth <= 280 && tab.querySelector('.tab-title').clientWidth >= 110; })()")?, true, "active natural title at {width}px");
    }
    println!("PASS 2 active title keeps 200 to 280px at 820, 560 and 480px.");
    chrome(ui, "(treeAct(S.frame.tabs[0].id,'open'), true)")?;
    pump(300);
    ui.window.resize(1240, 800);
    pump(400);
    assert_eq!(chrome(ui, "[...document.querySelectorAll('.tab.icon-only')].some(t => { const tree = S.frame.tabs.find(tree => tree.id === t.dataset.id); const dot = t.querySelector('.unread-dot'); return tree.pending > 0 && dot && dot.offsetWidth > 0 && getComputedStyle(dot).position === 'absolute' && !t.querySelector('.tab-pinned-mark')?.offsetWidth; })")?, true);
    println!("PASS 5 icon corner shows its waiting count and hides the pin.");
    if let Ok(folder) = std::env::var("CANOPY_TAB_SHOTS") {
        std::fs::create_dir_all(&folder)?;
        for theme in ["dark", "light"] {
            chrome(ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
            pump(200);
            hover_chrome(ui, ".tab[aria-selected=true] .tab-settle")?;
            pump(80);
            std::process::Command::new("import")
                .args([
                    "-window",
                    "root",
                    &format!("{folder}/waiting-hover-{theme}.png"),
                ])
                .status()?;
        }
    }
    Ok(())
}

fn stored_tabs(data: &std::path::Path) -> Result<Vec<String>> {
    let db = rusqlite::Connection::open_with_flags(
        data.join("forest.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let tabs = db
        .prepare("SELECT root_id FROM tree_state WHERE place='tab' ORDER BY pinned DESC,position,root_id")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tabs)
}

fn tab_ids(ui: &Ui) -> Result<Vec<String>> {
    Ok(serde_json::from_value(chrome(
        ui,
        "S.frame.tabs.map(t => t.id)",
    )?)?)
}

fn native_drag(ui: &Ui, source: &str, end: (i32, i32), on_row: bool) -> Result<()> {
    let start = chrome_point(ui, &format!(".tab[data-id='{source}'] .mk"))?;
    move_to(start.0, start.1)?;
    pump(150);
    xdo(&["mousedown", "1"])?;
    pump(200);
    for step in 1..=25 {
        move_to(
            start.0 + (end.0 - start.0) * step / 25,
            start.1 + (end.1 - start.1) * step / 25,
        )?;
        pump(60);
    }
    pump(200);
    let indicator = chrome(
        ui,
        "document.getElementById('tab-row').classList.contains('drop-target')",
    )?;
    let dimmed = chrome(ui, "!!document.querySelector('.tab.dragging')")?;
    xdo(&["mouseup", "1"])?;
    assert_eq!(indicator, on_row, "drop indicator follows the native drag");
    assert_eq!(dimmed, true, "source stays dimmed during the native drag");
    pump(400);
    assert_eq!(chrome(ui, "!!document.querySelector('.tab.dragging') || document.getElementById('tab-row').classList.contains('drop-target')")?, false);
    Ok(())
}

fn tab_reorder(ui: std::rc::Rc<Ui>, data: &std::path::Path) -> Result<std::rc::Rc<Ui>> {
    ui.window.resize(1240, 800);
    key("ctrl+1")?;
    pump(400);
    let original = tab_ids(&ui)?;
    key("ctrl+shift+Page_Up")?;
    assert_eq!(stored_tabs(data)?, original);
    let outside = page_point(&ui, 400, 400)?;
    native_drag(&ui, &original[0], outside, false)?;
    assert_eq!(
        stored_tabs(data)?,
        original,
        "a drop on the page cannot reorder"
    );
    let (x, y, _, h) = rect(&ui, &format!(".tab[data-id='{}']", original[3]))?;
    let gap = screen_point(
        &ui,
        &ui.chrome.native(),
        (x - 1.) as i32,
        (y + h / 2.) as i32,
    )?;
    native_drag(&ui, &original[0], gap, true)?;
    let mut expected = original.clone();
    let moved = expected.remove(0);
    expected.insert(2, moved);
    wait("native drop reordered the strip", || {
        tab_ids(&ui).is_ok_and(|tabs| tabs == expected)
    })?;
    assert_eq!(stored_tabs(data)?, expected);
    assert_eq!(
        chrome(
            &ui,
            "document.querySelector('.tab[aria-selected=true]').dataset.id"
        )?,
        original[0]
    );
    key("ctrl+shift+Page_Up")?;
    expected.swap(1, 2);
    wait("keyboard move left", || {
        stored_tabs(data).is_ok_and(|tabs| tabs == expected)
    })?;
    key("ctrl+shift+Page_Down")?;
    expected.swap(1, 2);
    wait("keyboard move right", || {
        stored_tabs(data).is_ok_and(|tabs| tabs == expected)
    })?;
    println!("PASS native drag reorders, preserves selection and shows an indicator; page drops do nothing; keyboard moves use the stored order.");

    for id in &expected[..2] {
        hover_chrome(&ui, &format!(".tab[data-id='{id}']"))?;
        xdo(&["click", "3"])?;
        click_chrome(&ui, "#popover [data-tab-action=pin]")?;
        pump(200);
    }
    key("ctrl+2")?;
    wait("second pinned tab committed", || {
        ui.snapshot().is_ok_and(|snapshot| {
            snapshot
                .current
                .is_some_and(|id| id.to_string() == expected[1])
        })
    })?;
    key("ctrl+shift+Page_Down")?;
    assert_eq!(
        stored_tabs(data)?,
        expected,
        "last pinned tab stops at the border"
    );
    key("ctrl+shift+Page_Up")?;
    expected.swap(0, 1);
    wait("keyboard moves within pinned tabs", || {
        stored_tabs(data).is_ok_and(|tabs| tabs == expected)
    })?;
    key("ctrl+shift+Page_Down")?;
    expected.swap(0, 1);
    wait("keyboard returns within pinned tabs", || {
        stored_tabs(data).is_ok_and(|tabs| tabs == expected)
    })?;
    let source = expected[3].clone();
    let (x, y, _, h) = rect(&ui, &format!(".tab[data-id='{}']", expected[1]))?;
    let gap = screen_point(
        &ui,
        &ui.chrome.native(),
        (x + 2.) as i32,
        (y + h / 2.) as i32,
    )?;
    native_drag(&ui, &source, gap, true)?;
    let moved = expected.remove(3);
    expected.insert(1, moved);
    wait("drop among pinned tabs pins the source", || {
        tab_ids(&ui).is_ok_and(|tabs| tabs == expected)
    })?;
    assert_eq!(
        chrome(
            &ui,
            &format!("S.frame.tabs.find(t=>t.id==='{source}').pinned")
        )?,
        true
    );
    let (x, y, _, h) = rect(&ui, &format!(".tab[data-id='{}']", expected[5]))?;
    let gap = screen_point(
        &ui,
        &ui.chrome.native(),
        (x + 2.) as i32,
        (y + h / 2.) as i32,
    )?;
    native_drag(&ui, &source, gap, true)?;
    let moved = expected.remove(1);
    expected.insert(4, moved);
    wait("pinned tab dragged out becomes unpinned", || {
        tab_ids(&ui).is_ok_and(|tabs| tabs == expected)
    })?;
    assert_eq!(
        chrome(
            &ui,
            &format!("S.frame.tabs.find(t=>t.id==='{source}').pinned")
        )?,
        false
    );
    assert_eq!(stored_tabs(data)?, expected);
    println!("PASS real drags pin among pinned tabs and unpin outside; keyboard moves stay inside the pinned group.");
    for id in &expected[..2] {
        hover_chrome(&ui, &format!(".tab[data-id='{id}']"))?;
        xdo(&["click", "3"])?;
        click_chrome(&ui, "#popover [data-tab-action=unpin]")?;
        pump(200);
    }
    expected = stored_tabs(data)?;
    ui.window.resize(480, 800);
    key("ctrl+8")?;
    pump(400);
    let visible: Vec<String> = serde_json::from_value(chrome(
        &ui,
        "[...document.querySelectorAll('#tab-row .tab:not([hidden])')].map(t => t.dataset.id)",
    )?)?;
    assert!(visible.len() >= 2 && visible.len() < expected.len());
    let source = visible.last().unwrap();
    let target = &visible[0];
    let from = expected.iter().position(|id| id == source).unwrap();
    let to = expected.iter().position(|id| id == target).unwrap();
    let active = chrome(
        &ui,
        "document.querySelector('.tab[aria-selected=true]').dataset.id",
    )?;
    let (x, y, _, h) = rect(&ui, &format!(".tab[data-id='{target}']"))?;
    let gap = screen_point(
        &ui,
        &ui.chrome.native(),
        (x + 2.) as i32,
        (y + h / 2.) as i32,
    )?;
    native_drag(&ui, source, gap, true)?;
    let moved = expected.remove(from);
    expected.insert(to, moved);
    wait("overflow drag uses full indexes", || {
        tab_ids(&ui).is_ok_and(|tabs| tabs == expected)
    })?;
    assert_eq!(stored_tabs(data)?, expected);
    assert_eq!(
        chrome(
            &ui,
            "document.querySelector('.tab[aria-selected=true]').dataset.id"
        )?,
        active
    );
    println!("PASS dragging visible tabs with overflow preserves the hidden tabs' order and the active tree.");
    ui.window.close();
    drop(ui);
    pump(200);
    let ui = Ui::new(data)?;
    ui.window.show_all();
    ready(&ui)?;
    wait("restarted strip keeps tab order", || {
        tab_ids(&ui).is_ok_and(|tabs| tabs == expected)
    })?;
    assert_eq!(stored_tabs(data)?, expected);
    println!("PASS stored tab order survives closing and reopening the UI.");
    Ok(ui)
}
