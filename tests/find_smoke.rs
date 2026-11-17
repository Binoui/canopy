#[path = "common/http.rs"]
mod http;
use std::{cell::RefCell, io::Write, net::TcpListener, rc::Rc, thread, time::Duration};

use anyhow::{Context, Result};
use canopy::{address::web_url, browser::NavigationTarget, ui::Ui};
use gtk::prelude::*;
use url::Url;
use webkit2gtk::{FindControllerExt, FindOptions, WebViewExt};

mod common;
use common::*;

/// Twelve spans hold "canopy" in mixed case, five hold "forest", and nothing holds "zebra".
fn fixture(title: &str, canopies: usize) -> String {
    let mut body = format!("<!doctype html><title>{title}</title><body style='font:18px sans-serif;margin:40px'><h1>Plain heading</h1>");
    for i in 1..=canopies {
        let word = if i % 2 == 0 { "Canopy" } else { "canopy" };
        body.push_str(&format!(
            "<p id='p{i}' style='margin:0 0 60px'>Line {i}: <span id='m{i}'>{word}</span> grows here.</p>"
        ));
    }
    for i in 1..=5 {
        body.push_str(&format!("<p>Note <span id='f{i}'>forest</span>.</p>"));
    }
    body
}

fn serve(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        thread::spawn(move || -> std::io::Result<()> {
            let mut stream = stream;
            let request = http::request(&mut stream)?;
            let line = String::from_utf8_lossy(&request);
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
            if path.starts_with("/slow") {
                thread::sleep(Duration::from_millis(1500));
            }
            let body = match path.as_str() {
                "/find" => fixture("Find fixture", 12),
                "/three" => fixture("Three fixture", 3),
                _ => fixture("Other fixture", 2),
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        });
    }
}

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("[::]:0")?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || serve(listener));
    let at = |host: &str, path: &str| web_url(&format!("http://{host}:{port}{path}"));
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages
        .selected_page()
        .navigate(at("127.0.0.1", "/find")?, NavigationTarget::Root)?;
    loaded(&ui, "Find fixture")?;
    probe_replies(&ui, &at)?;
    ui.window.close();
    Ok(())
}

fn loaded(ui: &Ui, title: &str) -> Result<()> {
    wait(title, || {
        ui.snapshot()
            .ok()
            .and_then(|s| s.current_visit().map(|v| v.title.clone()))
            .is_some_and(|t| t == title)
            && !ui.pages.selected_page().native.is_loading()
    })
}

/// The match the page itself has selected, by the id of the element around it.
fn selection(ui: &Ui) -> Result<String> {
    let value = eval(
        ui,
        "JSON.stringify(getSelection().anchorNode?.parentElement?.id ?? null)",
    )?;
    Ok(value.as_str().unwrap_or("none").to_owned())
}

/// What WebKit's find controller answers on a live Canopy view, request by request. The
/// driver keeps one request in flight per view, so every request must end in exactly one
/// answer, in order: also after search_finish, a load, a process swap or a crash.
fn probe_replies(ui: &Ui, at: &dyn Fn(&str, &str) -> Result<Url>) -> Result<()> {
    let native = ui.pages.selected_page().native.clone();
    let controller = native.find_controller().context("find controller")?;
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let found = {
        let log = log.clone();
        controller
            .connect_found_text(move |_, count| log.borrow_mut().push(format!("found {count}")))
    };
    let failed = {
        let log = log.clone();
        controller.connect_failed_to_find_text(move |_| log.borrow_mut().push("failed".into()))
    };
    let terminated = {
        let log = log.clone();
        native
            .connect_web_process_terminated(move |_, _| log.borrow_mut().push("terminated".into()))
    };
    let options = (FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND).bits();
    let settle = |expected: usize, label: &str| -> Result<Vec<String>> {
        wait(label, || log.borrow().len() >= expected)?;
        pump(600);
        Ok(std::mem::take(&mut *log.borrow_mut()))
    };
    let isolated = |script: &str| {
        native.evaluate_javascript(
            script,
            Some("canopy-reading-memory"),
            None,
            None::<&gio::Cancellable>,
            |_| {},
        )
    };

    controller.search("canopy", options, u32::MAX);
    assert_eq!(settle(1, "first search")?, ["found 12"]);
    assert_eq!(selection(ui)?, "m1");
    for (next, expected) in [
        (true, "m2"),
        (true, "m3"),
        (false, "m2"),
        (false, "m1"),
        (false, "m12"),
        (true, "m1"),
    ] {
        if next {
            controller.search_next()
        } else {
            controller.search_previous()
        }
        assert_eq!(settle(1, "step")?, ["found 12"]);
        assert_eq!(selection(ui)?, expected, "a step moves one match and wraps");
    }
    println!("PASS WebKit: a fresh search selects the first match, and each step moves one match, wrapping both ways.");

    controller.search_next();
    controller.search_next();
    settle(2, "steps to m3")?;
    controller.search("line", options, u32::MAX);
    settle(1, "another query")?;
    println!(
        "FINDING WebKit: from m3, a new query starts at {}, not at the top.",
        selection(ui)?
    );
    for round in 0..6 {
        controller.search("canopy", options, u32::MAX);
        for _ in 0..round % 4 {
            controller.search_next();
        }
        settle(1 + round % 4, "move away from the top")?;
        isolated("getSelection().removeAllRanges()");
        controller.search("line", options, u32::MAX);
        assert_eq!(settle(1, "search after clearing")?, ["found 12"]);
        assert_eq!(
            selection(ui)?,
            "p1",
            "a cleared selection makes the search start at the top"
        );
    }
    println!("PASS WebKit: clearing the selection in the isolated world, unawaited, makes the next search start at the first match.");

    controller.search("canopy", options, u32::MAX);
    for _ in 0..4 {
        controller.search_next();
    }
    settle(5, "steps to m5")?;
    isolated("globalThis.__probe = getSelection().getRangeAt(0).cloneRange()");
    eval(
        ui,
        "getSelection().collapse(document.body, 0); JSON.stringify(true)",
    )?;
    controller.search_next();
    settle(1, "step after a click")?;
    println!(
        "FINDING WebKit: from m5, after a click moved the selection to the top, next selects {}.",
        selection(ui)?
    );
    eval(
        ui,
        "getSelection().collapse(document.body, 0); JSON.stringify(true)",
    )?;
    isolated("getSelection().removeAllRanges(); getSelection().addRange(globalThis.__probe)");
    controller.search_next();
    assert_eq!(settle(1, "step after restoring")?, ["found 12"]);
    assert_eq!(
        selection(ui)?,
        "m6",
        "restoring the match keeps the step relative to it"
    );
    println!("PASS WebKit: restoring the last match in the isolated world, unawaited, keeps the next step relative to it.");

    controller.search("forest", options, u32::MAX);
    controller.search_finish();
    assert_eq!(settle(1, "search then finish")?, ["found 5"]);
    controller.search_finish();
    pump(600);
    assert!(
        log.borrow().is_empty(),
        "search_finish alone sends no answer"
    );
    controller.search("canopy", options, u32::MAX);
    controller.search("forest", options, u32::MAX);
    controller.search("zebra", options, u32::MAX);
    controller.search_next();
    assert_eq!(
        settle(4, "burst")?,
        ["found 12", "found 5", "failed", "failed"]
    );
    println!("PASS WebKit: search_finish right after a search still answers that search, and back-to-back requests answer once each, in order.");

    controller.search("canopy", options, u32::MAX);
    native.load_uri(at("127.0.0.1", "/three")?.as_str());
    assert_eq!(settle(1, "search then same-site load")?.len(), 1);
    loaded(ui, "Three fixture")?;
    controller.search("canopy", options, u32::MAX);
    native.load_uri(at("localhost", "/other")?.as_str());
    assert_eq!(settle(1, "search then cross-site load")?.len(), 1);
    loaded(ui, "Other fixture")?;
    native.load_uri(at("127.0.0.1", "/slow")?.as_str());
    wait("provisional cross-site load", || native.is_loading())?;
    pump(200);
    controller.search("canopy", options, u32::MAX);
    assert_eq!(settle(1, "search during a provisional load")?.len(), 1);
    wait("slow load finished", || !native.is_loading())?;
    pump(300);
    assert!(log.borrow().is_empty(), "a load sends no answer by itself");
    controller.search("canopy", options, u32::MAX);
    native.set_child_visible(false);
    let hidden = settle(1, "answer to a hidden view");
    native.set_child_visible(true);
    assert_eq!(hidden?.len(), 1);
    println!("PASS WebKit: a search answers exactly once around same-site, cross-site and provisional loads, and while hidden.");

    controller.search("canopy", options, u32::MAX);
    native.terminate_web_process();
    let mut crash = settle(2, "search then crash")?;
    crash.sort();
    assert_eq!(
        crash,
        ["failed", "terminated"],
        "a crash still ends the request"
    );
    controller.search("canopy", options, u32::MAX);
    assert_eq!(
        settle(1, "search on a crashed view")?,
        ["failed"],
        "a view with no process still answers"
    );
    println!("PASS WebKit: a crash ends the request in flight, and a search on the crashed view still answers.");

    controller.disconnect(found);
    controller.disconnect(failed);
    native.disconnect(terminated);
    Ok(())
}
