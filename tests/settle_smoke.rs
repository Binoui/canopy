use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{Browser, Landing, NavigationTarget},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

mod common;
use common::*;

fn tabs(ui: &Ui) -> usize {
    chrome(ui, "document.querySelectorAll('#tab-row .tab').length")
        .unwrap()
        .as_u64()
        .unwrap() as usize
}

fn shown(ui: &Ui, id: canopy::browser::VisitId) -> bool {
    ui.snapshot().unwrap().current == Some(id) && !ui.engine.native.is_loading()
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
            let Ok(size) = stream.read(&mut request) else {
                continue;
            };
            let request = String::from_utf8_lossy(&request[..size]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let body = format!(
                "<!doctype html><title>Settling {path}</title><p>Settled tree fixture {path}</p>"
            );
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    let mut ids = Vec::new();
    for name in ["Alpha", "Beta", "Gamma"] {
        ui.engine.navigate(
            web_url(&format!("http://{address}/{name}"))?,
            NavigationTarget::Root,
        )?;
        wait(name, || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.title == format!("Settling /{name}"))
                && !ui.engine.native.is_loading()
        })?;
        ids.push(ui.snapshot()?.current.unwrap());
    }
    wait("three tabs", || tabs(&ui) == 3)?;
    ui.engine.focus();
    key("ctrl+t")?;
    wait("start surface", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|s| s == "start")
    })?;
    key("ctrl+w")?;
    wait("Ctrl+W returns from start to the page", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|s| s == "reading")
    })?;
    assert_eq!(tabs(&ui), 3, "Ctrl+T then Ctrl+W preserves every tab");
    assert!(shown(&ui, ids[2]));
    println!("PASS Ctrl+T then Ctrl+W returns to the page without settling.");

    key("ctrl+w")?;
    wait("Ctrl+W removes Gamma and shows Beta", || {
        tabs(&ui) == 2 && shown(&ui, ids[1])
    })?;
    println!("PASS Ctrl+W removes the tab and shows the tree used last.");

    let alpha = format!("#tab-row [data-id='{}']", ids[0]);
    hover_chrome(&ui, &alpha)?;
    xdo(&["click", "2"])?;
    wait("middle-click settles Alpha only", || tabs(&ui) == 1)?;
    assert!(
        shown(&ui, ids[1]),
        "Middle-click preserves the current page"
    );
    println!("PASS middle-click settles another tab only.");

    key("ctrl+1")?;
    wait("Ctrl+1 selects the first open tab", || shown(&ui, ids[1]))?;
    assert_eq!(text(&ui, "#tab-row .tab-title"), "Settling /Beta");
    println!("PASS Ctrl+1 selects the first open tab after Alpha is settled.");

    key("ctrl+w")?;
    wait("last tree clears the cursor", || {
        ui.snapshot().unwrap().current.is_none() && tabs(&ui) == 0 && !ui.engine.native.is_loading()
    })?;
    wait("blank home page and start surface", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|s| s == "start")
    })?;
    assert!(
        !ui.engine
            .native
            .uri()
            .is_some_and(|uri| uri.starts_with("http")),
        "The last settle replaces the live page"
    );
    assert_eq!(ui.snapshot()?.visits.len(), 3, "Settling deletes nothing");
    println!("PASS Ctrl+W on the last tree shows the new-tree surface over a blank page.");

    key("ctrl+e")?;
    wait("settled forest", || {
        visible(&ui, ".forest-divider") && visible(&ui, ".forest-node.settled")
    })?;
    assert!(text(&ui, ".forest-divider").contains("3 trees"));
    click_chrome(&ui, &format!(".forest-node[data-id='{}']", ids[0]))?;
    wait("forest click reopens Alpha", || {
        tabs(&ui) == 1 && shown(&ui, ids[0])
    })?;
    println!("PASS clicking a settled page in the forest brings its tab back.");

    key("ctrl+k")?;
    wait("palette input has focus", || {
        chrome_focused(&ui, "#palette-input")
    })?;
    type_text("Gamma")?;
    wait("palette finds settled Gamma", || {
        text(&ui, ".palette-item[data-kind=saved]").contains("Gamma")
    })?;
    click_chrome(&ui, ".palette-item[data-kind=saved]")?;
    wait("saved hit opens the reader", || {
        visible(&ui, "[data-act=live]")
    })?;
    click_chrome(&ui, "[data-act=live]")?;
    wait("palette reopens Gamma", || {
        tabs(&ui) == 2 && shown(&ui, ids[2])
    })?;
    assert_eq!(
        text(&ui, "#tab-row .tab:first-child .tab-title"),
        "Settling /Alpha",
        "Reopened trees retain their creation slots"
    );
    println!("PASS Ctrl+K finds a settled page and opening it reopens its tree.");

    hover_chrome(&ui, &alpha)?;
    click_chrome(&ui, &format!("[data-settle='{}']", ids[0]))?;
    wait("close mark settles Alpha only", || tabs(&ui) == 1)?;
    assert!(
        shown(&ui, ids[2]),
        "The close mark preserves the current page"
    );
    println!("PASS the tab close mark settles another tab only.");
    ui.window.close();

    for all_settled in [false, true] {
        let restart_dir = tempfile::tempdir()?;
        let mut browser = Browser::open(&restart_dir.path().join("forest.sqlite3"))?;
        let mut roots = Vec::new();
        for name in ["Alpha", "Beta"] {
            let url = browser.navigate(
                web_url(&format!("http://{address}/{name}"))?,
                NavigationTarget::Root,
            )?;
            browser.started(url.clone())?;
            browser.committed(url)?;
            roots.push(browser.snapshot()?.current.unwrap());
        }
        assert_eq!(
            browser.settle_tree(browser.tree_states()?[1].root)?,
            Landing::Restore(roots[0])
        );
        if all_settled {
            assert_eq!(
                browser.settle_tree(browser.tree_states()?[0].root)?,
                Landing::Unchanged
            );
        }
        drop(browser);
        let resumed = Ui::new(restart_dir.path())?;
        resumed.window.show_all();
        ready(&resumed)?;
        resumed.resume(&[])?;
        if all_settled {
            wait("restart with only settled trees shows start", || {
                resumed.snapshot().unwrap().current.is_none() && tabs(&resumed) == 0
            })?;
            assert!(!resumed
                .engine
                .native
                .uri()
                .is_some_and(|uri| uri.starts_with("http")));
        } else {
            wait("restart lands on the open tree", || {
                shown(&resumed, roots[0]) && tabs(&resumed) == 1
            })?;
        }
        resumed.window.close();
    }
    println!("PASS restarting in a settled tree uses the same landing policy.");
    Ok(())
}
