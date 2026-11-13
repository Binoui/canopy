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
        let (mut stream, _) = listener.accept()?;
        let mut request = [0; 4096];
        stream.read(&mut request)?;
        let body = "<!doctype html><title>Shortcut fixture</title><p>Keyboard shortcuts</p>";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    });
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.engine.navigate(url, NavigationTarget::Root)?;
    wait("fixture page", || {
        ui.snapshot().unwrap().current.is_some() && !ui.engine.native.is_loading()
    })?;
    server.join().unwrap()?;
    wait("reading surface", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|surface| surface == "reading")
    })?;
    ui.engine.focus();
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
    ui.window.close();
    Ok(())
}
