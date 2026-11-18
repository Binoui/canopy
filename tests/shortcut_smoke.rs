#[path = "common/http.rs"]
mod http;
use std::{io::Write, net::TcpListener, thread};

use anyhow::Result;
use canopy::{address::web_url, browser::NavigationTarget, ui::Ui};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

mod common;
use common::*;

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = web_url(&format!("http://{}/", listener.local_addr()?))?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        let mut loads = 0;
        while loads < 8 {
            let (mut stream, _) = listener.accept()?;
            let request = http::request(&mut stream)?;
            if request.starts_with(b"GET / HTTP/") || request.starts_with(b"GET /depth") {
                loads += 1;
            }
            let body = format!(
                "<!doctype html><title>Shortcut fixture {loads}</title><p>Keyboard shortcuts</p>"
            );
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
        }
        Ok(())
    });
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages
        .selected_page()
        .navigate(url, NavigationTarget::Root)?;
    wait("fixture page", || {
        ui.snapshot().unwrap().current.is_some() && !ui.pages.selected_page().native.is_loading()
    })?;
    wait("reading surface", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|surface| surface == "reading")
    })?;
    ui.pages.selected_page().focus();
    key("ctrl+shift+t")?;
    pump(250);
    assert_eq!(
        chrome(&ui, "canopy.state().surface")?,
        "reading",
        "Ctrl+Shift+T must not open the new-tree page"
    );
    println!("PASS Ctrl+Shift+T leaves the reading surface open.");
    key("ctrl+t")?;
    wait("Ctrl+T opens the new-tree page", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|surface| surface == "start")
    })?;
    println!("PASS Ctrl+T opens the new-tree page.");
    key("Escape")?;
    wait("return to reading", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|surface| surface == "reading")
    })?;
    let before = ui.snapshot()?;
    for (combo, title) in [
        ("F5", "Shortcut fixture 2"),
        ("ctrl+shift+r", "Shortcut fixture 3"),
    ] {
        ui.pages.selected_page().focus();
        key(combo)?;
        wait(&format!("{combo} reloads the page"), || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|visit| visit.title == title)
                && !ui.pages.selected_page().native.is_loading()
        })?;
        assert_eq!(ui.snapshot()?.current, before.current);
        assert_eq!(ui.snapshot()?.visits.len(), before.visits.len());
        println!("PASS {combo} reloads the current page without adding a visit.");
    }
    for depth in 0..5 {
        let snapshot = ui.snapshot()?;
        let parent = snapshot.current.unwrap();
        let mut url = snapshot.current_visit().unwrap().url.clone();
        url.set_path(&format!("/depth{depth}"));
        ui.pages
            .selected_page()
            .navigate(url, NavigationTarget::ChildOf(parent))?;
        wait("forest depth fixture", || {
            ui.snapshot().unwrap().current != Some(parent)
                && !ui.pages.selected_page().native.is_loading()
        })?;
    }
    server.join().unwrap()?;
    forest_popin(&ui)?;
    ui.window.close();
    Ok(())
}

fn forest_open(ui: &Ui) -> Result<()> {
    key("ctrl+e")?;
    wait("forest overlay", || visible(ui, "#forest"))?;
    assert_eq!(chrome(ui, "canopy.state().overlay")?, "forest");
    assert_eq!(
        chrome(
            ui,
            "document.querySelector('#forest-button').getAttribute('aria-pressed')"
        )?,
        "true"
    );
    Ok(())
}

fn forest_closed(ui: &Ui, surface: &str) -> Result<()> {
    wait("forest dismissed", || !visible(ui, "#forest"))?;
    assert!(chrome(ui, "canopy.state().overlay")?.is_null());
    assert_eq!(chrome(ui, "canopy.state().surface")?, surface);
    assert_eq!(
        chrome(
            ui,
            "document.querySelector('#forest-button').getAttribute('aria-pressed')"
        )?,
        "false"
    );
    Ok(())
}

fn forest_popin(ui: &Ui) -> Result<()> {
    assert_eq!(text(ui, "#forest-button"), "Forest");
    assert_eq!(
        chrome(ui, "document.querySelectorAll('#forest-button').length")?,
        1
    );
    assert!(rect(ui, "#forest-button")?.0 < rect(ui, ".tabs-divider")?.0);
    assert!(rect(ui, ".tabs-divider")?.0 < rect(ui, "#tab-row")?.0);
    assert!(visible(ui, ".tabs-divider"));
    for dismiss in ["shortcut", "button", "escape", "close", "scrim"] {
        forest_open(ui)?;
        assert_eq!(cover(ui), None);
        assert_eq!(rect(ui, "#forest > .scrim")?.1, 42.);
        let (left, _, width, _) = rect(ui, ".forest-pop")?;
        let window = chrome(ui, "innerWidth")?.as_f64().unwrap();
        assert!(width < window && (left - (window - left - width)).abs() <= 2.);
        match dismiss {
            "shortcut" => key("ctrl+e")?,
            "button" => click_chrome(ui, "#forest-button")?,
            "escape" => key("Escape")?,
            "close" => click_chrome(ui, "#forest-close")?,
            _ => {
                let (x, y) = screen_point(ui, &ui.chrome.native(), 10, 400)?;
                move_to(x, y)?;
                xdo(&["mousedown", "1"])?;
                pump(80);
                assert!(
                    !visible(ui, "#forest"),
                    "A press closes the scrim before release"
                );
                xdo(&["mouseup", "1"])?;
            }
        }
        forest_closed(ui, "reading")?;
        if dismiss != "scrim" {
            wait("page focus after dismissal", || {
                ui.pages.selected_page().native.has_focus()
            })?;
        }
        println!("PASS forest closes through {dismiss} and returns to the page.");
    }
    forest_open(ui)?;
    assert!(visible(ui, "#forest-count"));
    for width in [1240, 1920] {
        ui.window.resize(width, 800);
        pump(300);
        assert_eq!(chrome(ui, "(() => { const r=document.querySelector('.forest-pop').getBoundingClientRect(); return [r.left, innerWidth-r.right]; })()")?, serde_json::json!([22, 22]));
    }
    ui.window.resize(1240, 800);
    pump(300);
    let id = ui.snapshot()?.current.unwrap();
    click_chrome(ui, ".tree-card[aria-current='true']")?;
    forest_closed(ui, "reading")?;
    assert_eq!(ui.snapshot()?.current, Some(id));
    wait("card returns page focus", || {
        ui.pages.selected_page().native.has_focus()
    })?;
    println!("PASS forest uses the window width and current card returns to the page.");
    key("ctrl+t")?;
    forest_open(ui)?;
    key("Escape")?;
    forest_closed(ui, "start")?;
    assert!(visible(ui, "#start"));
    key("Escape")?;
    forest_closed(ui, "reading")?;
    let shots = std::env::var("CANOPY_FOREST_SHOTS").ok();
    for theme in ["dark", "light"] {
        open_settings(ui)?;
        click_chrome(ui, &format!("#theme-mode [data-mode={theme}]"))?;
        key("Escape")?;
        if let Some(dir) = &shots {
            save_window(ui, &format!("{dir}/forest-button-{theme}.png"))?;
        }
        forest_open(ui)?;
        pump(300);
        if let Some(dir) = &shots {
            save_window(ui, &format!("{dir}/forest-popin-{theme}.png"))?;
        }
        click_chrome(ui, "#forest-views [data-view=history]")?;
        pump(200);
        if let Some(dir) = &shots {
            save_window(ui, &format!("{dir}/forest-history-{theme}.png"))?;
        }
        key("Escape")?;
        key("ctrl+t")?;
        forest_open(ui)?;
        pump(300);
        if let Some(dir) = &shots {
            save_window(ui, &format!("{dir}/forest-start-{theme}.png"))?;
        }
        key("Escape")?;
        forest_closed(ui, "start")?;
        key("Escape")?;
    }
    key("ctrl+w")?;
    wait("last page settled", || {
        ui.snapshot().unwrap().current.is_none()
    })?;
    forest_open(ui)?;
    click_chrome(ui, "#forest-views [data-view=history]")?;
    wait("history rows", || visible(ui, ".forest-row"))?;
    key("Escape")?;
    forest_closed(ui, "start")?;
    if let Some(dir) = &shots {
        for theme in ["dark", "light"] {
            open_settings(ui)?;
            click_chrome(ui, &format!("#theme-mode [data-mode={theme}]"))?;
            key("Escape")?;
            forest_open(ui)?;
            pump(300);
            save_window(ui, &format!("{dir}/forest-settled-{theme}.png"))?;
            key("Escape")?;
            forest_closed(ui, "start")?;
        }
    }
    println!("PASS forest preserves the start surface and Escape closes it without a current page; History remains available.");
    Ok(())
}
