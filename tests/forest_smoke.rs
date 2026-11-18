use anyhow::Result;
use canopy::{
    browser::{Browser, ThemeMode},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
use common::*;

fn held_refresh(ui: &Ui, selector: &str) -> Result<()> {
    let id = chrome(ui, "S.forest.history?.preview?.id ?? document.querySelector('.tree-card[aria-current=true]')?.dataset.id")?
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
    wait("Trees", || visible(&ui, "#forest-trees .tree-card"))?;
    assert_eq!(
        text(&ui, "#forest-count"),
        "4 open · 2 saved · 126 in history"
    );
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('#forest-trees .tree-card')].map(c=>c.dataset.id)"
        )?,
        serde_json::json!(
            [roots[0], roots[1], roots[2], roots[3], roots[0], roots[4]].map(|id| id.to_string())
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
    click_chrome(&ui, "#forest-views [data-view=history]")?;
    wait("History", || visible(&ui, ".forest-row"))?;
    assert_eq!(
        chrome(&ui, "document.querySelectorAll('.forest-row').length")?,
        60
    );
    assert!(
        chrome(
            &ui,
            "document.querySelector('.forest-row').getAttribute('aria-selected') === 'true'"
        )? == true
    );
    let first = chrome(&ui, "S.forest.history.preview.id")?;
    let first_selector = format!(".forest-row[data-id='{}']", first.as_str().unwrap());
    key("Down")?;
    assert_ne!(chrome(&ui, "S.forest.history.preview.id")?, first);
    key("Up")?;
    assert_eq!(chrome(&ui, "S.forest.history.preview.id")?, first);
    assert!(visible(&ui, ".pv-row"));
    ui.window.resize(600, 800);
    pump(200);
    assert!(rect(&ui, "#forest-preview")?.1 < rect(&ui, "#forest-list")?.1);
    assert!(rect(&ui, "#forest-preview")?.3 <= 266.);
    ui.window.resize(2560, 900);
    pump(200);
    assert!(rect(&ui, "#forest-list")?.2 <= 720.);
    ui.window.resize(1240, 800);
    pump(200);
    chrome(
        &ui,
        "($('forest-list').scrollTop=$('forest-list').scrollHeight,true)",
    )?;
    pump(100);
    held_refresh(&ui, ".forest-more")?;
    wait("older rows", || {
        chrome(&ui, "document.querySelectorAll('.forest-row').length").is_ok_and(|v| v == 120)
    })?;
    chrome(&ui, "($('forest-list').scrollTop=0,true)")?;
    key("z")?;
    wait("Find letters", || {
        chrome(&ui, "S.forest.query").is_ok_and(|v| v == "z")
    })?;
    assert!(text(&ui, "#forest-empty").contains("matches"));
    key("Escape")?;
    wait("cleared Find", || {
        chrome(&ui, "S.forest.query").is_ok_and(|v| v == "")
    })?;
    assert!(visible(&ui, "#forest"));
    held_refresh(&ui, &first_selector)?;
    key("Menu")?;
    assert_eq!(text(&ui, "#popover"), "Rename\nSave tree");
    click_chrome(&ui, "[data-tab-action=rename]")?;
    type_text("History project renamed")?;
    key("Return")?;
    wait("renamed in forest", || {
        text(&ui, "#forest-list").contains("History project renamed")
    })?;
    assert!(visible(&ui, "#forest"));
    let tree = ui
        .pages
        .read(|b| b.tree_states())?
        .into_iter()
        .find(|s| s.root.root().to_string() == first.as_str().unwrap())
        .unwrap()
        .root;
    chrome(
        &ui,
        "(globalThis.heldControls=[...$('forest-preview').querySelectorAll('button')],true)",
    )?;
    ui.dispatch(ChromeIntent::RenameTree(
        tree,
        "History title refreshed".into(),
    ));
    wait("preview title updated", || {
        text(&ui, "#forest-preview").contains("History title refreshed")
    })?;
    assert!(
        chrome(
            &ui,
            "heldControls.every((el,i)=>el===$('forest-preview').querySelectorAll('button')[i])"
        )? == true
    );
    held_refresh(&ui, "[data-forest=save]")?;
    wait("Save moves selection", || {
        chrome(&ui, "S.forest.history.preview.id").is_ok_and(|v| v != first)
    })?;
    assert!(ui
        .pages
        .read(|b| b.tree_states())?
        .iter()
        .any(|s| s.root == tree && s.pin.is_some() && !s.lifecycle.is_open()));
    let reopen = chrome(&ui, "S.forest.history.preview.id")?;
    held_refresh(&ui, "[data-forest=reopen]")?;
    wait("Reopen navigates", || {
        !visible(&ui, "#forest")
            && ui.snapshot().is_ok_and(|s| {
                s.current
                    .is_some_and(|id| Some(id.to_string().as_str()) == reopen.as_str())
            })
    })?;
    key("ctrl+e")?;
    wait("Trees again", || visible(&ui, "#forest-trees .tree-card"))?;
    held_refresh(&ui, ".tree-card[aria-current=true]")?;
    assert!(!visible(&ui, "#forest"));
    key("ctrl+h")?;
    wait("History shortcut", || visible(&ui, ".forest-row"))?;
    key("ctrl+h")?;
    assert!(!visible(&ui, "#forest"));
    key("ctrl+e")?;
    wait("Trees shortcut", || {
        visible(&ui, "#forest-trees .tree-card")
    })?;
    key("ctrl+h")?;
    wait("switch shortcut", || visible(&ui, ".forest-row"))?;
    key("ctrl+e")?;
    assert!(!visible(&ui, "#forest"));
    key("ctrl+e")?;
    wait("current card menu", || {
        visible(&ui, ".tree-card[aria-current=true]")
    })?;
    chrome(
        &ui,
        "(document.querySelector('.tree-card[aria-current=true]').focus(),true)",
    )?;
    key("shift+F10")?;
    click_chrome(&ui, "[data-tab-action=settle]")?;
    pump(700);
    assert!(visible(&ui, "#forest"));
    assert_eq!(
        chrome(&ui, "S.forest.count")?,
        serde_json::json!("4 open · 3 saved · 125 in history")
    );
    click_chrome(&ui, "#forest-views [data-view=history]")?;
    wait("outline target", || visible(&ui, ".pv-row"))?;
    let page = chrome(&ui, "document.querySelector('.pv-row').dataset.id")?;
    held_refresh(&ui, ".pv-row")?;
    wait("held outline opens its page", || {
        !visible(&ui, "#forest")
            && ui.snapshot().is_ok_and(|s| {
                s.current
                    .is_some_and(|id| Some(id.to_string().as_str()) == page.as_str())
            })
    })?;
    key("ctrl+e")?;
    wait("forest for focus check", || visible(&ui, "#forest"))?;
    println!("PASS forest preview, row keys, older rows, Find/Escape, real settled Rename, retained controls, Save stays settled, Reopen, card activation, E/H shortcuts and Settle keeps the forest open.");
    key("Tab")?;
    assert!(chrome(&ui, "!!document.activeElement.closest('#forest')")? == true);
    key("Escape")?;
    wait("closed", || !visible(&ui, "#forest"))?;
    let current = ui.snapshot()?.current.unwrap();
    for state in ui.pages.read(|b| b.tree_states())? {
        if state.lifecycle.is_open() && state.root.root() != current {
            ui.pages.settle_tree(state.root)?;
        }
    }
    pump(300);
    key("ctrl+e")?;
    wait("last open card", || {
        visible(&ui, ".tree-card[aria-current=true]")
    })?;
    key("Menu")?;
    click_chrome(&ui, "[data-tab-action=settle]")?;
    wait("last tree settled with forest open", || {
        ui.snapshot().is_ok_and(|s| s.current.is_none()) && visible(&ui, "#forest")
    })?;
    assert!(chrome(&ui, "!!document.activeElement.closest('#forest')")? == true);
    key("Escape")?;
    println!("PASS settling the last open tree preserves the forest and its keyboard focus.");
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(100);
    }
    println!("PASS forest toolbar totals, ordered Open/Saved bands, bounded cards, width, history membership and focus containment.");
    ui.window.close();
    Ok(())
}
