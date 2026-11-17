use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{Browser, ImportedPage, NavigationTarget, ThemeMode},
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

fn refresh(ui: &Ui) -> Result<()> {
    let dark = chrome(ui, "canopy.state().dark")? == true;
    ui.dispatch(ChromeIntent::SetTheme(if dark {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    }));
    wait("refresh landed", || {
        chrome(ui, "canopy.state().dark").is_ok_and(|v| v == !dark)
    })
}

fn open(ui: &Ui, query: &str) -> Result<()> {
    key("ctrl+k")?;
    wait("palette focused", || chrome_focused(ui, "#palette-input"))?;
    if !query.is_empty() {
        type_text(query)?;
    }
    wait("three palette rows", || {
        chrome(ui, "canopy.state().rows.length").is_ok_and(|v| v.as_u64().unwrap_or(0) >= 3)
    })?;
    pump(200);
    Ok(())
}

fn choose(ui: &Ui, query: &str, refreshed: bool, mouse: bool) -> Result<(String, String)> {
    open(ui, query)?;
    let rows = chrome(ui, "canopy.state().rows")?;
    let default = chrome(
        ui,
        "document.querySelector('.palette-item[aria-selected=true]').dataset.row",
    )?;
    assert_ne!(default, "2", "third row must not be the default");
    let title = rows[2]["title"].as_str().unwrap().to_owned();
    if !mouse {
        key("Down")?;
        key("Down")?;
    }
    if refreshed {
        refresh(ui)?;
    }
    if mouse {
        click_chrome(ui, ".palette-item[data-row='2']")?;
    } else {
        assert_eq!(
            chrome(
                ui,
                "document.querySelector('.palette-item[aria-selected=true]').dataset.row"
            )?,
            "2"
        );
        key("Return")?;
    }
    Ok((title, rows[2]["icon"]["url"].as_str().unwrap().to_owned()))
}

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("0.0.0.0:0")?;
    let port = listener.local_addr()?.port();
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
    ui.dispatch(ChromeIntent::SetCaptureEnabled(false));
    let mut roots = Vec::new();
    for name in ["alpha", "beta", "gamma"] {
        ui.pages.selected_page().navigate(
            web_url(&format!("http://127.0.0.1:{port}/{name}"))?,
            NavigationTarget::Root,
        )?;
        wait("fixture loaded", || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.title == format!("Reading /{name}"))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        roots.push(ui.snapshot()?.current.unwrap());
    }
    for (refreshed, mouse) in [(false, false), (true, false), (false, true), (true, true)] {
        ui.dispatch(ChromeIntent::Restore(roots[2]));
        wait("gamma restored", || {
            ui.snapshot().unwrap().current == Some(roots[2])
                && !ui.pages.selected_page().native.is_loading()
        })?;
        pump(200);
        let (chosen, _) = choose(&ui, "Reading", refreshed, mouse)?;
        pump(700);
        let actual = ui.snapshot()?.current_visit().unwrap().title.clone();
        println!("palette refresh={refreshed} mouse={mouse}: picked {chosen:?}, opened {actual:?}");
        assert_eq!(actual, chosen, "palette must run the chosen row");
    }
    open(&ui, "Reading")?;
    key("Escape")?;
    let tree = ui
        .pages
        .read(|b| b.tree_states())?
        .into_iter()
        .find(|t| t.root.root() == roots[0])
        .unwrap()
        .root;
    ui.dispatch(ChromeIntent::RenameTree(tree, "Renamed tree".into()));
    refresh(&ui)?;
    open(&ui, "Reading")?;
    assert!(text(&ui, "#palette-results").contains("Renamed tree"));
    key("Escape")?;
    ui.dispatch(ChromeIntent::Bookmark {
        url: web_url(&format!("http://127.0.0.1:{port}/bookmark"))?,
        title: "Reading bookmark".into(),
    });
    refresh(&ui)?;
    open(&ui, "Reading")?;
    assert!(text(&ui, "#palette-results").contains("Reading bookmark"));
    key("Escape")?;
    println!("PASS reopening the same query shows renamed trees and added bookmarks.");

    let mut browser = Browser::open(&directory.path().join("forest.sqlite3"))?;
    let imported: Vec<_> = (2..=4)
        .map(|i| ImportedPage {
            url: web_url(&format!("http://127.0.0.{i}:{port}/suggestion-{i}")).unwrap(),
            title: format!("Suggestion {i}"),
            visits: 100 - i,
            typed: 1,
            last_visit: None,
        })
        .collect();
    browser.import_pages("palette", &imported, &[])?;
    for (refreshed, mouse) in [(false, false), (true, false), (false, true), (true, true)] {
        key("ctrl+t")?;
        wait("new tree focused", || chrome_focused(&ui, "#start-search"))?;
        let frozen = chrome(&ui, "canopy.state().start.suggested.map(s=>s.title)")?;
        let previous = ui.snapshot()?.current;
        let (chosen, url) = choose(&ui, "", refreshed, mouse)?;
        assert_eq!(frozen[2], chosen);
        wait("suggested page opened", || {
            let snapshot = ui.snapshot().unwrap();
            snapshot.current != previous
                && snapshot
                    .current_visit()
                    .is_some_and(|v| v.url.as_str() == url)
                && !ui.pages.selected_page().native.is_loading()
        })?;
        assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
        println!("PASS frozen empty-query suggestion refresh={refreshed} mouse={mouse} opens {chosen:?}.");
    }
    open(&ui, "Reading")?;
    let bookmark = browser
        .bookmarks()?
        .into_iter()
        .find(|b| b.title == "Reading bookmark")
        .unwrap();
    let row = chrome(
        &ui,
        "canopy.state().rows.findIndex(r=>r.title==='Reading bookmark')",
    )?;
    ui.dispatch(ChromeIntent::RemoveBookmark(bookmark.id));
    refresh(&ui)?;
    click_chrome(&ui, &format!(".palette-item[data-row='{row}']"))?;
    wait("removed bookmark URL opened", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url == bookmark.url)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(!browser.bookmarks()?.iter().any(|b| b.id == bookmark.id));
    println!("PASS a remembered removed bookmark opens its URL without recreating the bookmark.");
    Ok(())
}
