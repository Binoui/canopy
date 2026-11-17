use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{Browser, Navigation, NavigationTarget, Visibility},
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
    ui.window.close();
    Ok(())
}
