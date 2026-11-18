use anyhow::Result;
use canopy::{
    browser::{Browser, Move, Place, Slot, TreeId},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
use common::*;

fn card(tree: TreeId) -> String {
    format!("#forest-trees [data-id='{}']", tree.root())
}
fn main_button(tree: TreeId) -> String {
    format!("{} [data-main]", card(tree))
}
fn place(ui: &Ui, tree: TreeId) -> Place {
    ui.pages
        .read(|b| b.tree_states())
        .unwrap()
        .into_iter()
        .find(|s| s.root == tree)
        .unwrap()
        .place()
}
fn order(ui: &Ui, shelf: bool) -> Vec<TreeId> {
    ui.pages
        .read(|b| b.tree_states())
        .unwrap()
        .into_iter()
        .filter(|s| {
            if shelf {
                matches!(s.place(), Place::Shelf { .. })
            } else {
                s.place().is_tab()
            }
        })
        .map(|s| s.root)
        .collect()
}
fn focus(ui: &Ui, tree: TreeId) -> Result<()> {
    chrome(
        ui,
        &format!("document.querySelector({:?}).focus()", main_button(tree)),
    )?;
    pump(100);
    Ok(())
}
fn reveal(ui: &Ui, tree: TreeId) -> Result<()> {
    chrome(
        ui,
        &format!(
            "document.querySelector({:?}).scrollIntoView({{block:'nearest'}})",
            card(tree)
        ),
    )?;
    pump(100);
    Ok(())
}
fn target(ui: &Ui, lane: &str, before: Option<TreeId>) -> Result<(i32, i32)> {
    if let Some(tree) = before {
        reveal(ui, tree)?;
        let selector = card(tree);
        let (x, y, w, _) = rect(ui, &selector)?;
        return screen_point(
            ui,
            &ui.chrome.native(),
            (x + w / 2.) as i32,
            (y + 2.) as i32,
        );
    }
    let selector = format!("[data-lane='{lane}'] .lane-cards");
    let (x, y, w, h) = rect(ui, &selector)?;
    screen_point(
        ui,
        &ui.chrome.native(),
        (x + w / 2.) as i32,
        (y + h - 6.) as i32,
    )
}
fn drop_card(ui: &Ui, tree: TreeId, lane: &str, before: Option<TreeId>) -> Result<()> {
    reveal(ui, tree)?;
    let end = target(ui, lane, before)?;
    drag(ui, &main_button(tree), end, || {
        if before.is_none() && lane != "settled" {
            for step in 0..60 {
                move_to(end.0 + step % 2, end.1)?;
                pump(60);
            }
        }
        assert!(
            chrome(
                ui,
                &format!("!!document.querySelector('[data-lane={lane}].drop-here')")
            )? == true
        );
        assert!(
            chrome(
                ui,
                "!!document.querySelector('#forest-trees .drop-before,#forest-trees .drop-end')"
            )? == true
        );
        assert!(chrome(ui, "!!document.querySelector('#forest-trees .dragging')")? == true);
        Ok(())
    })?;
    assert!(visible(ui, "#forest"));
    assert!(
        chrome(
            ui,
            "!document.querySelector('#forest-trees .drop-here,#forest-trees .dragging')"
        )? == true
    );
    Ok(())
}
fn action(ui: &Ui, tree: TreeId, act: &str) -> Result<()> {
    reveal(ui, tree)?;
    click_chrome(ui, &format!("{} [data-act='{act}']", card(tree)))?;
    pump(150);
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
            let body = "<!doctype html><title>Lane fixture</title>Local page";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let database = dir.path().join("forest.sqlite3");
    if let Some(fixture) = std::env::var_os("CANOPY_FOREST_TREES_FIXTURE") {
        std::fs::copy(fixture, &database)?;
    }
    let mut b = Browser::open(&database)?;
    let existing = b.tree_states()?;
    let extra_today = existing
        .iter()
        .filter(|s| !matches!(s.place(), Place::History { .. }))
        .count();
    for s in existing
        .iter()
        .filter(|s| !matches!(s.place(), Place::History { .. }))
    {
        b.place_tree(s.root, Move::History)?;
    }
    let trees = b
        .add_roots(
            &(0..23)
                .map(|i| url::Url::parse(&format!("http://fixture.localhost:{port}/{i}")).unwrap())
                .collect::<Vec<_>>(),
        )?
        .into_iter()
        .map(|id| {
            b.tree_states()
                .unwrap()
                .into_iter()
                .find(|s| s.root.root() == id)
                .unwrap()
                .root
        })
        .collect::<Vec<_>>();
    for (i, t) in trees.iter().enumerate() {
        b.rename_tree(*t, &format!("Project {i}"))?;
        match i {
            0..=1 => {
                b.place_tree(*t, Move::Pin(true))?;
            }
            8..=10 => {
                b.place_tree(*t, Move::Shelf(Slot::End))?;
            }
            6..=7 | 11.. => {
                b.place_tree(*t, Move::History)?;
            }
            _ => {}
        }
    }
    drop(b);
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 800);
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+e")?;
    wait("three lanes", || visible(&ui, "[data-lane=settled]"))?;
    assert_eq!(
        text(&ui, "#forest-count"),
        format!(
            "6 tabs · 3 saved for later · {} in history",
            14 + existing.len()
        )
    );
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('.forest-lane')].map(l=>l.dataset.lane)"
        )?,
        serde_json::json!(["tabs", "shelf", "settled"])
    );
    assert_eq!(
        chrome(
            &ui,
            "document.querySelectorAll('[data-lane=settled] .tree-card').length"
        )?,
        14 + extra_today
    );
    assert_eq!(
        chrome(
            &ui,
            "document.querySelectorAll('[data-lane=tabs] .line').length"
        )?,
        2
    );
    assert_eq!(chrome(&ui,"[...document.querySelectorAll('[data-lane=tabs] .tree-card')].every(c=>c.querySelectorAll('[data-act]').length===3)")?,true);
    assert!(!visible(&ui, "#forest-find"));
    type_text("nothing")?;
    assert_eq!(chrome(&ui, "S.forest.query")?, "");
    for width in [1920, 728, 390] {
        ui.window.resize(width, 800);
        pump(200);
        assert_eq!(chrome(&ui,"(()=>{const r=$('forest').querySelector('.forest-pop').getBoundingClientRect();return [r.left,innerWidth-r.right]})()")?,serde_json::json!([22,22]));
        assert!(chrome(&ui,"[...document.querySelectorAll('.lane-cards')].every(e=>getComputedStyle(e).overflowY==='auto')")?==true);
        if width == 390 {
            assert!(
                chrome(
                    &ui,
                    "$('forest-trees').scrollWidth>$('forest-trees').clientWidth"
                )? == true
            );
            assert_eq!(rect(&ui, ".forest-lane")?.2, 300.);
        }
    }
    ui.window.resize(1240, 1000);
    pump(200);
    println!("PASS three counted lanes, all {} settled today, pinned lines, no Find and responsive independent scrolling over {} trees.",14+extra_today,23+existing.len());

    let t = &trees;
    click_chrome(&ui, &main_button(t[0]))?;
    wait("loaded page before lane moves", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(t[0].root()))
    })?;
    key("ctrl+e")?;
    wait("Trees over a loaded page", || {
        visible(&ui, "[data-lane=tabs]")
    })?;
    assert_eq!(
        chrome(
            &ui,
            &format!(
                "document.querySelector({:?}).getAttribute('aria-current')",
                card(t[0])
            )
        )?,
        "true"
    );
    drop_card(&ui, t[3], "tabs", Some(t[2]))?;
    assert!(
        order(&ui, false).iter().position(|x| *x == t[3])
            < order(&ui, false).iter().position(|x| *x == t[2])
    );
    drop_card(&ui, t[3], "tabs", Some(t[1]))?;
    assert!(place(&ui, t[3]).pinned());
    drop_card(&ui, t[3], "tabs", Some(t[2]))?;
    assert!(place(&ui, t[3]).pinned());
    drop_card(&ui, t[3], "tabs", None)?;
    assert!(!place(&ui, t[3]).pinned());
    drop_card(&ui, t[3], "tabs", Some(t[2]))?;
    assert!(!place(&ui, t[3]).pinned());
    drop_card(&ui, t[10], "shelf", Some(t[8]))?;
    assert_eq!(order(&ui, true)[0], t[10]);
    drop_card(&ui, t[4], "shelf", Some(t[8]))?;
    assert_eq!(order(&ui, true)[1], t[4]);
    drop_card(&ui, t[5], "settled", None)?;
    assert!(matches!(place(&ui, t[5]), Place::History { .. }));
    drop_card(&ui, t[9], "settled", None)?;
    assert!(matches!(place(&ui, t[9]), Place::History { .. }));
    let current = ui.pages.read(|b| b.snapshot())?.current;
    assert_eq!(current, Some(t[0].root()));
    drop_card(&ui, t[8], "tabs", Some(t[2]))?;
    assert!(place(&ui, t[8]).is_tab());
    assert_eq!(ui.pages.read(|b| b.snapshot())?.current, current);
    drop_card(&ui, t[11], "tabs", None)?;
    assert_eq!(order(&ui, false).last(), Some(&t[11]));
    drop_card(&ui, t[12], "shelf", None)?;
    assert_eq!(order(&ui, true).last(), Some(&t[12]));
    println!("PASS real drags reorder both lanes, pin among pins, unpin among tabs, keep pin at the border and move every pair of lanes without navigation.");

    let before = ui.pages.read(|b| b.tree_states())?;
    reveal(&ui, t[2])?;
    let outside = screen_point(&ui, &ui.chrome.native(), 10, 10)?;
    drag(&ui, &main_button(t[2]), outside, || {
        assert_eq!(
            chrome(&ui, "!!document.querySelector('.drop-here')")?,
            false
        );
        Ok(())
    })?;
    assert_eq!(
        format!("{before:?}"),
        format!("{:?}", ui.pages.read(|b| b.tree_states())?)
    );
    let end = target(&ui, "shelf", Some(t[10]))?;
    reveal(&ui, t[2])?;
    drag(&ui, &main_button(t[2]), end, || {
        ui.dispatch(ChromeIntent::RenameTree(t[0], "Refresh during drag".into()));
        wait("held forest paint", || {
            chrome(&ui, "canopy.state().deferred.includes('forest')").is_ok_and(|v| v == true)
        })?;
        Ok(())
    })?;
    assert_eq!(order(&ui, true)[0], t[2]);
    wait("deferred rename painted", || {
        text(&ui, "#forest-trees").contains("Refresh during drag")
    })?;
    assert_eq!(chrome(&ui, "canopy.state().buttons")?, 0);
    println!("PASS release outside changes nothing and native drag survives a held refresh and releases the press guard.");

    let bar_end = chrome_point(&ui, &format!(".tab[data-id='{}']", t[1].root()))?;
    reveal(&ui, t[12])?;
    drag(&ui, &main_button(t[12]), bar_end, || {
        assert_eq!(
            chrome(&ui, "$('tab-row').classList.contains('drop-target')")?,
            true
        );
        Ok(())
    })?;
    assert!(place(&ui, t[12]).is_tab());
    let end = target(&ui, "shelf", None)?;
    drag(
        &ui,
        &format!(".tab[data-id='{}'] .mk", t[12].root()),
        end,
        || {
            assert_eq!(
                chrome(
                    &ui,
                    "!!document.querySelector('[data-lane=shelf].drop-here')"
                )?,
                true
            );
            Ok(())
        },
    )?;
    assert!(matches!(place(&ui, t[12]), Place::Shelf { .. }));
    println!("PASS a shelf card travels onto the bar and a tab travels onto the shelf with the shared native payload.");

    focus(&ui, t[0])?;
    key("alt+Down")?;
    pump(200);
    assert_eq!(order(&ui, false)[1], t[0]);
    key("alt+Down")?;
    pump(150);
    assert_eq!(order(&ui, false)[1], t[0]);
    key("alt+Up")?;
    pump(200);
    assert_eq!(order(&ui, false)[0], t[0]);
    assert_eq!(
        chrome(
            &ui,
            "document.activeElement.closest('[data-id]').dataset.id"
        )?,
        t[0].root().to_string()
    );
    key("Right")?;
    assert_eq!(
        chrome(
            &ui,
            "document.activeElement.closest('[data-lane]').dataset.lane"
        )?,
        "shelf"
    );
    key("Down")?;
    key("Up")?;
    key("Left")?;
    assert_eq!(
        chrome(
            &ui,
            "document.activeElement.closest('[data-lane]').dataset.lane"
        )?,
        "tabs"
    );
    action(&ui, t[1], "unpin")?;
    assert!(!place(&ui, t[1]).pinned());
    action(&ui, t[1], "pin")?;
    assert!(place(&ui, t[1]).pinned());
    action(&ui, t[1], "save")?;
    assert!(matches!(place(&ui, t[1]), Place::Shelf { .. }));
    focus(&ui, t[1])?;
    key("alt+Down")?;
    pump(150);
    assert_ne!(order(&ui, true)[0], t[1]);
    action(&ui, t[1], "settle")?;
    assert!(matches!(place(&ui, t[1]), Place::History { .. }));
    focus(&ui, t[1])?;
    key("alt+Return")?;
    pump(250);
    assert!(place(&ui, t[1]).is_tab());
    assert_eq!(ui.pages.read(|b| b.snapshot())?.current, current);
    assert!(visible(&ui, "#forest"));
    for (tree, labels) in [
        (t[0], "Unpin tab|Save for later|Rename|Settle tree"),
        (t[2], "Open|Rename|Settle tree"),
        (t[13], "Reopen|Save for later|Rename|Delete"),
    ] {
        for menu_key in ["Menu", "shift+F10", "right-click"] {
            focus(&ui, tree)?;
            if menu_key == "right-click" {
                reveal(&ui, tree)?;
                let (x, y) = chrome_point(&ui, &main_button(tree))?;
                move_to(x, y)?;
                xdo(&["click", "3"])?;
                pump(100);
            } else {
                key(menu_key)?;
            }
            assert_eq!(chrome(&ui,"[...$('popover').querySelectorAll('[data-tab-action]')].map(e=>e.textContent.trim()).join('|')")?,labels);
            key("Escape")?;
        }
    }
    focus(&ui, t[13])?;
    let (x, y) = chrome_point(&ui, &main_button(t[13]))?;
    move_to(x, y)?;
    xdo(&["click", "3"])?;
    pump(100);
    assert_eq!(chrome(&ui,"[...$('popover').querySelectorAll('[data-tab-action]')].map(e=>e.textContent.trim()).join('|')")?,"Reopen|Save for later|Rename|Delete");
    click_chrome(&ui, "#popover [data-tab-action=save]")?;
    pump(200);
    assert!(matches!(place(&ui, t[13]), Place::Shelf { .. }));
    assert!(visible(&ui, "#forest"));
    for _ in 0..30 {
        key("Tab")?;
        assert_eq!(
            chrome(&ui, "!!document.activeElement.closest('#forest')")?,
            true
        );
    }
    println!("PASS keyboard movement stays inside pin groups, arrows cross lanes, actions and menus mirror drops, Alt+Enter keeps the page and Tab stays inside.");

    focus(&ui, t[0])?;
    key("Menu")?;
    click_chrome(&ui, "#popover [data-tab-action=rename]")?;
    type_text("Renamed through menu")?;
    key("Return")?;
    wait("rename keeps forest open", || {
        text(&ui, "#forest-trees").contains("Renamed through menu")
    })?;
    assert!(visible(&ui, "#forest"));
    reveal(&ui, t[0])?;
    let selector = format!("{} [data-act=save]", card(t[0]));
    let (x, y) = chrome_point(&ui, &selector)?;
    move_to(x, y)?;
    chrome(
        &ui,
        &format!("window.heldCardButton=document.querySelector({selector:?})"),
    )?;
    xdo(&["mousedown", "1"])?;
    wait("nested action pressed", || {
        chrome(&ui, "canopy.state().buttons").is_ok_and(|v| v == 1)
    })?;
    ui.dispatch(ChromeIntent::RenameTree(
        t[3],
        "Refresh during button press".into(),
    ));
    wait("nested action paint held", || {
        chrome(&ui, "canopy.state().deferred.includes('forest')").is_ok_and(|v| v == true)
    })?;
    assert_eq!(
        chrome(
            &ui,
            &format!("heldCardButton===document.querySelector({selector:?})")
        )?,
        true
    );
    xdo(&["mouseup", "1"])?;
    pump(250);
    wait("pressed action saves the tree", || {
        matches!(place(&ui, t[0]), Place::Shelf { .. })
    })?;
    assert!(matches!(place(&ui, t[0]), Place::Shelf { .. }));
    assert!(visible(&ui, "#forest"));
    let before = order(&ui, true);
    reveal(&ui, t[0])?;
    let end = target(&ui, "tabs", None)?;
    drag(&ui, &main_button(t[0]), end, || {
        xdo(&["key", "Escape"])?;
        pump(150);
        assert_eq!(chrome(&ui, "tabDrag")?, serde_json::Value::Null);
        Ok(())
    })?;
    assert_eq!(order(&ui, true), before);
    assert_eq!(
        chrome(
            &ui,
            "!!document.querySelector('#forest-trees .drop-here,#forest-trees .dragging')"
        )?,
        false
    );
    if !visible(&ui, "#forest") {
        key("ctrl+e")?;
    }
    println!("PASS Rename and a nested action survive refresh, and Escape cancels a native drag without a move.");
    for tree in [t[1], t[2], t[14]] {
        reveal(&ui, tree)?;
        click_chrome(&ui, &main_button(tree))?;
        wait("card navigation closes forest", || !visible(&ui, "#forest"))?;
        wait("card navigation selects tree", || {
            ui.pages
                .read(|b| b.snapshot())
                .is_ok_and(|s| s.current == Some(tree.root()))
        })?;
        key("ctrl+e")?;
        wait("Trees again", || visible(&ui, "[data-lane=tabs]"))?;
    }
    focus(&ui, t[13])?;
    key("Return")?;
    wait("Enter navigates", || !visible(&ui, "#forest"))?;
    key("ctrl+e")?;
    click_chrome(&ui, "[data-show-view=history]")?;
    wait("History link", || visible(&ui, "#forest-history"))?;
    let first = order(&ui, false)[0];
    let selector = format!(".tab[data-id='{}']", first.root());
    let (x, y, _, h) = rect(&ui, &selector)?;
    let end = screen_point(
        &ui,
        &ui.chrome.native(),
        (x + 2.) as i32,
        (y + h / 2.) as i32,
    )?;
    drag(
        &ui,
        &format!(".tab[data-id='{}'] .mk", t[13].root()),
        end,
        || Ok(()),
    )?;
    assert_eq!(order(&ui, false)[0], t[13]);
    assert!(visible(&ui, "#forest-history"));
    println!("PASS the bar reorders while History is showing without leaving the forest.");
    key("Escape")?;
    key("ctrl+e")?;
    key("Escape")?;
    assert!(!visible(&ui, "#forest"));
    println!("PASS a press without movement in each lane and Enter navigate; All of History switches views and Escape closes.");
    let expected_tabs = order(&ui, false);
    let expected_shelf = order(&ui, true);
    ui.window.close();
    drop(ui);
    pump(200);
    let ui = Ui::new(dir.path())?;
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+e")?;
    wait("restarted Trees", || visible(&ui, "[data-lane=tabs]"))?;
    let states = ui.pages.read(|b| b.tree_states())?;
    assert_eq!(
        states
            .iter()
            .filter(|s| s.place().is_tab())
            .map(|s| s.root)
            .collect::<Vec<_>>(),
        expected_tabs
    );
    assert_eq!(
        states
            .iter()
            .filter(|s| matches!(s.place(), Place::Shelf { .. }))
            .map(|s| s.root)
            .collect::<Vec<_>>(),
        expected_shelf
    );
    assert_eq!(
        chrome(&ui, "S.forest.trees.lanes[1].cards.map(c=>c.id)")?,
        serde_json::json!(expected_shelf
            .iter()
            .map(|t| t.root().to_string())
            .collect::<Vec<_>>())
    );
    ui.window.close();
    println!(
        "PASS tab and shelf order and reopened place persist when the UI is closed and reopened."
    );
    Ok(())
}
