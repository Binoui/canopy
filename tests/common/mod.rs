#![allow(dead_code)]

use std::{
    cell::RefCell,
    process::Command,
    rc::Rc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use canopy::ui::Ui;
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use webkit2gtk::WebViewExt;

pub fn start(_directory: &std::path::Path) -> Result<()> {
    canopy::init_desktop().context("This test needs a display. Run it under xvfb-run -a.")?;
    Ok(())
}

/// Waits until the chrome view has loaded and drawn its first frame.
pub fn ready(ui: &Ui) -> Result<()> {
    wait("chrome ready", || ui.chrome.is_ready())?;
    pump(200);
    Ok(())
}

pub fn wait(label: &str, condition: impl Fn() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        if condition() {
            return Ok(());
        }
        if Instant::now() > deadline {
            bail!("Timed out waiting for {label}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub fn pump(ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Runs xdotool while GTK keeps handling events, so timers and crossings stay realistic.
pub fn xdo(args: &[&str]) -> Result<()> {
    let mut child = Command::new("xdotool").args(args).spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("xdotool {args:?} failed");
            }
            return Ok(());
        }
        pump(1);
    }
}

pub fn window_id() -> Result<String> {
    let output = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "Canopy"])
        .output()?;
    let ids = String::from_utf8(output.stdout)?;
    Ok(ids.lines().last().context("X11 window")?.to_owned())
}

pub fn focus_window() -> Result<()> {
    xdo(&["windowfocus", &window_id()?])
}

pub fn key(combo: &str) -> Result<()> {
    focus_window()?;
    xdo(&["key", "--clearmodifiers", combo])?;
    pump(60);
    Ok(())
}

pub fn type_text(text: &str) -> Result<()> {
    xdo(&["type", "--delay", "4", text])?;
    pump(60);
    Ok(())
}

/// Root-window coordinates of a point inside `widget`.
pub fn screen_point(ui: &Ui, widget: &impl IsA<gtk::Widget>, x: i32, y: i32) -> Result<(i32, i32)> {
    let toplevel = widget
        .as_ref()
        .toplevel()
        .context("widget is in a window")?;
    let (x, y) = widget
        .as_ref()
        .translate_coordinates(&toplevel, x, y)
        .context("widget position")?;
    let (_, ox, oy) = toplevel
        .window()
        .or_else(|| ui.window.window())
        .context("realized window")?
        .origin();
    Ok((x + ox, y + oy))
}

pub fn move_to(x: i32, y: i32) -> Result<()> {
    xdo(&["mousemove", "--sync", &x.to_string(), &y.to_string()])
}

pub fn click_at(x: i32, y: i32) -> Result<()> {
    move_to(x, y)?;
    xdo(&["click", "1"])?;
    pump(80);
    Ok(())
}

/// A point in the page, in window coordinates, converted to the screen.
pub fn page_point(ui: &Ui, x: i32, y: i32) -> Result<(i32, i32)> {
    let (_, ox, oy) = ui.window.window().context("realized window")?.origin();
    Ok((x + ox, y + oy))
}

/// Evaluates `expression` in the chrome view and returns its value as JSON.
pub fn chrome(ui: &Ui, expression: &str) -> Result<Value> {
    let output = Rc::new(RefCell::new(None));
    let result = output.clone();
    let script = format!("JSON.stringify((() => ({expression}))() ?? null)");
    ui.chrome.native().evaluate_javascript(
        &script,
        None,
        None,
        None::<&gio::Cancellable>,
        move |value| {
            *result.borrow_mut() = Some(value.map(|value| value.to_str().to_string()));
        },
    );
    wait("chrome JavaScript", || output.borrow().is_some())?;
    let text = output.borrow_mut().take().unwrap()?;
    Ok(serde_json::from_str(&text)?)
}

fn quoted(selector: &str) -> String {
    serde_json::to_string(selector).unwrap()
}

/// Whether an element of the chrome is rendered.
pub fn visible(ui: &Ui, selector: &str) -> bool {
    chrome(
        ui,
        &format!(
            "(() => {{ const e = document.querySelector({}); return !!e && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden'; }})()",
            quoted(selector)
        ),
    )
    .is_ok_and(|value| value == true)
}

pub fn text(ui: &Ui, selector: &str) -> String {
    chrome(
        ui,
        &format!(
            "document.querySelector({})?.innerText ?? ''",
            quoted(selector)
        ),
    )
    .ok()
    .and_then(|value| value.as_str().map(str::to_owned))
    .unwrap_or_default()
}

/// The chrome element's box, in window coordinates.
pub fn rect(ui: &Ui, selector: &str) -> Result<(f64, f64, f64, f64)> {
    let r = chrome(
        ui,
        &format!(
            "(() => {{ const r = document.querySelector({}).getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; }})()",
            quoted(selector)
        ),
    )?;
    let n = |i: usize| r[i].as_f64().unwrap_or_default();
    Ok((n(0), n(1), n(2), n(3)))
}

/// Waits until the strip has finished moving, so element positions hold still.
pub fn settle(ui: &Ui) -> Result<()> {
    wait("chrome animations", || {
        chrome(ui, "canopy.state().animating").is_ok_and(|value| value == false)
    })
}

/// Screen point at the center of a chrome element, once the strip holds still.
pub fn chrome_point(ui: &Ui, selector: &str) -> Result<(i32, i32)> {
    settle(ui)?;
    let (x, y, w, h) = rect(ui, selector)?;
    screen_point(
        ui,
        &ui.chrome.native(),
        (x + w / 2.) as i32,
        (y + h / 2.) as i32,
    )
}

pub fn click_chrome(ui: &Ui, selector: &str) -> Result<()> {
    let (x, y) = chrome_point(ui, selector)?;
    click_at(x, y)
}

pub fn hover_chrome(ui: &Ui, selector: &str) -> Result<()> {
    let (x, y) = chrome_point(ui, selector)?;
    move_to(x, y)
}

/// Whether the chrome view has the keyboard and `selector` is its focused element.
pub fn chrome_focused(ui: &Ui, selector: &str) -> bool {
    ui.chrome.native().has_focus()
        && chrome(
            ui,
            &format!(
                "document.hasFocus() && !!document.activeElement?.matches({})",
                quoted(selector)
            ),
        )
        .is_ok_and(|value| value == true)
}

pub fn node(id: impl std::fmt::Display) -> String {
    format!("#strip-nodes [data-id='{id}']")
}

pub fn strip_height(ui: &Ui) -> i64 {
    chrome(ui, "document.getElementById('strip').offsetHeight")
        .ok()
        .and_then(|value| value.as_i64())
        .unwrap_or_default()
}

/// How far down the chrome view reaches, as it last told Rust; `None` covers the window.
pub fn cover(ui: &Ui) -> Option<i64> {
    let state = chrome(ui, "canopy.state().reported").ok()?;
    state.as_str()?.split('/').next()?.parse().ok()
}

/// Hovers the strip and waits until it has opened and its nodes settled.
pub fn open_strip(ui: &Ui) -> Result<()> {
    let (x, y) = page_point(ui, ui.window.allocated_width() - 60, 42 + 26)?;
    move_to(x, y)?;
    wait("strip open", || strip_height(ui) == 120)?;
    pump(350);
    Ok(())
}

pub fn leave_strip(ui: &Ui) -> Result<()> {
    let (x, y) = page_point(ui, ui.window.allocated_width() - 60, 600)?;
    move_to(x, y)?;
    wait("strip closed", || strip_height(ui) == 52)?;
    pump(300);
    Ok(())
}

pub fn toast_text(ui: &Ui) -> Option<String> {
    visible(ui, "#toast").then(|| text(ui, "#toast"))
}

/// Opens the Memory menu from the dot on the current-page pill.
pub fn open_memory_menu(ui: &Ui) -> Result<()> {
    if !visible(ui, "#popover #automatic-capture") {
        click_chrome(ui, "#memory-dot")?;
        wait("Memory menu", || visible(ui, "#popover #automatic-capture"))?;
        pump(100);
    }
    Ok(())
}

/// Opens Settings from the gear at the end of the tab row.
pub fn open_settings(ui: &Ui) -> Result<()> {
    if !visible(ui, "#popover #theme-mode") {
        click_chrome(ui, "#settings-button")?;
        wait("Settings", || visible(ui, "#popover #theme-mode"))?;
        pump(100);
    }
    Ok(())
}

/// Whether the page's favicon has loaded on `selector`, in the given color channel order.
pub fn icon_is_blue(ui: &Ui, selector: &str) -> bool {
    chrome(
        ui,
        &format!(
            "(() => {{ const img = document.querySelector({}); if (!img || !img.complete || !img.naturalWidth) return false; const c = document.createElement('canvas'); c.width = c.height = 16; const x = c.getContext('2d'); x.drawImage(img, 0, 0, 16, 16); const d = x.getImageData(8, 8, 1, 1).data; return d[2] > 200 && d[0] < 80; }})()",
            quoted(&format!("{selector} img"))
        ),
    )
    .is_ok_and(|value| value == true)
}

pub fn eval(ui: &Ui, script: &str) -> Result<Value> {
    let output = Rc::new(RefCell::new(None));
    let result = output.clone();
    ui.engine.native.evaluate_javascript(
        script,
        None,
        None,
        None::<&gio::Cancellable>,
        move |value| {
            *result.borrow_mut() = Some(value.map(|value| value.to_str().to_string()));
        },
    );
    wait("JavaScript", || output.borrow().is_some())?;
    let text = output.borrow_mut().take().unwrap()?;
    Ok(serde_json::from_str(&text)?)
}

/// Screen point at the center of a page element.
pub fn element_point(ui: &Ui, id: &str) -> Result<(i32, i32)> {
    let script = format!("(() => {{const r=document.getElementById({}).getBoundingClientRect();return JSON.stringify({{x:r.x+r.width/2,y:r.y+r.height/2}})}})()", serde_json::to_string(id)?);
    let point = eval(ui, &script)?;
    screen_point(
        ui,
        &ui.engine.native,
        point["x"].as_f64().unwrap() as i32,
        point["y"].as_f64().unwrap() as i32,
    )
}

pub fn save_window(ui: &Ui, path: &str) -> Result<()> {
    pump(150);
    if let Some(window) = ui.window.window() {
        let a = ui.window.allocation();
        window
            .pixbuf(0, 0, a.width(), a.height())
            .context("window capture")?
            .savev(path, "png", &[])?;
    }
    Ok(())
}
