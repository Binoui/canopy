use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use anyhow::Result;
use canopy::{address::web_url, browser::NavigationTarget, ui::Ui};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

mod common;
use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = web_url(&format!("http://{}/", listener.local_addr()?))?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        let mut loads = 0;
        while loads < 3 {
            let (mut stream, _) = listener.accept()?;
            let mut request = [0; 4096];
            let size = stream.read(&mut request)?;
            if request[..size].starts_with(b"GET / HTTP/") {
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
    server.join().unwrap()?;
    ui.window.close();
    Ok(())
}
