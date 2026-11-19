#[path = "common/http.rs"]
mod http;
use std::{io::Write, net::TcpListener, thread};

use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{Browser, Landing, Navigation, NavigationTarget, Visibility},
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
    ui.snapshot().unwrap().current == Some(id) && !ui.pages.selected_page().native.is_loading()
}

fn reopen_scroll(
    ui: &std::rc::Rc<Ui>,
    base: &str,
    hits: &std::sync::atomic::AtomicUsize,
) -> Result<()> {
    ui.pages.go(canopy::pages::Go::Address {
        url: web_url(&format!("{base}/a"))?,
        target: NavigationTarget::Root,
    })?;
    wait("A loaded", || {
        !ui.pages.selected_page().native.is_loading() && ui.snapshot().unwrap().current.is_some()
    })?;
    let a = ui.snapshot()?.current.unwrap();
    ui.pages.go(canopy::pages::Go::Address {
        url: web_url(&format!("{base}/b"))?,
        target: NavigationTarget::ChildOf(a),
    })?;
    wait("B loaded", || {
        !ui.pages.selected_page().native.is_loading() && ui.snapshot().unwrap().current != Some(a)
    })?;
    let b = ui.snapshot()?.current.unwrap();
    eval(
        ui,
        "scrollTo(0,1500); window.marker='old'; JSON.stringify(true)",
    )?;
    wait("B scroll saved", || {
        ui.pages.read(|brow| brow.scroll(b)).unwrap() == Some(1500)
    })?;
    let forest = ui.pages.read(|b| {
        canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)
    })?;
    ui.pages
        .place_tree(forest.tree_of(a).unwrap(), canopy::browser::Move::History)?;
    wait("home", || {
        ui.pages.selected_page().native.uri().as_deref() == Some("about:blank")
            && !ui.pages.selected_page().native.is_loading()
    })?;
    let before = hits.load(std::sync::atomic::Ordering::Relaxed);
    ui.pages.go(canopy::pages::Go::Visit(b))?;
    wait("reopened B", || shown(ui, b))?;
    assert_eq!(
        eval(ui, "JSON.stringify(window.marker ?? null)")?,
        serde_json::Value::Null,
        "reopening must request a fresh page"
    );
    wait("reopened scroll", || {
        eval(ui, "JSON.stringify(scrollY)").unwrap() == serde_json::json!(1500)
    })?;
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::Relaxed),
        before + 1,
        "reopened page must reach the server"
    );
    println!("PASS reopen_scroll");
    Ok(())
}

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let b_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = b_hits.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let Ok(request) = http::request(&mut stream) else {
                continue;
            };
            let request = String::from_utf8_lossy(&request);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            if path == "/b" {
                counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let body = format!(
                "<!doctype html><title>Settling {path}</title><body style='height:10000px'><p>Settled tree fixture {path}</p>"
            );
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    if std::env::var("SETTLE_CASE").as_deref() == Ok("reopen_scroll") {
        reopen_scroll(&ui, &format!("http://{address}"), &b_hits)?;
        ui.window.close();
        return Ok(());
    }
    let mut ids = Vec::new();
    for name in ["Alpha", "Beta", "Gamma"] {
        ui.pages.selected_page().navigate(
            web_url(&format!("http://{address}/{name}"))?,
            NavigationTarget::Root,
        )?;
        wait(name, || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.title == format!("Settling /{name}"))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        ids.push(ui.snapshot()?.current.unwrap());
    }
    wait("three tabs", || tabs(&ui) == 3)?;
    ui.pages.selected_page().focus();
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
    assert_eq!(text(&ui, "#tab-row .tab-name"), "Settling /Beta");
    println!("PASS Ctrl+1 selects the first open tab after Alpha is settled.");

    key("ctrl+w")?;
    wait("last tree clears the cursor", || {
        ui.snapshot().unwrap().current.is_none()
            && tabs(&ui) == 0
            && !ui.pages.selected_page().native.is_loading()
    })?;
    wait("blank home page and start surface", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|s| s == "start")
    })?;
    assert!(
        !ui.pages
            .selected_page()
            .native
            .uri()
            .is_some_and(|uri| uri.starts_with("http")),
        "The last settle replaces the live page"
    );
    assert_eq!(ui.snapshot()?.visits.len(), 3, "Settling deletes nothing");
    println!("PASS Ctrl+W on the last tree shows the new-tree surface over a blank page.");

    key("ctrl+e")?;
    wait("forest", || visible(&ui, "#forest"))?;
    click_chrome(&ui, "#forest-views [data-view=history]")?;
    wait("settled history", || visible(&ui, ".forest-row"))?;
    assert!(text(&ui, ".forest-day").contains("3 trees"));
    click_chrome(&ui, &format!(".forest-row[data-id='{}']", ids[0]))?;
    key("Return")?;
    wait("history reopens its last page", || {
        tabs(&ui) == 1 && shown(&ui, ids[0])
    })?;
    ui.pages.go(canopy::pages::Go::Address {
        url: web_url(&format!("http://{address}/Alpha-child"))?,
        target: NavigationTarget::ChildOf(ids[0]),
    })?;
    wait("Alpha child loaded", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.title == "Settling /Alpha-child")
            && !ui.pages.selected_page().native.is_loading()
    })?;
    key("ctrl+w")?;
    wait("settle again", || tabs(&ui) == 0)?;
    key("ctrl+e")?;
    wait("forest again", || visible(&ui, "#forest"))?;
    click_chrome(&ui, "#forest-views [data-view=history]")?;
    wait("history again", || visible(&ui, ".forest-row"))?;
    click_chrome(&ui, &format!(".forest-row[data-id='{}']", ids[0]))?;
    wait("unfolded History page", || {
        visible(&ui, &format!(".history-page[data-id='{}']", ids[0]))
    })?;
    assert_eq!(
        chrome(&ui, "document.querySelectorAll('.history-page').length")?,
        2
    );
    assert!(text(&ui, ".history-page").contains("Alpha"));
    assert!(
        chrome(
            &ui,
            "document.querySelector('.history-page.last').textContent.includes('Alpha-child')"
        )? == true
    );
    click_chrome(&ui, &format!(".history-page[data-id='{}']", ids[0]))?;
    wait("history reopens Alpha", || {
        tabs(&ui) == 1 && shown(&ui, ids[0])
    })?;
    println!("PASS history reopens a settled tree.");

    key("ctrl+k")?;
    let palette_focus = wait("palette input has focus", || {
        chrome_focused(&ui, "#palette-input")
    });
    if palette_focus.is_err() {
        eprintln!("palette active={:?} visible={} page_focus={} chrome_focus={} page_sensitive={} page_uri={:?}",
            chrome(&ui, "document.activeElement?.id"), visible(&ui, "#palette"), ui.pages.selected_page().native.is_focus(), ui.chrome.native().is_focus(), ui.pages.selected_page().native.is_sensitive(), ui.pages.selected_page().native.uri());
    }
    palette_focus?;
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
        text(&ui, "#tab-row .tab:first-child .tab-name"),
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
        let mut navigation = Navigation::default();
        for name in ["Alpha", "Beta"] {
            let url = browser.navigate(
                &mut navigation,
                web_url(&format!("http://{address}/{name}"))?,
                NavigationTarget::Root,
            )?;
            let ticket = navigation.pending_ticket().unwrap();
            navigation.started(ticket);
            browser.commit_load(&mut navigation, ticket, url, Visibility::Selected)?;
            roots.push(browser.snapshot()?.current.unwrap());
        }
        assert_eq!(
            browser.place_tree(
                browser.tree_states()?[1].root,
                canopy::browser::Move::History
            )?,
            Landing::Restore(roots[0])
        );
        if all_settled {
            assert_eq!(
                browser.place_tree(
                    browser.tree_states()?[0].root,
                    canopy::browser::Move::History
                )?,
                Landing::Start
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
                .pages
                .selected_page()
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
    let directory = tempfile::tempdir()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    reopen_scroll(&ui, &format!("http://{address}"), &b_hits)?;
    ui.window.close();
    Ok(())
}
