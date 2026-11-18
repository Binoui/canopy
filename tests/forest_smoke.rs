use anyhow::Result;
use canopy::{
    browser::{Browser, ThemeMode},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
use common::*;

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let dir = tempfile::tempdir()?;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            let body = "<!doctype html><title>Forest fixture</title>Local page";
            let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        }
    });
    let mut b = Browser::open(&dir.path().join("forest.sqlite3"))?;
    let roots = b.add_roots(
        &(0..131)
            .map(|i| url::Url::parse(&format!("http://fixture.localhost:{port}/{i}")).unwrap())
            .collect::<Vec<_>>(),
    )?;
    for (i, id) in roots.iter().enumerate() {
        let tree = b
            .tree_states()?
            .into_iter()
            .find(|s| s.root.root() == *id)
            .unwrap()
            .root;
        b.rename_tree(tree, &format!("Project {i}"))?;
        if i >= 4 {
            b.settle_tree(tree)?;
        }
        if i == 0 || i == 4 {
            b.set_pinned(tree, true)?;
        }
    }
    drop(b);
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 800);
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+e")?;
    wait("forest", || visible(&ui, "#forest"))?;
    key("Tab")?;
    assert!(chrome(&ui, "!!document.activeElement.closest('#forest')")? == true);
    key("ctrl+h")?;
    wait("History", || visible(&ui, ".forest-row"))?;
    key("ctrl+h")?;
    assert!(!visible(&ui, "#forest"));
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(100);
    }
    println!("PASS shared forest shortcuts, focus containment and themes.");
    ui.window.close();
    Ok(())
}
