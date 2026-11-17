mod common;
#[path = "common/http.rs"]
mod http;

use anyhow::Result;
use canopy::{address::web_url, browser::NavigationTarget, pages::Go, ui::Ui};
use common::*;
use gtk::prelude::*;
use std::{io::Write, net::TcpListener, thread};
use webkit2gtk::WebViewExt;

fn scaled(ui: &Ui, percent: u16) -> Result<()> {
    let result = wait(&format!("native and badge at {percent}%"), || {
        (ui.pages.selected_page().native.zoom_level() - f64::from(percent) / 100.).abs() < 0.0001
            && chrome(ui, "canopy.state().pageScale.percent").is_ok_and(|p| p == percent)
            && visible(ui, "#page-scale") == (percent != 100)
    });
    if result.is_err() {
        eprintln!("scale timeout: expected={percent}, native={}, chrome={:?}, current={:?}", ui.pages.selected_page().native.zoom_level(), chrome(ui, "({scale:canopy.state().pageScale,current:S.frame.current,cover:canopy.state().reported})"), ui.snapshot()?.current);
    }
    result?;
    if percent != 100 {
        assert_eq!(text(ui, "#page-scale"), format!("{percent}%"));
    }
    assert_eq!(ui.chrome.native().zoom_level(), 1.);
    assert_eq!(cover(ui), Some(94));
    Ok(())
}

fn navigate(ui: &Ui, url: &str, target: NavigationTarget) -> Result<canopy::browser::VisitId> {
    ui.pages.go(Go::Address {
        url: web_url(url)?,
        target,
    })?;
    wait("page committed", || {
        ui.snapshot()
            .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.as_str() == url))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    Ok(ui.snapshot()?.current.unwrap())
}

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = format!("http://{}", listener.local_addr()?);
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let Ok(request) = http::request(&mut stream) else {
                    return;
                };
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                if path == "/redirect" {
                    let host = request
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("Host: ")
                                .or_else(|| line.strip_prefix("host: "))
                        })
                        .unwrap();
                    let host = host.replace("127.0.0.1", "localhost");
                    let _ = write!(stream, "HTTP/1.1 302 Found\r\nLocation: http://{host}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    return;
                }
                if path == "/slow" {
                    thread::sleep(std::time::Duration::from_secs(2));
                }
                let body = format!("<!doctype html><title>Scale {path}</title><body style='height:5000px;font:20px sans-serif'><h1>Page size</h1><p>Text, images and links scale together.</p><a id=link href='/background' style='display:block;margin-top:150px'>Background page</a></body>");
                let _ = write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            });
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+equal")?;
    assert!(!visible(&ui, "#page-scale"));
    let root = navigate(&ui, &format!("{origin}/root"), NavigationTarget::Root)?;
    scaled(&ui, 100)?;
    ui.pages.selected_page().focus();
    for combo in [
        "ctrl+equal",
        "ctrl+plus",
        "ctrl+shift+plus",
        "ctrl+shift+equal",
        "ctrl+KP_Add",
        "ctrl+shift+KP_Add",
        "ctrl+KP_Equal",
    ] {
        key(combo)?;
        scaled(&ui, 110)?;
        key("ctrl+0")?;
        scaled(&ui, 100)?;
    }
    for combo in ["ctrl+minus", "ctrl+KP_Subtract"] {
        key(combo)?;
        scaled(&ui, 90)?;
        key("ctrl+KP_0")?;
        scaled(&ui, 100)?;
    }
    for _ in 0..25 {
        key("ctrl+equal")?;
    }
    scaled(&ui, 500)?;
    for _ in 0..25 {
        key("ctrl+minus")?;
    }
    scaled(&ui, 25)?;
    key("ctrl+0")?;
    println!(
        "PASS scale shortcuts, keypad, shifted plus, clamps and reset match native and chrome."
    );

    let (x, y) = screen_point(&ui, &ui.pages.selected_page().native, 400, 300)?;
    move_to(x, y)?;
    xdo(&["keydown", "ctrl", "click", "4", "keyup", "ctrl"])?;
    scaled(&ui, 110)?;
    pump(500);
    scaled(&ui, 110)?;
    xdo(&["keydown", "ctrl", "click", "5", "keyup", "ctrl"])?;
    scaled(&ui, 100)?;
    pump(500);
    scaled(&ui, 100)?;
    xdo(&["click", "5"])?;
    pump(200);
    scaled(&ui, 100)?;
    assert!(eval(&ui, "JSON.stringify(scrollY)")?.as_i64().unwrap() > 0);
    eval(&ui, "scrollTo(0,0);JSON.stringify(true)")?;
    println!("PASS handled Ctrl+wheel takes exactly one step with no later WebKit step; plain wheel scrolls.");

    key("ctrl+equal")?;
    key("ctrl+equal")?;
    key("ctrl+equal")?;
    scaled(&ui, 150)?;
    let first_view = ui.pages.selected_page().native.clone();
    let other = navigate(&ui, &format!("{origin}/other"), NavigationTarget::Root)?;
    scaled(&ui, 150)?;
    key("ctrl+minus")?;
    scaled(&ui, 125)?;
    assert_eq!(
        first_view.zoom_level(),
        1.25,
        "hidden same-origin view updates immediately"
    );
    key("ctrl+1")?;
    wait("Ctrl+1 selects first tree", || {
        ui.snapshot().unwrap().current == Some(root)
    })?;
    scaled(&ui, 125)?;
    key("ctrl+2")?;
    wait("Ctrl+2 selects second tree", || {
        ui.snapshot().unwrap().current == Some(other)
    })?;
    println!("PASS new trees inherit site scale, hidden views update immediately and Ctrl+1/2 still switch trees.");

    navigate(
        &ui,
        &format!("{origin}/other#part"),
        NavigationTarget::ChildOf(other),
    )?;
    scaled(&ui, 125)?;
    ui.pages.go(Go::Parent)?;
    wait("same-document parent visit", || {
        ui.snapshot().unwrap().current == Some(other)
    })?;
    scaled(&ui, 125)?;
    println!(
        "PASS same-document visit changes keep the percentage badge tied to the current visit."
    );

    let count = ui.snapshot()?.visits.len();
    // element_point reports CSS pixels; native pointer input uses scaled widget pixels.
    let point = eval(&ui,"JSON.stringify((() => { const r=link.getBoundingClientRect();return [r.x+10,r.y+10]; })())")?;
    let (x, y) = screen_point(
        &ui,
        &ui.pages.selected_page().native,
        (point[0].as_f64().unwrap() * 1.25) as i32,
        (point[1].as_f64().unwrap() * 1.25) as i32,
    )?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    wait("background page queued", || {
        ui.snapshot().unwrap().visits.len() == count + 1
    })?;
    let queued = ui.snapshot()?.visits.last().unwrap().id;
    wait("background committed with site scale", || {
        ui.pages
            .background_view(queued)
            .is_some_and(|v| !v.is_loading() && v.zoom_level() == 1.25)
    })?;
    let background = ui.pages.background_view(queued).unwrap();
    key("ctrl+equal")?;
    scaled(&ui, 150)?;
    assert_eq!(background.zoom_level(), 1.5);
    ui.pages.go(Go::Visit(queued))?;
    wait("preloaded page promoted", || {
        ui.snapshot().unwrap().current == Some(queued)
    })?;
    scaled(&ui, 150)?;
    assert_eq!(ui.pages.selected_page().native, background);
    println!(
        "PASS background commits apply policy before promotion and receive same-origin changes."
    );

    ui.pages.settle_branch(queued)?;
    wait("settled branch releases view", || {
        !ui.pages.live_visits().contains(&queued)
    })?;
    key("ctrl+minus")?;
    scaled(&ui, 125)?;
    ui.pages.go(Go::Visit(queued))?;
    wait("checkpoint revived", || {
        ui.snapshot().unwrap().current == Some(queued)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    scaled(&ui, 125)?;
    println!(
        "PASS checkpoint revival uses the current site policy instead of its old native scale."
    );

    let different = origin.replace("127.0.0.1", "localhost");
    navigate(
        &ui,
        &format!("{different}/different"),
        NavigationTarget::Root,
    )?;
    scaled(&ui, 100)?;
    ui.pages
        .selected_page()
        .native
        .load_uri(&format!("{origin}/slow"));
    pump(150);
    key("ctrl+equal")?;
    // The old document is still displayed while native.uri already names the destination.
    scaled(&ui, 110)?;
    wait("slow replacement", || {
        !ui.pages.selected_page().native.is_loading()
    })?;
    scaled(&ui, 125)?;
    ui.pages
        .selected_page()
        .native
        .load_uri(&format!("{origin}/redirect"));
    wait("redirect final", || {
        ui.pages
            .selected_page()
            .native
            .uri()
            .is_some_and(|u| u.ends_with("/final"))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    scaled(&ui, 110)?;
    println!("PASS provisional URI cannot change the policy target; committed redirects use their final origin.");

    key("ctrl+equal")?;
    key("ctrl+equal")?;
    scaled(&ui, 150)?;
    for theme in ["dark", "light"] {
        open_settings(&ui)?;
        click_chrome(&ui, &format!("#theme-mode [data-mode={theme}]"))?;
        key("Escape")?;
        scaled(&ui, 150)?;
        if let Ok(dir) = std::env::var("CANOPY_SCALE_SHOTS") {
            save_window(&ui, &format!("{dir}/scale-150-{theme}.png"))?;
        }
    }
    click_chrome(&ui, "#page-scale")?;
    scaled(&ui, 100)?;
    if let Ok(dir) = std::env::var("CANOPY_SCALE_SHOTS") {
        save_window(&ui, &format!("{dir}/scale-reset.png"))?;
    }
    println!("PASS percentage badge resets scale and fits both chrome themes.");
    let database = rusqlite::Connection::open(directory.path().join("forest.sqlite3"))?;
    database.execute_batch("CREATE TRIGGER refuse_scale BEFORE INSERT ON site_page_scale BEGIN SELECT RAISE(ABORT,'Scale write refused'); END;")?;
    key("ctrl+equal")?;
    scaled(&ui, 100)?;
    wait("refused scale write error", || {
        error_toast_text(&ui).is_some_and(|text| text.contains("Scale write refused"))
    })?;
    database.execute_batch("DROP TRIGGER refuse_scale;")?;
    println!("PASS a failed scale write leaves the page and badge unchanged.");
    key("ctrl+equal")?;
    scaled(&ui, 110)?;
    ui.window.close();
    drop(ui);
    drop(first_view);
    drop(background);
    pump(300);
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.resume(&[])?;
    wait("restored page", || {
        ui.snapshot().unwrap().current.is_some() && !ui.pages.selected_page().native.is_loading()
    })?;
    scaled(&ui, 110)?;
    println!("PASS restart restores the remembered site level in the native view and badge.");
    database.execute("UPDATE site_page_scale SET percent = 101", [])?;
    ui.pages.reload()?;
    wait("reload with invalid stored scale", || {
        !ui.pages.selected_page().native.is_loading()
    })?;
    scaled(&ui, 100)?;
    wait("invalid stored scale error", || {
        error_toast_text(&ui).is_some_and(|text| text.contains("Invalid stored page scale"))
    })?;
    println!("PASS commit-time policy errors reset native scale to 100% and report the error.");
    ui.window.close();
    Ok(())
}
