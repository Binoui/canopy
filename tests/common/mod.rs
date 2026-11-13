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

pub fn start(directory: &std::path::Path) -> Result<()> {
    canopy::ui::register_font(directory)?;
    gtk::init().context("This test needs a display. Run it under xvfb-run -a.")?;
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

pub fn center(ui: &Ui, widget: &impl IsA<gtk::Widget>) -> Result<(i32, i32)> {
    let a = widget.as_ref().allocation();
    screen_point(ui, widget, a.width() / 2, a.height() / 2)
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

pub fn click(ui: &Ui, widget: &impl IsA<gtk::Widget>) -> Result<()> {
    let (x, y) = center(ui, widget)?;
    click_at(x, y)
}

/// A point in the page, in window coordinates, converted to the screen.
pub fn page_point(ui: &Ui, x: i32, y: i32) -> Result<(i32, i32)> {
    let (_, ox, oy) = ui.window.window().context("realized window")?.origin();
    Ok((x + ox, y + oy))
}

pub fn find_named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.widget_name() == name {
        return Some(widget.clone());
    }
    widget
        .downcast_ref::<gtk::Container>()?
        .children()
        .iter()
        .find_map(|child| find_named(child, name))
}

pub fn named(widget: &impl IsA<gtk::Widget>, name: &str) -> gtk::Widget {
    find_named(widget.as_ref(), name).unwrap_or_else(|| panic!("missing widget {name}"))
}

pub fn find_all(widget: &gtk::Widget, name: &str, out: &mut Vec<gtk::Widget>) {
    if widget.widget_name() == name {
        out.push(widget.clone());
    }
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            find_all(&child, name, out);
        }
    }
}

pub fn label_texts(widget: &gtk::Widget) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        out.push(label.text().to_string());
    }
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            out.extend(label_texts(&child));
        }
    }
    out
}

pub fn popover_of(ui: &Ui, button: &str) -> gtk::Popover {
    named(&ui.window, button)
        .downcast::<gtk::MenuButton>()
        .unwrap()
        .popover()
        .expect("menu button has a popover")
}

pub fn strip_height(ui: &Ui) -> i32 {
    named(&ui.window, "strip").allocated_height()
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
    pump(50);
    Ok(())
}

pub fn node(ui: &Ui, id: impl std::fmt::Display) -> gtk::Widget {
    named(&ui.window, &format!("node-{id}"))
}

pub fn toast_text(ui: &Ui) -> Option<String> {
    let revealer = named(&ui.window, "toast-revealer")
        .downcast::<gtk::Revealer>()
        .unwrap();
    revealer.reveals_child().then(|| {
        named(&ui.window, "toast-label")
            .downcast::<gtk::Label>()
            .unwrap()
            .text()
            .to_string()
    })
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
