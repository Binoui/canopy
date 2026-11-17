#[path = "common/http.rs"]
mod http;
use std::{cell::RefCell, io::Write, net::TcpListener, rc::Rc, thread, time::Duration};

use anyhow::{Context, Result};
use canopy::{
    address::web_url,
    browser::NavigationTarget,
    pages::{FindAction, FindTarget},
    ui::Ui,
};
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
    watch_keys(&ui)?;
    open_and_type(&ui)?;
    steps_wrap(&ui)?;
    keys_from_the_page(&ui)?;
    refocus_and_edit(&ui)?;
    escape_closes(&ui)?;
    reopen_drains(&ui)?;
    surfaces_close_first(&ui)?;
    same_document_keeps(&ui)?;
    new_document_closes(&ui)?;
    switch_closes(&ui, &at)?;
    ui.pages
        .selected_page()
        .native
        .load_uri(at("127.0.0.1", "/find")?.as_str());
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

fn find_state(ui: &Ui) -> serde_json::Value {
    chrome(ui, "canopy.state().find").unwrap_or_default()
}

fn count(ui: &Ui) -> String {
    find_state(ui)["count"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// The match the page itself has selected, by the id of the element around it.
fn selection(ui: &Ui) -> Result<String> {
    let value = eval(
        ui,
        "JSON.stringify(getSelection().anchorNode?.parentElement?.id ?? null)",
    )?;
    Ok(value.as_str().unwrap_or("none").to_owned())
}

/// The bar's count and the page's selected match agree.
fn shows(ui: &Ui, expected: &str, selected: &str) -> Result<()> {
    wait(&format!("find shows {expected}"), || {
        count(ui) == expected && find_state(ui)["state"] != "pending"
    })
    .with_context(|| format!("the bar shows {}", find_state(ui)))?;
    assert_eq!(
        selection(ui)?,
        selected,
        "the page selects the match the bar counts as {expected}"
    );
    Ok(())
}

/// Letters and Escape presses that reach the page, counted by the page itself.
fn watch_keys(ui: &Ui) -> Result<()> {
    eval(ui, "window.__keys = 0; window.__escapes = 0; addEventListener('keydown', (e) => { if (e.key.length === 1) __keys++; if (e.key === 'Escape') __escapes++; }, true); JSON.stringify(true)")?;
    Ok(())
}

fn page_counter(ui: &Ui, name: &str) -> Result<i64> {
    Ok(eval(ui, &format!("JSON.stringify(window.{name})"))?
        .as_i64()
        .unwrap_or(-1))
}

/// Whether WebKit paints `id`, a match that is not the selected one, with its yellow
/// highlight. Reads the window's pixels at the element's top-left corner.
fn highlighted(ui: &Ui, id: &str, on: bool) -> Result<()> {
    let label = format!("{id} {}", if on { "highlighted" } else { "plain" });
    wait(&label, || {
        let Ok(corner) = eval(ui, &format!("(() => {{ const r = document.getElementById('{id}').getBoundingClientRect(); return JSON.stringify(r.top > 0 && r.bottom < innerHeight ? [r.left + 1, r.top + 1] : null); }})()")) else {
            return false;
        };
        let (Some(x), Some(y)) = (corner[0].as_f64(), corner[1].as_f64()) else {
            return false;
        };
        let pixel = ui
            .window
            .window()
            .and_then(|window| window.pixbuf(x as i32, y as i32 + 94, 1, 1));
        pixel.is_some_and(|pixel| (pixel.read_pixel_bytes()[2] < 128) == on)
    })
}

fn open_and_type(ui: &Ui) -> Result<()> {
    ui.pages.selected_page().focus();
    pump(100);
    assert!(ui.pages.selected_page().native.has_focus());
    highlighted(ui, "m2", false)?;
    focus_window()?;
    xdo(&[
        "key",
        "--clearmodifiers",
        "ctrl+f",
        "type",
        "--delay",
        "1",
        "canopy",
    ])?;
    shows(ui, "1 of 12", "m1")?;
    assert_eq!(
        find_state(ui)["input"],
        "canopy",
        "every letter typed right after Ctrl+F lands in the bar"
    );
    assert_eq!(page_counter(ui, "__keys")?, 0, "no letter reaches the page");
    assert!(chrome_focused(ui, "#find-input"));
    highlighted(ui, "m2", true)?;
    let (x, y, _, height) = rect(ui, "#find-bar")?;
    let (pill, _, pill_width, _) = rect(ui, "#search-pill")?;
    assert!(
        y >= 0. && y + height <= 42.,
        "the bar sits in the 42 px header"
    );
    assert!(x >= pill + pill_width, "the bar follows the search pill");
    assert_eq!(cover(ui), Some(94), "the chrome keeps its 94 px band");
    println!("PASS Ctrl+F opens the bar in the header, the first letters land in it, and the first match shows as 1 of 12.");
    Ok(())
}

fn steps_wrap(ui: &Ui) -> Result<()> {
    key("Return")?;
    shows(ui, "2 of 12", "m2")?;
    key("Return")?;
    shows(ui, "3 of 12", "m3")?;
    focus_window()?;
    xdo(&[
        "key",
        "--clearmodifiers",
        "--repeat",
        "9",
        "--delay",
        "15",
        "Return",
    ])?;
    shows(ui, "12 of 12", "m12")?;
    key("Return")?;
    shows(ui, "1 of 12", "m1")?;
    key("shift+Return")?;
    shows(ui, "12 of 12", "m12")?;
    key("shift+Return")?;
    shows(ui, "11 of 12", "m11")?;
    println!("PASS Enter and Shift+Enter step one match at a time, quick presses queue, and both directions wrap.");
    Ok(())
}

fn keys_from_the_page(ui: &Ui) -> Result<()> {
    key("F6")?;
    wait("F6 gives the page the keyboard", || {
        ui.pages.selected_page().native.has_focus()
    })?;
    assert!(!find_state(ui).is_null(), "F6 leaves the bar open");
    for (combo, expected, selected) in [
        ("F3", "12 of 12", "m12"),
        ("ctrl+g", "1 of 12", "m1"),
        ("shift+F3", "12 of 12", "m12"),
        ("ctrl+shift+g", "11 of 12", "m11"),
    ] {
        key(combo)?;
        shows(ui, expected, selected)?;
    }
    println!("PASS F6 returns to the page with the bar open, and F3, Ctrl+G and their Shift forms step from there.");
    let point = eval(ui, "(() => { const r = document.getElementById('p11').getBoundingClientRect(); return JSON.stringify({x: r.left + 6, y: r.top + r.height / 2}); })()")?;
    let (x, y) = screen_point(
        ui,
        &ui.pages.selected_page().native,
        point["x"].as_f64().unwrap_or_default() as i32,
        point["y"].as_f64().unwrap_or_default() as i32,
    )?;
    click_at(x, y)?;
    wait("the click moves the page's selection", || {
        selection(ui).is_ok_and(|id| id == "p11")
    })?;
    key("ctrl+g")?;
    shows(ui, "12 of 12", "m12")?;
    println!("PASS a click in the page between steps does not shift the count: the next step starts from the counted match.");
    Ok(())
}

fn refocus_and_edit(ui: &Ui) -> Result<()> {
    let keys = page_counter(ui, "__keys")?;
    key("ctrl+f")?;
    wait("Ctrl+F refocuses the bar and selects its text", || {
        chrome_focused(ui, "#find-input")
            && chrome(ui, "document.getElementById('find-input').selectionStart === 0 && document.getElementById('find-input').selectionEnd === 6").is_ok_and(|v| v == true)
    })?;
    assert_eq!(count(ui), "12 of 12", "refocusing does not search again");
    type_text("forest")?;
    shows(ui, "1 of 5", "f1")?;
    highlighted(ui, "f2", true)?;
    assert_eq!(page_counter(ui, "__keys")?, keys);
    key("ctrl+a")?;
    type_text("zebra")?;
    wait("no matches", || count(ui) == "No matches")?;
    assert_eq!(
        chrome(ui, "document.getElementById('find-next').disabled && document.getElementById('find-previous').disabled")?,
        true
    );
    key("Return")?;
    pump(200);
    assert_eq!(count(ui), "No matches");
    highlighted(ui, "f2", false)?;
    key("ctrl+a")?;
    key("BackSpace")?;
    wait("an empty query shows no count", || {
        find_state(ui)["state"] == "empty" && count(ui).is_empty()
    })?;
    println!("PASS Ctrl+F again selects the query, a new query starts at 1 of 5, No matches disables stepping, and an empty query clears the count.");
    Ok(())
}

fn escape_closes(ui: &Ui) -> Result<()> {
    type_text("canopy")?;
    shows(ui, "1 of 12", "m1")?;
    highlighted(ui, "m2", true)?;
    key("Escape")?;
    wait("Escape closes the bar", || find_state(ui).is_null())?;
    wait("the page has the keyboard", || {
        ui.pages.selected_page().native.has_focus()
    })?;
    highlighted(ui, "m2", false)?;
    assert!(!visible(ui, "#find-bar"));
    assert_eq!(
        page_counter(ui, "__escapes")?,
        0,
        "the bar took that Escape"
    );
    key("Escape")?;
    wait("the next Escape reaches the page", || {
        page_counter(ui, "__escapes").is_ok_and(|n| n == 1)
    })?;
    println!("PASS Escape closes the bar, clears the highlights and returns the keyboard to the page; the next Escape belongs to the page.");
    Ok(())
}

/// Close and reopen while the first search is still in flight: all five commands run in
/// one turn, before WebKit can answer. Real keys cannot promise that timing.
fn reopen_drains(ui: &Ui) -> Result<()> {
    for action in [
        FindAction::Open,
        FindAction::Query("canopy".into()),
        FindAction::Close,
        FindAction::Open,
        FindAction::Query("forest".into()),
    ] {
        ui.pages.find(FindTarget::Selected, action);
    }
    let landed = serde_json::json!({ "state": "position", "index": 1, "total": 5 });
    wait("the reopened session's search", || {
        find_state(ui)["matches"] == landed
    })?;
    assert_eq!(find_state(ui)["query"], "forest");
    assert_eq!(selection(ui)?, "f1");
    pump(500);
    assert_eq!(
        find_state(ui)["matches"],
        landed,
        "the closed session's reply changed nothing"
    );
    key("Escape")?;
    wait("closed", || find_state(ui).is_null())?;
    println!(
        "PASS closing and reopening while a search is in flight ends on the new query, 1 of 5."
    );
    Ok(())
}

fn surfaces_close_first(ui: &Ui) -> Result<()> {
    key("ctrl+f")?;
    type_text("forest")?;
    shows(ui, "1 of 5", "f1")?;
    key("ctrl+k")?;
    wait("palette open", || {
        chrome(ui, "canopy.state().palette").is_ok_and(|v| v == true)
    })?;
    assert!(!find_state(ui).is_null());
    key("Escape")?;
    wait("Escape closes the palette first", || {
        chrome(ui, "canopy.state().palette").is_ok_and(|v| v == false)
    })?;
    pump(200);
    assert!(
        !find_state(ui).is_null(),
        "find outlives the palette's Escape"
    );
    assert!(
        chrome_focused(ui, "#find-input"),
        "the palette gives the keyboard back to the bar"
    );
    key("Escape")?;
    wait("the second Escape closes find", || find_state(ui).is_null())?;
    println!("PASS an open palette takes Escape before the find bar.");
    Ok(())
}

fn same_document_keeps(ui: &Ui) -> Result<()> {
    key("ctrl+f")?;
    type_text("canopy")?;
    shows(ui, "1 of 12", "m1")?;
    eval(ui, "location.hash = 'here'; JSON.stringify(true)")?;
    wait("fragment in the address", || {
        ui.pages
            .selected_page()
            .native
            .uri()
            .is_some_and(|uri| uri.ends_with("#here"))
    })?;
    pump(400);
    assert!(
        !find_state(ui).is_null(),
        "a same-document change keeps find"
    );
    key("ctrl+g")?;
    shows(ui, "2 of 12", "m2")?;
    println!("PASS a same-document address change keeps find open and counting.");
    Ok(())
}

fn new_document_closes(ui: &Ui) -> Result<()> {
    eval(ui, "location.href = '/three'; JSON.stringify(true)")?;
    loaded(ui, "Three fixture")?;
    wait("a new document closes find", || find_state(ui).is_null())?;
    watch_keys(ui)?;
    key("Escape")?;
    wait("Escape reaches the new document", || {
        page_counter(ui, "__escapes").is_ok_and(|n| n == 1)
    })?;
    println!("PASS loading another document closes find, and Escape then reaches that page.");
    Ok(())
}

fn switch_closes(ui: &Ui, at: &dyn Fn(&str, &str) -> Result<Url>) -> Result<()> {
    key("ctrl+f")?;
    type_text("canopy")?;
    shows(ui, "1 of 3", "m1")?;
    let old = find_state(ui)["id"].as_u64().context("session id")?;
    ui.pages
        .selected_page()
        .navigate(at("localhost", "/second")?, NavigationTarget::Root)?;
    loaded(ui, "Other fixture")?;
    wait("switching pages closes find", || find_state(ui).is_null())?;
    chrome(
        ui,
        &format!("intent('find', {{ session: {old}, action: 'query', text: 'canopy' }})"),
    )?;
    chrome(
        ui,
        &format!("intent('find', {{ session: {old}, action: 'next' }})"),
    )?;
    pump(500);
    assert!(find_state(ui).is_null(), "the old session takes no command");
    assert_eq!(
        selection(ui)?,
        "none",
        "nothing was searched on the new page"
    );
    highlighted(ui, "m2", false)?;
    key("ctrl+1")?;
    loaded(ui, "Three fixture")?;
    pump(500);
    assert!(
        find_state(ui).is_null(),
        "returning to the page does not bring find back"
    );
    highlighted(ui, "m2", false)?;
    println!("PASS switching pages closes find, the old session takes no command, and returning keeps it closed.");
    Ok(())
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
