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
            b.set_pinned(tree.root, true)?;
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
        chrome(&ui, "document.querySelectorAll('.tab').length").is_ok_and(|v| v == 16)
    })?;
    settle(&ui)?;
    assert_eq!(chrome(&ui, "(() => { const el = document.getElementById('memory-dot'); const r = el.getBoundingClientRect(); return [-11, 11].every(dx => [-11, 11].every(dy => el.contains(document.elementFromPoint(r.x+r.width/2+dx, r.y+r.height/2+dy)))) })()")?, true);
    assert_eq!(chrome(&ui, "[...document.querySelectorAll('.tab:not([hidden]):not(.icon-only) .tab-settle')].every(el => el.offsetWidth >= 24 && el.offsetHeight >= 24)")?, true);
    println!("PASS memory dot and tab settle have 24 by 24 hit areas.");
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
        "(intent('select-tree', {id: S.frame.tabs[S.frame.tabs.length-1].id}), true)",
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
                const active = document.querySelector('.tab.on');
                const arrow = document.getElementById('tab-overflow');
                const plus = document.getElementById('new-tree');
                return !active.hidden && tabs.every(t => t.getBoundingClientRect().right <= (arrow.hidden ? plus : arrow).getBoundingClientRect().left)
                    && plus.getBoundingClientRect().right <= window.innerWidth
                    && tabs.every((t,i) => !i || tabs[i-1].getBoundingClientRect().right <= t.getBoundingClientRect().left);
            })()"#
                )?,
                true
            );
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
            assert_eq!(chrome(&ui, "(() => { const buttons = [...document.querySelectorAll('.overflow-tree button')]; const tab = document.querySelector('.tab.on'); const cross = tab.querySelector('[data-settle]'); renderTabs(); refreshPopover(); return buttons.every(b => b.isConnected) && tab.isConnected && cross.isConnected; })()")?, true);
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
        chrome(&ui, "document.querySelectorAll('.tab').length").is_ok_and(|v| v == 15)
    })?;
    println!("PASS settle in the overflow removes its tree.");
    ui.window.resize(820, 800);
    chrome(&ui, "(() => { const keep = S.frame.tabs.slice(0,4).map(t => t.id); for (const t of S.frame.tabs.slice(4)) intent('settle-tree', {id:t.id}); intent('rename-tree', {id:keep[0], name:'GTK documentation and application development'}); intent('select-tree', {id:keep[0]}); return true; })()")?;
    wait("four trees", || {
        chrome(&ui, "S.frame.tabs.length").is_ok_and(|v| v == 4)
    })?;
    pump(400);
    assert_eq!(chrome(&ui, "[...document.querySelectorAll('.tab')].every(t => !t.hidden && !t.classList.contains('icon-only'))")?, true);
    assert_eq!(
        chrome(&ui, "document.querySelector('.tab.on').offsetWidth")?,
        200
    );
    println!("PASS 2 four trees at 820px keep all titles and active gets 200px.");
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
    click_chrome(&ui, ".tab.on .tab-settle")?;
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
    chrome(
        ui,
        "(intent('select-tree', {id: S.frame.tabs[0].id}), true)",
    )?;
    pump(400);
    chrome(
        ui,
        "(intent('select-tree', {id: S.frame.tabs[1].id}), true)",
    )?;
    pump(500);
    chrome(
        ui,
        "(intent('select-tree', {id: S.frame.tabs[0].id}), true)",
    )?;
    pump(500);
    chrome(ui, "(() => { window.reviewFrames = []; const present = canopy.present; canopy.present = (...args) => { present(...args); const on = document.querySelector('.tab.on'); window.reviewFrames.push(!on.classList.contains('icon-only') && on.offsetWidth >= 110 && [...document.querySelectorAll('.tab:not(.on)')].every(t => t.hidden || t.classList.contains('icon-only'))); }; return true; })()")?;
    let target = chrome(
        ui,
        "document.querySelector('.tab.on + .tab.icon-only')?.dataset.id",
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
        chrome(ui, "document.querySelector('.tab.on').dataset.id")?,
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
    assert_eq!(chrome(ui, "(() => { const cross = document.querySelector('.tab.on .tab-settle'); const original = window.ipc.postMessage; let settled = false; window.ipc.postMessage = (message) => { settled ||= JSON.parse(message).kind === 'settle-tree'; }; const r = cross.getBoundingClientRect(); const event = (type, detail, x = r.x+12) => cross.dispatchEvent(new MouseEvent(type, {bubbles:true, button:0, detail, clientX:x, clientY:r.y+12})); cross.style.opacity = '0'; event('mousedown',1); cross.style.opacity = '1'; event('mouseup',1); event('click',1); event('mousedown',2); event('mouseup',2); event('click',2); event('mousedown',1,r.x-1); event('mouseup',1); event('click',1); window.ipc.postMessage = original; cross.style.opacity = ''; return !settled; })()")?, true);
    println!("PASS 1 invisible-at-press cross and second double-click cannot settle.");
    assert_eq!(
        chrome(
            ui,
            "getComputedStyle(document.querySelector('.tab-title')).textAlign"
        )?,
        "left"
    );
    println!("PASS 3 short tab titles align left.");
    hover_chrome(ui, ".tab.on .tab-settle")?;
    assert_eq!(chrome(ui, "(() => { const el = document.querySelector('.tab.on .tab-settle'); const inner = getComputedStyle(el, '::before'); return el.offsetWidth >= 24 && el.offsetHeight >= 24 && parseFloat(inner.width) >= 14 && parseFloat(inner.width) <= 16 && parseFloat(inner.height) >= 14 && parseFloat(inner.height) <= 16 && getComputedStyle(el).backgroundColor === 'rgba(0, 0, 0, 0)'; })()")?, true);
    println!("PASS 4 settle hover draws a 16px square inside its 24px hit area.");
    move_to(1000, 900)?;
    for width in [820, 560, 480] {
        ui.window.resize(width, 800);
        pump(400);
        assert_eq!(chrome(ui, "(() => { const tab = document.querySelector('.tab.on'); return tab.offsetWidth === 200 && tab.querySelector('.tab-title').clientWidth >= 130; })()")?, true, "active natural title at {width}px");
    }
    println!("PASS 2 active title keeps 200px at 820, 560 and 480px.");
    chrome(ui, "(intent('select-tree', {id:S.frame.tabs[0].id}), true)")?;
    pump(300);
    ui.window.resize(1240, 800);
    pump(400);
    assert_eq!(chrome(ui, "[...document.querySelectorAll('.tab.icon-only')].some(t => { const tree = S.frame.tabs.find(tree => tree.id === t.dataset.id); const count = t.querySelector('.count'); return tree.pending > 0 && count && count.offsetWidth > 0 && count.textContent === String(tree.pending) && getComputedStyle(count).position === 'absolute' && !t.querySelector('.tab-pinned-mark')?.offsetWidth; })")?, true);
    println!("PASS 5 icon corner shows its waiting count and hides the pin.");
    if let Ok(folder) = std::env::var("CANOPY_TAB_SHOTS") {
        std::fs::create_dir_all(&folder)?;
        for theme in ["dark", "light"] {
            chrome(ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
            pump(200);
            hover_chrome(ui, ".tab.on .tab-settle")?;
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
