use anyhow::Result;
use canopy::{
    browser::{Browser, Move, Place, Slot, ThemeMode},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;
mod common;
use common::*;
fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let dir = tempfile::tempdir()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            let body = "<!doctype html><title>Place fixture</title>Page";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let path = dir.path().join("forest.sqlite3");
    let mut b = Browser::open(&path)?;
    let ids = b.add_roots(
        &(0..5)
            .map(|i| url::Url::parse(&format!("http://fixture.localhost:{port}/{i}")).unwrap())
            .collect::<Vec<_>>(),
    )?;
    let states = b.tree_states()?;
    let trees = states.iter().map(|s| s.root).collect::<Vec<_>>();
    for (i, t) in trees.iter().enumerate() {
        b.rename_tree(*t, &format!("Place {i}"))?;
    }
    b.place_tree(trees[3], Move::History)?;
    b.place_tree(trees[4], Move::History)?;
    b.place_tree(trees[2], Move::Tabs(Slot::First))?;
    drop(b);
    let b = Browser::open(&path)?;
    assert_eq!(b.tree_states()?[0].root, trees[2]);
    drop(b);
    let file = dir.path().join("kept.pdf");
    std::fs::write(&file, b"download stays")?;
    let c = rusqlite::Connection::open(&path)?;
    c.execute("INSERT INTO captures(visit_id,document_url,title,body,captured_at,kind) VALUES(?1,'https://fixture.test','Delete capture','erasedcontent',1,'page')",[ids[4].to_string()])?;
    c.execute(
        "INSERT INTO downloads(visit_id,path,size,saved_at) VALUES(?1,?2,14,1)",
        rusqlite::params![ids[4].to_string(), file.to_str()],
    )?;
    c.execute("INSERT INTO bookmarks(url,title,position,added_at) VALUES('https://fixture.test','Keep bookmark',0,1)",[])?;
    drop(c);
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 800);
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+1")?;
    wait("numbered shortcut follows stored order", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(ids[2]))
    })?;
    let tab = format!(".tab[data-id='{}']", ids[2]);
    hover_chrome(&ui, &tab)?;
    xdo(&["click", "3"])?;
    assert_eq!(
        text(&ui, "#popover"),
        "Pin tab\nSave for later\nRename\nSettle tree"
    );
    click_chrome(&ui, "[data-tab-action=pin]")?;
    // Three trees in a wide window: the pin keeps its name and shows the pin mark.
    wait("pinned and named", || {
        visible(
            &ui,
            &format!("{tab}.pinned:not(.icon-only) .tab-pinned-mark"),
        )
    })?;
    // Short of room, the pin folds to its icon.
    ui.window.resize(560, 800);
    wait("pinned icon", || {
        visible(&ui, &format!("{tab}.pinned.icon-only"))
    })?;
    ui.window.resize(1240, 800);
    wait("pinned named again", || {
        visible(&ui, &format!("{tab}.pinned:not(.icon-only)"))
    })?;
    hover_chrome(&ui, &tab)?;
    xdo(&["click", "3"])?;
    click_chrome(&ui, "[data-tab-action=unpin]")?;
    wait("unpinned", || !visible(&ui, &format!("{tab}.pinned")))?;
    hover_chrome(&ui, &tab)?;
    xdo(&["click", "3"])?;
    click_chrome(&ui, "[data-tab-action=pin]")?;
    wait("pinned again", || visible(&ui, &format!("{tab}.pinned")))?;
    assert_eq!(
        chrome(&ui, &format!("document.querySelector(\"{tab}\").draggable"))?,
        true
    );
    std::fs::create_dir_all(".cache/shots/tree-places")?;
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(theme));
        pump(200);
        let theme = if theme == ThemeMode::Dark {
            "dark"
        } else {
            "light"
        };
        save_window(&ui, &format!(".cache/shots/tree-places/tabs-{theme}.png"))?;
        hover_chrome(&ui, &tab)?;
        xdo(&["click", "3"])?;
        save_window(
            &ui,
            &format!(".cache/shots/tree-places/tab-menu-{theme}.png"),
        )?;
        key("Escape")?;
    }
    hover_chrome(&ui, &tab)?;
    xdo(&["click", "3"])?;
    click_chrome(&ui, "[data-tab-action=save]")?;
    wait("Save retires tab and lands", || {
        !visible(&ui, &tab) && ui.snapshot().is_ok_and(|s| s.current != Some(ids[2]))
    })?;
    assert!(ui
        .pages
        .read(|b| b.tree_states())?
        .iter()
        .any(|s| s.root == trees[2] && matches!(s.place(), Place::Shelf { .. })));
    assert!(toast_text(&ui).is_some_and(|t| t.contains("Saved ‘Place 2’ for later")));
    key("ctrl+t")?;
    wait("shelf on start", || visible(&ui, "[data-start=tree]"))?;
    assert!(text(&ui, "#start-content").contains("Saved for later"));
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(theme));
        pump(200);
        save_window(
            &ui,
            &format!(
                ".cache/shots/tree-places/shelf-{}.png",
                if theme == ThemeMode::Dark {
                    "dark"
                } else {
                    "light"
                }
            ),
        )?;
    }
    click_chrome(&ui, &format!("[data-start=tree][data-id='{}']", ids[2]))?;
    wait("shelf opens and leaves shelf", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(ids[2])) && visible(&ui, &tab)
    })?;
    assert_eq!(
        ui.pages
            .read(|b| b.tree_states())?
            .iter()
            .filter(|s| matches!(s.place(), Place::Shelf { .. }))
            .count(),
        0
    );
    hover_chrome(&ui, &tab)?;
    xdo(&["click", "3"])?;
    click_chrome(&ui, "[data-tab-action=pin]")?;
    key("ctrl+w")?;
    wait("Ctrl W settles pinned tab", || !visible(&ui, &tab))?;
    key("ctrl+h")?;
    wait("History", || visible(&ui, ".forest-row"))?;
    let row = format!(".forest-row[data-id='{}']", ids[4]);
    hover_chrome(&ui, &row)?;
    xdo(&["click", "3"])?;
    click_chrome(&ui, "[data-tab-action=delete]")?;
    wait("delete row and feedback", || {
        !visible(&ui, &row) && visible(&ui, "[data-delete-id]")
    })?;
    assert!(ui.pages.read(|b| b.search("erasedcontent"))?.is_empty());
    ui.pages.go(canopy::pages::Go::Visit(ids[4]))?;
    assert!(ui.pages.read(|b| b.downloads())?.is_empty());
    assert!(file.exists());
    assert_eq!(ui.pages.read(|b| b.bookmarks())?.len(), 1);
    click_chrome(&ui, "[data-delete-id]")?;
    wait("Undo restores row", || visible(&ui, &row))?;
    assert!(!ui.pages.read(|b| b.search("erasedcontent"))?.is_empty());
    assert_eq!(ui.pages.read(|b| b.downloads())?.len(), 1);
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(theme));
        pump(200);
        let theme = if theme == ThemeMode::Dark {
            "dark"
        } else {
            "light"
        };
        save_window(
            &ui,
            &format!(".cache/shots/tree-places/history-{theme}.png"),
        )?;
        click_chrome(&ui, "#forest-views [data-view=canopy]")?;
        pump(150);
        save_window(&ui, &format!(".cache/shots/tree-places/trees-{theme}.png"))?;
        click_chrome(&ui, "#forest-views [data-view=history]")?;
        pump(150);
    }
    key("Escape")?;
    ui.pages.go(canopy::pages::Go::Tree(trees[3]))?;
    settle(&ui)?;
    ui.pages.selected_page().navigate(
        url::Url::parse(&format!("http://fixture.localhost:{port}/checkpoint-child"))?,
        canopy::browser::NavigationTarget::ChildOf(ids[3]),
    )?;
    wait("checkpoint child", || {
        ui.snapshot().is_ok_and(|s| {
            s.current_visit()
                .is_some_and(|v| v.url.path() == "/checkpoint-child")
        }) && !ui.pages.selected_page().native.is_loading()
    })?;
    let child = ui.snapshot()?.current.unwrap();
    assert!(ui.pages.selected_history_visits().contains(&ids[3]));
    ui.pages.place_tree(trees[3], Move::History)?;
    wait("settle landing leaves checkpoint tree", || {
        ui.snapshot().is_ok_and(|s| s.current != Some(child))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    pump(1000);
    key("ctrl+h")?;
    wait("checkpoint tree in History", || visible(&ui, ".forest-row"))?;
    let row = format!(".forest-row[data-id='{}']", ids[3]);
    hover_chrome(&ui, &row)?;
    xdo(&["click", "3"])?;
    click_chrome(&ui, "[data-tab-action=delete]")?;
    wait("checkpoint tree deleted", || {
        !visible(&ui, &row) && visible(&ui, "[data-delete-id]")
    })?;
    click_chrome(&ui, "[data-delete-id]")?;
    wait("checkpoint tree restored", || visible(&ui, &row))?;
    hover_chrome(&ui, &row)?;
    xdo(&["click", "3"])?;
    click_chrome(&ui, "[data-tab-action=open]")?;
    wait("restored tree opens cold at its last page", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(child))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert_eq!(ui.pages.selected_history_visits(), vec![child]);
    println!(
        "PASS Delete drops the settled tree’s native checkpoint; Undo reopens its last page cold."
    );
    println!("PASS stored order and Ctrl+1, tab Pin/Unpin, Save with landing and info, shelf Open, pinned Ctrl+W, History Delete/Undo, capture removal/restoration, bookmark and download file retention, dark/light chrome.");
    ui.window.close();
    Ok(())
}
