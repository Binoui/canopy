use anyhow::Result;
use canopy::chrome::ChromeHost;
use canopy::{
    browser::{Browser, Move, Place},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
use common::*;
fn selected(ui: &Ui) -> String {
    chrome(
        ui,
        "document.querySelector('.forest-row[aria-selected=true]').dataset.id",
    )
    .unwrap()
    .as_str()
    .unwrap()
    .to_owned()
}
fn count(ui: &Ui) -> usize {
    ui.pages
        .read(|b| b.tree_states())
        .unwrap()
        .iter()
        .filter(|s| matches!(s.place(), Place::History { .. }))
        .count()
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
            let body = "<!doctype html><title>History fixture</title>Local page";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let mut b = Browser::open(&dir.path().join("forest.sqlite3"))?;
    let roots = b.add_roots(
        &(0..132)
            .map(|i| {
                url::Url::parse(&format!("http://site{}.localhost:{port}/{i}", i % 12)).unwrap()
            })
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
        if i > 0 {
            b.place_tree(tree, Move::History)?;
        }
    }
    drop(b);
    let db = rusqlite::Connection::open(dir.path().join("forest.sqlite3"))?;
    let now = glib::DateTime::now_local()?;
    for (i, id) in roots.iter().enumerate().skip(1) {
        let day = now.add_days(if i < 81 { 0 } else { -(i as i32 * 4) })?;
        db.execute(
            "UPDATE tree_state SET placed_at=? WHERE root_id=?",
            rusqlite::params![day.to_unix() * 1000 + i as i64, id.to_string()],
        )?;
    }
    drop(db);
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 800);
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+h")?;
    wait("History rows", || visible(&ui, ".forest-row"))?;
    assert_eq!(
        chrome(&ui, "document.querySelectorAll('.forest-row').length")?,
        20
    );
    assert!(chrome(&ui, "!document.querySelector('#forest-preview')")? == true);
    assert!(text(&ui, "#history-count").contains("131 of 131"));
    assert_eq!(chrome(&ui, "S.forest.history.rail.sites.length")?, 9);
    assert!(text(&ui, "#forest-list").contains("One page"));
    assert!(text(&ui, ".forest-day").contains("80 trees"));
    let first = selected(&ui);
    key("Down")?;
    assert_ne!(selected(&ui), first);
    assert!(!visible(&ui, ".history-page"));
    key("Right")?;
    wait("unfolded outline", || visible(&ui, ".history-page"))?;
    key("Left")?;
    assert!(!visible(&ui, ".history-page"));
    key("space")?;
    assert!(visible(&ui, ".history-page"));
    click_chrome(&ui, ".forest-row[aria-selected=true]")?;
    assert!(!visible(&ui, ".history-page"));
    key("Right")?;
    chrome(
        &ui,
        "(globalThis.kept=[...document.querySelector('.history-item.selected').children],true)",
    )?;
    let tree = ui
        .pages
        .read(|b| b.tree_states())?
        .into_iter()
        .find(|s| s.root.root().to_string() == selected(&ui))
        .unwrap()
        .root;
    ui.dispatch(canopy::chrome::ChromeIntent::RenameTree(
        tree,
        "Renamed History".into(),
    ));
    wait("renamed row", || {
        text(&ui, "#forest-list").contains("Renamed History")
    })?;
    assert!(
        chrome(
            &ui,
            "kept.every((el,i)=>el===document.querySelector('.history-item.selected').children[i])"
        )? == true
    );
    key("Delete")?;
    wait("Delete and successor", || {
        count(&ui) == 130 && visible(&ui, "#forest-undo button")
    })?;
    key("z")?;
    assert!(visible(&ui, "#forest-undo button"));
    key("Escape")?;
    click_chrome(&ui, "#forest-undo button")?;
    wait("Undo", || count(&ui) == 131)?;
    click_chrome(&ui, ".forest-day button")?;
    wait("Delete entire day", || {
        count(&ui) == 51 && text(&ui, "#forest-undo").contains("80 trees")
    })?;
    click_chrome(&ui, "#forest-undo button")?;
    wait("Undo day", || count(&ui) == 131)?;
    println!("PASS History settle-day groups and complete day counts, one-page trails, independent selection/folds, retained row/action/outline nodes, Delete and Undo, full Delete day and Undo.");
    chrome(&ui, "($('forest-list').scrollTop=0,true)")?;
    click_chrome(&ui, ".forest-row")?;
    key("Left")?;
    chrome(
        &ui,
        "($('forest-list').scrollTop=$('forest-list').scrollHeight,true)",
    )?;
    wait("older rows load without pulling back selection", || {
        chrome(
            &ui,
            "document.querySelectorAll('.forest-row').length>=40 && $('forest-list').scrollTop>100",
        )
        .is_ok_and(|v| v == true)
    })?;
    key("p")?;
    type_text("roject")?;
    wait("Find starts at the first result", || {
        chrome(
            &ui,
            "S.forest.query==='project' && $('forest-list').scrollTop<50",
        )
        .is_ok_and(|v| v == true)
    })?;
    key("Escape")?;
    key("z")?;
    wait("typing starts Find", || {
        text(&ui, "#forest-empty").contains("matches")
    })?;
    key("Escape")?;
    wait("Find clears", || visible(&ui, ".forest-row"))?;
    assert!(visible(&ui, "#forest"));
    click_chrome(&ui, "#history-rail [data-forest=site]")?;
    assert!(!visible(&ui, ".forest-day button"));
    let filtered = chrome(&ui, "S.forest.history.count")?;
    click_chrome(&ui, "#history-rail [data-forest=site]")?;
    assert_ne!(chrome(&ui, "S.forest.history.count")?, filtered);
    click_chrome(&ui, "#history-rail [data-bucket^=month-]")?;
    wait("old month jump", || visible(&ui, "[data-before=true]"))?;
    assert!(chrome(&ui, "document.querySelectorAll('.forest-row').length<=40")? == true);
    let before = chrome(&ui, "S.forest.history.before.edge")?;
    chrome(&ui, "($('forest-list').scrollTop=0,true)")?;
    wait("newer edge loads", || {
        chrome(&ui, "S.forest.history.before?.edge ?? null").is_ok_and(|v| v != before)
    })?;
    key("Escape")?;
    assert!(!visible(&ui, "#forest"));
    key("ctrl+h")?;
    wait("History again", || visible(&ui, ".forest-row"))?;
    key("s")?;
    wait("Save moves successor", || count(&ui) == 130)?;
    assert!(visible(&ui, "#forest"));
    key("Right")?;
    let page = chrome(&ui, "document.querySelector('.history-page').dataset.id")?;
    let (x, y) = chrome_point(&ui, ".history-page")?;
    move_to(x, y)?;
    xdo(&["mousedown", "1"])?;
    let tree = ui
        .pages
        .read(|b| b.tree_states())?
        .into_iter()
        .find(|s| s.root.root().to_string() == selected(&ui))
        .unwrap()
        .root;
    ui.dispatch(canopy::chrome::ChromeIntent::RenameTree(
        tree,
        "Refreshed under a held outline".into(),
    ));
    wait("held outline refresh", || {
        chrome(&ui, "canopy.state().deferred.includes('forest')").is_ok_and(|v| v == true)
    })?;
    xdo(&["mouseup", "1"])?;
    wait("outline reopens page", || {
        !visible(&ui, "#forest")
            && ui.snapshot().is_ok_and(|s| {
                s.current
                    .is_some_and(|id| Some(id.to_string().as_str()) == page.as_str())
            })
    })?;
    key("ctrl+h")?;
    wait("History for Enter", || visible(&ui, ".forest-row"))?;
    let target = selected(&ui);
    key("Return")?;
    wait("Enter reopens", || {
        !visible(&ui, "#forest")
            && ui
                .snapshot()
                .is_ok_and(|s| s.current.is_some_and(|id| id.to_string() == target))
    })?;
    key("ctrl+h")?;
    wait("History for double click", || visible(&ui, ".forest-row"))?;
    let target = selected(&ui);
    let (x, y) = chrome_point(&ui, ".forest-row[aria-selected=true]")?;
    move_to(x, y)?;
    xdo(&["click", "--repeat", "2", "--delay", "100", "1"])?;
    wait("double click reopens", || {
        !visible(&ui, "#forest")
            && ui
                .snapshot()
                .is_ok_and(|s| s.current.is_some_and(|id| id.to_string() == target))
    })?;
    key("ctrl+h")?;
    wait("History for menu", || visible(&ui, ".forest-row"))?;
    key("Menu")?;
    assert_eq!(
        text(&ui, "#popover"),
        "Reopen\nSave for later\nRename\nDelete"
    );
    key("Escape")?;
    ui.window.resize(390, 800);
    pump(300);
    assert!(visible(&ui, "#history-chips"));
    assert!(!visible(&ui, "#history-rail"));
    ui.window.resize(1920, 900);
    pump(300);
    assert!(visible(&ui, "#history-rail"));
    let deleted = selected(&ui);
    key("Delete")?;
    wait("delete before restart", || {
        !text(&ui, "#forest-list").contains(&deleted)
    })?;
    let remaining = count(&ui);
    ui.window.close();
    drop(ui);
    pump(300);
    let b = Browser::open(&dir.path().join("forest.sqlite3"))?;
    assert!(!b
        .tree_states()?
        .iter()
        .any(|s| s.root.root().to_string() == deleted));
    drop(b);
    let ui = Ui::new(dir.path())?;
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+h")?;
    wait("restart History", || visible(&ui, ".forest-row"))?;
    assert_eq!(count(&ui), remaining);
    assert!(!visible(&ui, "#forest-undo button"));
    println!("PASS typing Find and two-step Escape, site toggle and filtered Delete day guard, old month jump and newer-edge load, Save, outline/Enter reopen, History menu, narrow chips, wide rail and deletion surviving a window restart.");
    ui.window.close();
    Ok(())
}
