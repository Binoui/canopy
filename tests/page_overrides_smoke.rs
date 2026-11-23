mod common;
#[path = "common/http.rs"]
mod http;

use std::{cell::Cell, io::Write, net::TcpListener, rc::Rc, thread};

use anyhow::Result;
use canopy::{address::web_url, browser::NavigationTarget, ui::Ui};
use gtk::prelude::*;
use serde_json::json;
use webkit2gtk::WebViewExt;

use common::*;

const PAGE: &str = include_str!("fixtures/page-overrides.html");
const PASTE: &str = "ctrl+shift+v";

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = format!("http://{}", listener.local_addr()?);
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            if http::request(&mut stream).is_ok() {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
            }
        }
    });

    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages
        .selected_page()
        .navigate(web_url(&origin)?, NavigationTarget::Root)?;
    let view = ui.pages.selected_page().native;
    wait("override fixture and frame", || {
        view.title().as_deref() == Some("Page overrides") && !view.is_loading()
    })?;

    let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
    clipboard.set_text("CLIP");
    paste(&ui, &clipboard)?;
    chrome_paste(&ui)?;

    let menus = Rc::new(Cell::new(0));
    view.connect_context_menu({
        let menus = menus.clone();
        move |_, _, _, _| {
            menus.set(menus.get() + 1);
            false
        }
    });
    let top = element_point(&ui, "menu")?;
    let frame_point = eval(
        &ui,
        "const f = document.querySelector('iframe'); const r = f.getBoundingClientRect();
         const m = f.contentDocument.getElementById('menu').getBoundingClientRect();
         JSON.stringify([r.left + f.clientLeft + m.left + 30, r.top + f.clientTop + m.top + m.height / 2])",
    )?;
    let frame = screen_point(
        &ui,
        &view,
        frame_point[0].as_f64().unwrap() as i32,
        frame_point[1].as_f64().unwrap() as i32,
    )?;
    let targets = [(top, "menus"), (frame, "frames[0].eval('menus')")];
    for (point, page_count) in targets {
        blocked_menu(&ui, point, page_count, &menus)?;
    }
    for (point, page_count) in targets {
        forced_menu(&ui, point, page_count, &menus)?;
    }
    println!("PASS plain right-click reaches the page blocker and Shift+right-click raises the native menu in the top page and iframe.");

    let (x, y) = element_point(&ui, "waiting")?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    let background = || {
        ui.snapshot()
            .ok()
            .and_then(|s| {
                s.visits
                    .iter()
                    .find(|v| v.url.path() == "/waiting")
                    .map(|v| v.id)
            })
            .and_then(|visit| ui.pages.background_view(visit))
            .filter(|view| view.title().as_deref() == Some("Page overrides") && !view.is_loading())
    };
    wait("waiting page finished loading off-screen", || {
        background().is_some()
    })?;
    let queued = background().unwrap();
    key("ctrl+Tab")?;
    wait("background view is shown", || {
        ui.pages.selected_page().native == queued
    })?;
    let menus = Rc::new(Cell::new(0));
    queued.connect_context_menu({
        let menus = menus.clone();
        move |_, _, _, _| {
            menus.set(menus.get() + 1);
            false
        }
    });
    let point = element_point(&ui, "menu")?;
    blocked_menu(&ui, point, "menus", &menus)?;
    forced_menu(&ui, point, "menus", &menus)?;
    println!("PASS a page loaded in the background has the override the first time it is shown.");
    fullscreen_paste(&ui)?;
    Ok(())
}

fn paste(ui: &Ui, clipboard: &gtk::Clipboard) -> Result<()> {
    let (x, y) = element_point(ui, "field")?;
    click_at(x, y)?;
    key("Home")?;
    key("Right")?;
    key("ctrl+v")?;
    wait("normal Ctrl+V reaches the keydown blocker", || {
        eval(ui, "JSON.stringify(events)").is_ok_and(|v| v == json!([["keydown", false]]))
    })?;
    eval(ui, "blockPasteKey = false; JSON.stringify(true)")?;
    key("ctrl+v")?;
    wait("normal Ctrl+V reaches the paste blocker", || {
        eval(ui, "JSON.stringify(events.at(-1))").is_ok_and(|v| v == json!(["paste"]))
    })?;
    assert_eq!(eval(ui, "JSON.stringify(field.value)")?, "ab");
    eval(
        ui,
        "events.length = 0; blockPasteKey = true; JSON.stringify(true)",
    )?;
    key(PASTE)?;
    wait("forced paste inserts at the caret", || {
        eval(ui, "JSON.stringify(field.value)").is_ok_and(|v| v == "aCLIPb")
    })?;
    assert_eq!(
        eval(ui, "JSON.stringify(events)")?,
        json!([["beforeinput", "ab"], ["input", "aCLIPb"]])
    );
    println!("PASS Ctrl+V is blocked by page handlers. Ctrl+Shift+V inserts at the caret and reaches beforeinput and input only.");

    clipboard.set_text("one\ntwo");
    key("ctrl+a")?;
    key(PASTE)?;
    wait("multiline clipboard edited the single-line input", || {
        eval(ui, "JSON.stringify(field.value)").is_ok_and(|v| v != "aCLIPb")
    })?;
    println!(
        "FINDING multiline clipboard in a single-line input: {}",
        eval(ui, "JSON.stringify(field.value)")?
    );
    let (x, y) = element_point(ui, "lines")?;
    click_at(x, y)?;
    key(PASTE)?;
    wait("multiline clipboard edited the textarea", || {
        eval(ui, "JSON.stringify(lines.value)").is_ok_and(|v| v != "")
    })?;
    println!(
        "FINDING multiline clipboard in a textarea: {}",
        eval(ui, "JSON.stringify(lines.value)")?
    );

    let (x, y) = element_point(ui, "editable")?;
    click_at(x, y)?;
    clipboard.set_text("<b>CLIP</b>");
    key(PASTE)?;
    wait("contenteditable receives literal text", || {
        eval(ui, "JSON.stringify(editable.textContent)").is_ok_and(|v| v == "<b>CLIP</b>")
    })?;
    assert_eq!(eval(ui, "JSON.stringify(editable.children.length)")?, 0);

    clipboard.clear();
    assert!(!clipboard.wait_is_text_available());
    let before = eval(ui, "JSON.stringify(events)")?;
    key(PASTE)?;
    pump(200);
    assert_eq!(eval(ui, "JSON.stringify(events)")?, before);
    clipboard.set_text("CLIP");
    let (x, y) = element_point(ui, "menu")?;
    click_at(x, y)?;
    assert_eq!(
        eval(ui, "JSON.stringify(document.activeElement.tagName)")?,
        "BODY"
    );
    key(PASTE)?;
    pump(200);
    assert_eq!(eval(ui, "JSON.stringify(events)")?, before);
    assert!(toast_text(ui).is_none());
    println!("PASS contenteditable receives literal text. Pasting with no text or no focused editor leaves the page untouched.");
    Ok(())
}

fn chrome_paste(ui: &Ui) -> Result<()> {
    let before = eval(ui, "JSON.stringify(events)")?;
    for (shortcut, input) in [
        ("ctrl+k", "#palette-input"),
        ("ctrl+f", "#find-input"),
        ("ctrl+shift+e", "#tree-filter"),
    ] {
        key(shortcut)?;
        if input == "#tree-filter" {
            wait("workbench filter", || visible(ui, input))?;
            click_chrome(ui, input)?;
        }
        wait(input, || chrome_focused(ui, input))?;
        key(PASTE)?;
        wait(
            "the chrome's native paste enters the clipboard text",
            || chrome(ui, "document.activeElement.value").is_ok_and(|v| v == "CLIP"),
        )?;
        key("Escape")?;
        if input == "#tree-filter" {
            key("Escape")?;
        }
    }
    assert_eq!(eval(ui, "JSON.stringify(events)")?, before);
    println!("PASS the palette, find bar and workbench filter keep their native plain-text paste.");
    Ok(())
}

fn fullscreen_paste(ui: &Ui) -> Result<()> {
    let (x, y) = element_point(ui, "fullscreen")?;
    click_at(x, y)?;
    wait("page entered fullscreen", || ui.pages.is_fullscreen())?;
    let (x, y) = element_point(ui, "field")?;
    click_at(x, y)?;
    let before = eval(ui, "JSON.stringify(field.value)")?;
    key(PASTE)?;
    wait("the fullscreen page gets Ctrl+Shift+V", || {
        eval(ui, "JSON.stringify(events.at(-1))").is_ok_and(|v| v == json!(["keydown", true]))
    })?;
    assert_eq!(eval(ui, "JSON.stringify(field.value)")?, before);
    eval(ui, "document.exitFullscreen(); JSON.stringify(true)")?;
    wait("page exited fullscreen", || !ui.pages.is_fullscreen())?;
    println!("PASS WebKit gets Ctrl+Shift+V during page fullscreen.");
    Ok(())
}

fn blocked_menu(ui: &Ui, point: (i32, i32), page_count: &str, menus: &Cell<u32>) -> Result<()> {
    let count = menus.get();
    let page_count = format!("JSON.stringify({page_count})");
    let before = eval(ui, &page_count)?.as_u64().unwrap();
    click_at(point.0, point.1)?;
    xdo(&["click", "3"])?;
    wait("the page sees the plain contextmenu", || {
        eval(ui, &page_count).is_ok_and(|v| v == before + 1)
    })?;
    assert_eq!(menus.get(), count);
    Ok(())
}

fn forced_menu(ui: &Ui, point: (i32, i32), page_count: &str, menus: &Cell<u32>) -> Result<()> {
    let count = menus.get();
    let page_count = format!("JSON.stringify({page_count})");
    let before = eval(ui, &page_count)?;
    move_to(point.0 + 10, point.1)?;
    xdo(&["keydown", "Shift_L", "click", "3", "keyup", "Shift_L"])?;
    wait("WebKit emits context-menu for Shift+right-click", || {
        menus.get() == count + 1
    })?;
    assert_eq!(eval(ui, &page_count)?, before);
    xdo(&["key", "Escape"])?;
    pump(250);
    Ok(())
}
