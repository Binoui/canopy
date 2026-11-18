use anyhow::Result;
use canopy::{
    browser::Browser,
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
use common::*;

fn held_refresh(ui: &Ui, selector: &str) -> Result<()> {
    let id = chrome(ui, "S.forest.history?.preview?.id ?? document.querySelector('.tree-card[aria-current=true]')?.dataset.id ?? document.querySelector('.tree-card')?.dataset.id")?
        .as_str()
        .unwrap()
        .to_owned();
    let tree = ui
        .pages
        .read(|b| b.tree_states())?
        .into_iter()
        .find(|s| s.root.root().to_string() == id)
        .unwrap()
        .root;
    let (x, y) = chrome_point(ui, selector)?;
    move_to(x, y)?;
    xdo(&["mousedown", "1"])?;
    wait("held forest control", || {
        chrome(ui, "canopy.state().buttons").is_ok_and(|v| v == 1)
    })?;
    ui.dispatch(ChromeIntent::RenameTree(
        tree,
        format!("Refreshed {selector}"),
    ));
    wait("forest refresh held until release", || {
        chrome(ui, "canopy.state().deferred.includes('forest')").is_ok_and(|v| v == true)
    })?;
    xdo(&["mouseup", "1"])?;
    pump(200);
    Ok(())
}

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
            b.place_tree(tree, canopy::browser::Move::History)?;
        }
        if i == 0 {
            b.place_tree(tree, canopy::browser::Move::Pin(true))?;
        }
        if i == 4 {
            b.place_tree(
                tree,
                canopy::browser::Move::Shelf(canopy::browser::Slot::First),
            )?;
        }
    }
    drop(b);
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 800);
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+e")?;
    wait("Trees", || visible(&ui, "#forest-trees .tree-card"))?;
    assert_eq!(
        text(&ui, "#forest-count"),
        "4 tabs · 1 saved for later · 126 in history"
    );
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('#forest-trees .tree-card')].map(c=>c.dataset.id)"
        )?,
        serde_json::json!(
            [roots[0], roots[1], roots[2], roots[3], roots[4]].map(|id| id.to_string())
        )
    );
    assert!(text(&ui, "#forest-trees").contains("Saved"));
    assert!(chrome(&ui,"[...document.querySelectorAll('#forest-trees .tree-card')].every(c=>[...c.querySelectorAll('.pc-page,.pc-meta')].every(e=>e.getBoundingClientRect().bottom<=c.getBoundingClientRect().bottom))")?==true);
    for width in [1240, 2560, 600] {
        ui.window.resize(width, 800);
        pump(200);
        assert_eq!(chrome(&ui,"(()=>{const r=$('forest').querySelector('.forest-pop').getBoundingClientRect();return [r.left,innerWidth-r.right]})()")?,serde_json::json!([22,22]));
        assert!(rect(&ui, "#forest-trees .tree-card")?.2 <= 240.);
    }
    ui.window.resize(1240, 800);
    pump(200);
    held_refresh(&ui, ".tree-card")?;
    assert!(!visible(&ui, "#forest"));
    println!("PASS Trees ordered bands, bounded cards, widths and held activation.");
    ui.window.close();
    Ok(())
}
