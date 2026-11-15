use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{NavigationTarget, TreeId, VisitId},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
use webkit2gtk::WebViewExt;
mod common;
use common::*;

fn tree(ui: &Ui, root: VisitId) -> TreeId {
    ui.pages
        .read(|b| b.tree_states())
        .unwrap()
        .into_iter()
        .find(|t| t.root.root() == root)
        .unwrap()
        .root
}

fn rename(ui: &Ui, root: VisitId, name: &str) {
    ui.dispatch(ChromeIntent::RenameTree(tree(ui, root), name.into()));
    pump(250);
}

fn press(ui: &Ui, selector: &str) -> Result<()> {
    hover_chrome(ui, selector)?;
    xdo(&["mousedown", "1"])?;
    pump(80);
    Ok(())
}

fn held_back(ui: &Ui) -> Result<()> {
    wait("a frame is held back while the button is down", || {
        chrome(ui, "canopy.state().deferred.includes('present')").is_ok_and(|held| held == true)
    })
}

fn release() -> Result<()> {
    xdo(&["mouseup", "1"])?;
    pump(600);
    Ok(())
}

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = [0; 4096];
            let Ok(n) = stream.read(&mut request) else {
                continue;
            };
            let request = String::from_utf8_lossy(&request[..n]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let body = format!("<!doctype html><title>Reading {path}</title><body>fixture");
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.resize(1100, 700);
    ui.window.show_all();
    ready(&ui)?;
    let mut roots = Vec::new();
    for name in ["alpha", "beta", "gamma"] {
        ui.pages.selected_page().navigate(
            web_url(&format!("http://{address}/{name}"))?,
            NavigationTarget::Root,
        )?;
        wait("root loaded", || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.title == format!("Reading /{name}"))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        roots.push(ui.snapshot()?.current.unwrap());
    }
    pump(500);
    let (alpha, beta, gamma) = (roots[0], roots[1], roots[2]);
    press(&ui, &format!(".tab[data-id='{alpha}']"))?;
    for title in ["First refresh", "Second refresh", "Last refresh"] {
        rename(&ui, gamma, title);
    }
    held_back(&ui)?;
    let held = !text(&ui, "#tab-row").contains("Last refresh");
    release()?;
    let tab = ui.snapshot()?.current == Some(alpha);
    println!("tab click across three refreshes: {tab}");
    wait("latest deferred title", || {
        text(&ui, "#tab-row").contains("Last refresh")
    })?;
    println!("latest deferred title appears after release: true");

    hover_chrome(&ui, &format!(".tab[data-id='{beta}']"))?;
    xdo(&["click", "3"])?;
    wait("tab menu", || {
        visible(&ui, "#popover [data-tab-action='pin']")
    })?;
    press(&ui, "#popover [data-tab-action='pin']")?;
    rename(&ui, gamma, "Menu refresh");
    held_back(&ui)?;
    release()?;
    let menu = ui
        .pages
        .read(|b| b.tree_states())?
        .iter()
        .any(|t| t.root.root() == beta && t.pin.is_some());
    println!("menu row click across refresh: {menu}");
    key("Escape")?;
    if !menu {
        ui.dispatch(ChromeIntent::PinTree {
            tree: tree(&ui, beta),
            pinned: true,
        });
        pump(300);
    }

    key("ctrl+t")?;
    wait("pinned card", || {
        visible(&ui, &format!("[data-start=tree][data-id='{beta}']"))
    })?;
    press(&ui, &format!("[data-start=tree][data-id='{beta}']"))?;
    rename(&ui, beta, "Card refresh");
    chrome(&ui, "canopy.icons(1000)")?;
    held_back(&ui)?;
    release()?;
    let card = ui.snapshot()?.current == Some(beta);
    println!("pinned card click across refresh: {card}");
    rename(&ui, gamma, "Immediate refresh");
    wait("unpressed frame", || {
        text(&ui, "#tab-row").contains("Immediate refresh")
    })?;
    println!("unpressed frame appears immediately: true");
    press(&ui, &format!(".tab[data-id='{alpha}']"))?;
    rename(&ui, gamma, "Focus recovery");
    ui.pages.selected_page().native.grab_focus();
    wait("focus loss releases deferred frame", || {
        text(&ui, "#tab-row").contains("Focus recovery")
    })?;
    release()?;
    println!("focus loss releases deferred frame: true");

    press(&ui, &format!(".tab[data-id='{alpha}']"))?;
    rename(&ui, gamma, "Missed release recovery");
    let (x, y) = page_point(&ui, 500, 500)?;
    move_to(x, y)?;
    release()?;
    hover_chrome(&ui, "#settings-button")?;
    wait("return after outside release", || {
        text(&ui, "#tab-row").contains("Missed release recovery")
    })?;
    println!("outside release does not freeze chrome: true");
    assert!(
        tab && menu && card && held,
        "refresh must preserve every held click and defer painting"
    );
    Ok(())
}
