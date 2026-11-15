use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{NavigationTarget, SourceContext, ThemeMode, TreeId},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
use webkit2gtk::WebViewExt;
mod common;
use common::*;

fn tree(ui: &Ui, root: canopy::browser::VisitId) -> TreeId {
    ui.pages
        .read(|b| b.tree_states())
        .unwrap()
        .into_iter()
        .find(|t| t.root.root() == root)
        .unwrap()
        .root
}

fn menu(ui: &Ui, selector: &str) -> Result<()> {
    hover_chrome(ui, selector)?;
    xdo(&["click", "3"])?;
    wait("context menu", || visible(ui, "#popover"))
}

fn start_page(ui: &Ui) -> Result<()> {
    wait("landing loaded", || {
        !ui.pages.selected_page().native.is_loading()
    })?;
    pump(200);
    settle(ui)?;
    key("ctrl+t")?;
    wait("start page focused", || chrome_focused(ui, "#start-search"))
}

fn shots(ui: &Ui, name: &str) -> Result<()> {
    if std::env::var_os("CANOPY_NEWTREE_SHOTS").is_some() {
        move_to(5, 590)?;
        pump(500);
        std::fs::create_dir_all(".cache/shots")?;
        save_window(ui, &format!(".cache/shots/newtree-{name}.png"))?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = [0; 4096];
            let Ok(n) = stream.read(&mut request) else {
                continue;
            };
            let request = String::from_utf8_lossy(&request[..n]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let body =
                format!("<!doctype html><title>Reading {path}</title><body>New tree fixture");
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.resize(900, 600);
    ui.window.show_all();
    ready(&ui)?;
    wait("empty page", || visible(&ui, "#start-search"))?;
    assert!(text(&ui, "#start-content")
        .contains("Pin a tree from its tab menu. It stays here and never settles on its own."));
    assert!(visible(&ui, "#strip-placeholder"));
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(200);
        shots(
            &ui,
            if mode == ThemeMode::Dark {
                "empty-dark"
            } else {
                "empty-light"
            },
        )?;
    }
    let mut roots = Vec::new();
    for name in ["Reading", "Recipes", "Weekend"] {
        ui.pages.selected_page().navigate(
            web_url(&format!("http://{address}/{name}"))?,
            NavigationTarget::Root,
        )?;
        wait("root loaded", || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.title == format!("Reading /{name}"))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        let root = ui.snapshot()?.current.unwrap();
        roots.push(root);
        ui.dispatch(ChromeIntent::RenameTree(tree(&ui, root), name.into()));
        ui.dispatch(ChromeIntent::PinTree {
            tree: tree(&ui, root),
            pinned: true,
        });
    }
    let root = roots[1];
    ui.pages.selected_page().navigate(
        web_url(&format!("http://{address}/recipes-child"))?,
        NavigationTarget::ChildOf(root),
    )?;
    wait("child loaded", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.title == "Reading /recipes-child")
            && !ui.pages.selected_page().native.is_loading()
    })?;
    let last = ui.snapshot()?.current.unwrap();
    ui.pages.read(|b| b.snapshot())?;
    ui.dispatch(ChromeIntent::SettleTree(tree(&ui, root)));
    wait("settled", || {
        ui.pages
            .read(|b| b.tree_states())
            .unwrap()
            .iter()
            .any(|t| t.root == tree(&ui, root) && !t.lifecycle.is_open() && t.pin.is_some())
    })?;
    let source = SourceContext {
        page_url: web_url(&format!("http://{address}/Reading"))?,
        citation_url: None,
        link_text: "Waiting page".into(),
        excerpt: String::new(),
    };
    canopy::browser::Browser::open(&directory.path().join("forest.sqlite3"))?.queue_link(
        roots[0],
        web_url(&format!("http://{address}/waiting"))?,
        source,
    )?;
    start_page(&ui)?;
    let expected: Vec<_> = roots.iter().map(ToString::to_string).collect();
    assert_eq!(
        chrome(&ui, "canopy.state().start.pinned.map(t=>t.id)")?,
        serde_json::json!(expected)
    );
    assert!(text(&ui, ".band-head").contains("Pinned trees"));
    assert_eq!(
        chrome(
            &ui,
            "new Set([...document.querySelectorAll('.pinned-card')].map(e=>e.offsetTop)).size"
        )?,
        1
    );
    assert!(text(&ui, ".pinned-card .count").contains('1'));
    assert!(text(&ui, ".pinned-card .pc-meta").contains("pages"));
    assert_eq!(
        chrome(
            &ui,
            "document.querySelectorAll('.pinned-card .last').length"
        )?,
        3
    );
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(200);
        shots(
            &ui,
            if mode == ThemeMode::Dark {
                "three-dark"
            } else {
                "three-light"
            },
        )?;
    }
    println!("PASS empty page, placeholder, headings, pin order, three columns, waiting count and last-page rings.");
    click_chrome(&ui, &format!(".pinned-card[data-id='{root}']"))?;
    wait("settled card reopens last page", || {
        ui.snapshot().unwrap().current == Some(last)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    wait("reopened tab", || {
        visible(&ui, &format!(".tab[data-id='{root}']"))
    })?;
    assert!(!visible(&ui, "#strip-placeholder"));
    start_page(&ui)?;
    type_text("q")?;
    wait("typed palette", || visible(&ui, "#palette"))?;
    assert_eq!(
        chrome(&ui, "document.getElementById('palette-input').value")?,
        "q"
    );
    type_text("uokka")?;
    assert_eq!(
        chrome(&ui, "document.getElementById('palette-input').value")?,
        "quokka"
    );
    assert!(text(&ui, "#palette-context").contains("New tree"));
    key("Escape")?;
    click_chrome(&ui, "#start-search")?;
    wait("search click palette", || visible(&ui, "#palette"))?;
    key("Escape")?;
    key("ctrl+k")?;
    wait("Ctrl K palette", || visible(&ui, "#palette"))?;
    key("Escape")?;
    start_page(&ui)?;
    key("ctrl+alt+x")?;
    key("ctrl+x")?;
    assert!(!visible(&ui, "#palette"));
    key("Tab")?;
    assert!(chrome_focused(&ui, ".pinned-card"));
    key("space")?;
    wait("Space activates card", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|s| s == "reading")
    })?;
    start_page(&ui)?;
    key("Tab")?;
    key("Return")?;
    wait("Enter activates card", || {
        chrome(&ui, "canopy.state().surface").is_ok_and(|s| s == "reading")
    })?;
    println!("PASS settled card restores last page, typing appends in new-tree scope, search, Ctrl+K, modifiers, Tab, Space and Enter.");
    let tab = format!(".tab[data-id='{}']", roots[0]);
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(200);
        menu(&ui, &tab)?;
        assert_eq!(text(&ui, "#popover"), "Rename\nUnpin tree\nSettle tree");
        assert_eq!(chrome(&ui, "[...document.querySelectorAll('[data-tab-action]')].every(e=>e.getBoundingClientRect().bottom <= document.getElementById('popover').getBoundingClientRect().bottom)")?, true);
        shots(
            &ui,
            if mode == ThemeMode::Dark {
                "menu-dark"
            } else {
                "menu-light"
            },
        )?;
        type_text("z")?;
        assert!(!visible(&ui, "#palette"));
        key("Escape")?;
    }
    menu(&ui, &tab)?;
    click_chrome(&ui, "[data-tab-action=rename]")?;
    wait("rename focused", || chrome_focused(&ui, "#tree-name"))?;
    type_text("Reading renamed")?;
    key("Return")?;
    wait("renamed", || text(&ui, &tab).contains("Reading renamed"))?;
    menu(&ui, &tab)?;
    click_chrome(&ui, "[data-tab-action=pin]")?;
    wait("unpinned", || {
        !visible(&ui, &format!("{tab} .tab-pinned-mark"))
    })?;
    menu(&ui, &tab)?;
    assert!(text(&ui, "#popover").contains("Pin tree"));
    click_chrome(&ui, "[data-tab-action=pin]")?;
    wait("pin mark", || {
        visible(&ui, &format!("{tab} .tab-pinned-mark"))
    })?;
    menu(&ui, &tab)?;
    click_chrome(&ui, "[data-tab-action=settle]")?;
    wait("settled from menu", || !visible(&ui, &tab))?;
    start_page(&ui)?;
    assert_eq!(chrome(&ui, "canopy.state().start.pinned.length")?, 3);
    menu(&ui, ".pinned-card")?;
    assert_eq!(text(&ui, "#popover"), "Unpin tree");
    click_chrome(&ui, "[data-tab-action=pin]")?;
    wait("card removed", || {
        chrome(&ui, "canopy.state().start.pinned.length").is_ok_and(|v| v == 2)
    })?;
    println!(
        "PASS tab menu, rename, pin mark, unpin, re-pin, settle keeps pin, card context menu."
    );
    let count = ui.snapshot()?.visits.len();
    click_chrome(&ui, "[data-start=shortcut]")?;
    wait("frequent tile plants root", || {
        ui.snapshot().unwrap().visits.len() == count + 1
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    {
        let mut browser = canopy::browser::Browser::open(&directory.path().join("forest.sqlite3"))?;
        let urls = (0..12)
            .map(|i| web_url(&format!("http://{address}/extra-{i}")))
            .collect::<Result<Vec<_>>>()?;
        let extra = browser.add_roots(&urls)?;
        for id in extra {
            let tree = browser
                .tree_states()?
                .into_iter()
                .find(|t| t.root.root() == id)
                .unwrap()
                .root;
            browser.set_pinned(tree, true)?;
        }
    }
    ui.dispatch(ChromeIntent::SetTheme(ThemeMode::Dark));
    start_page(&ui)?;
    wait("extra cards", || {
        chrome(&ui, "canopy.state().start.pinned.length").is_ok_and(|v| v == 14)
    })?;
    assert_eq!(chrome(&ui, "document.getElementById('start').scrollHeight > document.getElementById('start').clientHeight")?, true);
    chrome(&ui, "document.getElementById('start').scrollTop=10000")?;
    pump(100);
    assert_eq!(chrome(&ui, "document.querySelector('#start-content section:last-child').getBoundingClientRect().bottom <= innerHeight")?, true);
    shots(&ui, "scroll-dark")?;
    println!("PASS 900 by 600 scrolls and the final band remains reachable.");
    let trees = ui.pages.read(|b| b.tree_states())?;
    for tree in trees {
        ui.dispatch(ChromeIntent::SettleTree(tree.root));
        pump(150);
    }
    wait("placeholder with settled pins", || {
        visible(&ui, "#strip-placeholder")
    })?;
    chrome(&ui, "document.getElementById('start').scrollTop=0")?;
    pump(100);
    click_chrome(&ui, ".pinned-card")?;
    wait("placeholder leaves", || !visible(&ui, "#strip-placeholder"))?;
    println!("PASS frequent tile plants and all-settled placeholder clears on card click.");
    ui.window.close();
    Ok(())
}
