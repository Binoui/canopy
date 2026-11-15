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
    assert!(text(&ui, "#start-content").contains("Ctrl+D bookmarks the page you are on."));
    assert!(!visible(&ui, ".sg-row"));
    key("ctrl+d")?;
    wait("no current page toast", || {
        toast_text(&ui).is_some_and(|t| t == "No page to bookmark")
    })?;
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
    ctrl_d(&ui)?;
    bookmarks(&ui, directory.path(), &format!("http://{address}"))?;
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
    suggested(&ui, directory.path(), &format!("http://{address}"))?;
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
        chrome(&ui, "canopy.state().start.pinned.length").is_ok_and(|v| v == 15)
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
    println!("PASS all-settled placeholder clears on card click.");
    ui.window.close();
    Ok(())
}

fn ctrl_d(ui: &Ui) -> Result<()> {
    let snapshot = ui.snapshot()?;
    let current = snapshot.current_visit().unwrap();
    for context in ["page", "chrome", "start"] {
        match context {
            "page" => ui.pages.selected_page().native.grab_focus(),
            "chrome" => {
                key("F6")?;
                wait("chrome has keyboard", || ui.chrome.native().has_focus())?;
            }
            _ => start_page(ui)?,
        }
        key("ctrl+d")?;
        wait("Bookmarked toast", || {
            toast_text(ui).is_some_and(|t| t == "Bookmarked")
        })?;
        let saved = ui.pages.read(|b| b.bookmarks())?;
        assert_eq!(saved.len(), 1);
        assert_eq!(
            (&saved[0].url, &saved[0].title),
            (&current.url, &current.title)
        );
        key("ctrl+d")?;
        wait("Bookmark removed toast", || {
            toast_text(ui).is_some_and(|t| t == "Bookmark removed")
        })?;
        assert!(ui.pages.read(|b| b.bookmarks())?.is_empty());
        assert_eq!(ui.snapshot()?.current, snapshot.current);
        assert_eq!(ui.snapshot()?.visits.len(), snapshot.visits.len());
        if context == "start" {
            key("Escape")?;
        }
    }
    println!("PASS real Ctrl+D adds/removes the stored URL and title with page, chrome and start focus without navigation.");
    Ok(())
}

fn bookmarks(ui: &Ui, data: &std::path::Path, base: &str) -> Result<()> {
    use canopy::browser::{Browser, ImportedBookmark};
    let mut browser = Browser::open(&data.join("forest.sqlite3"))?;
    let make = |path: &str, title: &str, folder: &str, visits| -> Result<ImportedBookmark> {
        Ok(ImportedBookmark {
            url: web_url(&format!("{base}/{path}"))?,
            title: title.into(),
            folder: folder.into(),
            visits,
        })
    };
    let mut imported = Vec::new();
    for i in 0..17 {
        imported.push(make(
            &format!("bookmark-{i}"),
            if i == 0 {
                ""
            } else {
                "Bookmark with a very long title that should be cut"
            },
            "Bookmarks bar",
            100 - i,
        )?);
    }
    for folder in ["Work", "Reading", "Weekend"] {
        imported.push(make(
            &format!("folder-{folder}"),
            folder,
            &format!("Bookmarks bar › {folder}"),
            1,
        )?);
    }
    browser.import_pages("test", &[], &imported)?;
    ui.dispatch(ChromeIntent::SetTheme(ThemeMode::Dark));
    start_page(ui)?;
    wait("bookmarks rendered", || {
        chrome(ui, "canopy.state().start.bookmarks.tiles.length").is_ok_and(|v| v == 15)
    })?;
    assert_eq!(
        chrome(
            ui,
            "document.querySelectorAll('[data-start=bookmark]').length"
        )?,
        15
    );
    assert_eq!(chrome(ui, "new Set([...document.querySelectorAll('[data-start=bookmark]')].map(e=>e.offsetTop)).size")?, 2);
    assert!(text(ui, "[data-start=bookmark] .label").contains("127.0.0.1"));
    assert_eq!(text(ui, "[data-start=more] .label"), "2 more");
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(200);
        shots(
            ui,
            if mode == ThemeMode::Dark {
                "bookmarks-dark"
            } else {
                "bookmarks-light"
            },
        )?;
        click_chrome(ui, "[data-start=folder]")?;
        assert_eq!(
            chrome(
                ui,
                "document.querySelectorAll('#popover [data-start=bookmark]').length"
            )?,
            1
        );
        shots(
            ui,
            if mode == ThemeMode::Dark {
                "folder-dark"
            } else {
                "folder-light"
            },
        )?;
        key("Escape")?;
    }
    click_chrome(ui, "[data-start=more]")?;
    assert_eq!(
        chrome(
            ui,
            "document.querySelectorAll('#popover [data-start=bookmark]').length"
        )?,
        2
    );
    key("Escape")?;
    let before_tile = ui.snapshot()?.visits.len();
    click_chrome(ui, "[data-start=bookmark]")?;
    wait("tile bookmark plants", || {
        ui.snapshot().unwrap().visits.len() == before_tile + 1
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    start_page(ui)?;
    let count = ui.snapshot()?.visits.len();
    click_chrome(ui, "[data-start=folder]")?;
    click_chrome(ui, "#popover [data-start=bookmark]")?;
    wait("folder bookmark opens", || {
        ui.snapshot().unwrap().visits.len() == count + 1
            && !ui.pages.selected_page().native.is_loading()
    })?;
    let chosen = ui.snapshot()?.current_visit().unwrap().url.clone();
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    start_page(ui)?;
    click_chrome(ui, "[data-start=folder]")?;
    click_chrome(ui, "#popover [data-start=bookmark]")?;
    wait("existing address still plants", || {
        ui.snapshot().unwrap().visits.len() == count + 2
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url, chosen);
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    start_page(ui)?;
    click_chrome(ui, "[data-start=folder]")?;
    menu(ui, "#popover [data-start=bookmark]")?;
    assert_eq!(text(ui, "#popover"), "Remove bookmark");
    click_chrome(ui, "[data-remove-bookmark]")?;
    wait("folder bookmark removed", || {
        browser.bookmarks().unwrap().len() == 19
    })?;
    menu(ui, "[data-start=bookmark]")?;
    click_chrome(ui, "[data-remove-bookmark]")?;
    wait("tile removed", || browser.bookmarks().unwrap().len() == 18)?;
    key("ctrl+k")?;
    type_text("folder-Weekend")?;
    wait("bookmark palette group", || {
        visible(ui, "#palette [data-kind=bookmark]")
    })?;
    assert_eq!(
        chrome(
            ui,
            "document.querySelectorAll('#palette [data-kind=bookmark]').length"
        )?,
        1
    );
    assert!(text(ui, "#palette-results").contains("Bookmarks"));
    key("Escape")?;
    for i in 0..10 {
        browser.import_pages(
            "test",
            &[],
            &[make(
                &format!("overflow-{i}"),
                "Overflow",
                &format!("Bookmarks bar › Extra folder {i}"),
                0,
            )?],
        )?;
    }
    ui.dispatch(ChromeIntent::SetTheme(ThemeMode::Dark));
    start_page(ui)?;
    wait("folder overflow", || visible(ui, "[data-start=folders]"))?;
    click_chrome(ui, "[data-start=folders]")?;
    assert!(visible(ui, "#popover [data-start=folder]"));
    click_chrome(ui, "#popover [data-start=folder]")?;
    assert!(visible(ui, "#popover [data-start=bookmark]"));
    browser.import_pages(
        "test",
        &[],
        &[make("repaint", "Repaint", "Bookmarks bar", 0)?],
    )?;
    ui.dispatch(ChromeIntent::SetTheme(ThemeMode::Light));
    pump(300);
    assert!(visible(ui, "#popover [data-start=bookmark]"));
    key("Escape")?;
    key("Escape")?;
    println!("PASS bookmark grid, overflow, folder lists, palette, always planting roots, tile and row removal, folder overflow, dark/light shots at 900 by 600.");
    Ok(())
}

fn suggested(ui: &Ui, data: &std::path::Path, base: &str) -> Result<()> {
    key("Escape")?;
    use canopy::browser::{Browser, VisitId};
    use rusqlite::{params, Connection};
    let connection = Connection::open(data.join("forest.sqlite3"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let seed = |url: &str, title: &str, count: usize, parent: Option<VisitId>| -> Result<VisitId> {
        let id: VisitId = ulid::Ulid::new().to_string().parse()?;
        connection.execute("INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?1,?2,?3,?4,?5,?5)",params![id.to_string(),parent.map(|v|v.to_string()),url,title,now])?;
        if parent.is_none() {
            connection.execute(
                "INSERT INTO tree_state(root_id,last_visit_id,settled_at) VALUES(?1,?1,?2)",
                params![id.to_string(), now],
            )?;
        }
        for _ in 0..count {
            connection.execute(
                "INSERT INTO visit_events(visit_id,from_visit_id,kind,at) VALUES(?1,?2,'new',?3)",
                params![id.to_string(), parent.map(|v| v.to_string()), now],
            )?;
        }
        Ok(id)
    };
    let page = seed(&format!("{base}/suggest-page"), "Daily reading", 8, None)?;
    let waiting = seed(&format!("{base}/suggest-tree"), "Dinner plans", 1, None)?;
    for i in 0..5 {
        let child = seed(
            &format!("{base}/tree-{i}"),
            "Dinner detail",
            1,
            Some(waiting),
        )?;
        connection.execute(
            "UPDATE visit_events SET from_visit_id=NULL WHERE visit_id=?1",
            [child.to_string()],
        )?;
    }
    let first = seed(
        &format!("{}/path-first", base.replace("127.0.0.1", "localhost")),
        "First",
        1,
        None,
    )?;
    seed(&format!("{base}/path-second"), "Second", 4, Some(first))?;
    let challenger = seed(&format!("{base}/challenger"), "New arrival", 1, None)?;
    let mut browser = Browser::open(&data.join("forest.sqlite3"))?;
    browser.queue_link(
        waiting,
        web_url(&format!("{base}/suggest-waiting"))?,
        SourceContext {
            page_url: web_url(base)?,
            citation_url: None,
            link_text: "Waiting".into(),
            excerpt: String::new(),
        },
    )?;
    ui.dispatch(ChromeIntent::SetTheme(ThemeMode::Dark));
    start_page(ui)?;
    wait("three suggested kinds", || {
        chrome(ui, "canopy.state().start.suggested.map(s=>s.kind)")
            .is_ok_and(|v| v == serde_json::json!(["page", "tree", "path"]))
    })?;
    let keys = chrome(ui, "canopy.state().start.suggested.map(s=>s.key)")?;
    assert_eq!(
        chrome(
            ui,
            "new Set([...document.querySelectorAll('.sg')].map(e=>e.offsetTop)).size"
        )?,
        1
    );
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(250);
        hover_chrome(ui, ".sg-main")?;
        pump(100);
        assert_eq!(
            chrome(
                ui,
                "getComputedStyle(document.querySelector('.sg-actions')).opacity"
            )?,
            "1"
        );
        if std::env::var_os("CANOPY_NEWTREE_SHOTS").is_some() {
            std::fs::create_dir_all(".cache/shots")?;
            save_window(
                ui,
                if mode == ThemeMode::Dark {
                    ".cache/shots/suggested-dark.png"
                } else {
                    ".cache/shots/suggested-light.png"
                },
            )?;
        }
    }
    key("ctrl+k")?;
    wait("suggestions in palette", || {
        visible(ui, "#palette [data-kind=suggestion]")
    })?;
    assert_eq!(
        chrome(
            ui,
            "canopy.state().rows.filter(r=>r.kind==='suggestion').map(r=>r.title)"
        )?,
        chrome(ui, "canopy.state().start.suggested.map(s=>s.title)")?
    );
    key("Escape")?;
    for _ in 0..20 {
        connection.execute(
            "INSERT INTO visit_events(visit_id,kind,at) VALUES(?1,'return',?2)",
            params![challenger.to_string(), now],
        )?;
    }
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(250);
        assert_eq!(
            chrome(ui, "canopy.state().start.suggested.map(s=>s.key)")?,
            keys
        );
    }
    click_chrome(ui, "[data-start=suggestion]")?;
    wait("painted page opens", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == "/suggest-page")
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    start_page(ui)?;
    wait("arrival reranks next entry", || {
        chrome(ui, "canopy.state().start.suggested[0].title").is_ok_and(|v| v == "New arrival")
    })?;
    let chosen = web_url(&format!("{base}/challenger"))?;
    hover_chrome(ui, ".sg-main")?;
    click_chrome(ui, "[data-start=keep]")?;
    wait("Keep page bookmarks", || {
        browser.bookmarks().unwrap().iter().any(|b| b.url == chosen)
    })?;
    wait("Keep removes without filling", || {
        chrome(ui, "canopy.state().start.suggested.length").is_ok_and(|v| v == 2)
    })?;
    let declined: canopy::suggest::SuggestionKey =
        chrome(ui, "canopy.state().start.suggested[0].key")?
            .as_str()
            .unwrap()
            .parse()?;
    hover_chrome(ui, ".sg-main")?;
    click_chrome(ui, "[data-start=decline]")?;
    wait("cross persists", || {
        browser.declined().unwrap().contains(&declined)
    })?;
    wait("cross removes without filling", || {
        chrome(ui, "canopy.state().start.suggested.length").is_ok_and(|v| v == 1)
    })?;
    key("Escape")?;
    start_page(ui)?;
    wait("tree suggested", || {
        chrome(
            ui,
            "canopy.state().start.suggested.some(s=>s.kind==='tree')",
        )
        .is_ok_and(|v| v == true)
    })?;
    let tree_key = format!("tree {waiting}");
    let selector = format!("[data-start=keep][data-id='{tree_key}']");
    hover_chrome(
        ui,
        &format!("[data-start=suggestion][data-id='{tree_key}']"),
    )?;
    click_chrome(ui, &selector)?;
    wait("Keep tree pins", || {
        browser
            .tree_states()
            .unwrap()
            .iter()
            .any(|t| t.root.root() == waiting && t.pin.is_some())
    })?;
    wait("kept tree leaves", || !visible(ui, &selector))?;
    assert!(!chrome(ui, "canopy.state().start.suggested.map(s=>s.key)")?
        .as_array()
        .unwrap()
        .contains(&serde_json::json!(declined.to_string())));
    let path_key = "path localhost 127.0.0.1";
    click_chrome(
        ui,
        &format!("[data-start=suggestion][data-id='{path_key}']"),
    )?;
    wait("suggested path loads its child", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == "/path-second" && v.parent.is_some())
            && !ui.pages.selected_page().native.is_loading()
    })?;
    let snapshot = ui.snapshot()?;
    let second = snapshot.current_visit().unwrap();
    let root = snapshot
        .visits
        .iter()
        .find(|v| Some(v.id) == second.parent)
        .unwrap();
    assert_eq!(root.url.path(), "/path-first");
    assert!(root.parent.is_none());
    start_page(ui)?;
    wait("path offered again", || {
        visible(
            ui,
            &format!("[data-start=suggestion][data-id='{path_key}']"),
        )
    })?;
    hover_chrome(
        ui,
        &format!("[data-start=suggestion][data-id='{path_key}']"),
    )?;
    click_chrome(ui, &format!("[data-start=keep][data-id='{path_key}']"))?;
    wait("Keep path bookmarks first page", || {
        browser
            .bookmarks()
            .unwrap()
            .iter()
            .any(|b| b.url.path() == "/path-first")
    })?;
    wait("kept path leaves", || {
        !visible(
            ui,
            &format!("[data-start=suggestion][data-id='{path_key}']"),
        )
    })?;
    key("Escape")?;
    ui.dispatch(ChromeIntent::PinTree {
        tree: tree(ui, waiting),
        pinned: false,
    });
    start_page(ui)?;
    wait("settled tree offered again", || {
        visible(
            ui,
            &format!("[data-start=suggestion][data-id='{tree_key}']"),
        )
    })?;
    click_chrome(
        ui,
        &format!("[data-start=suggestion][data-id='{tree_key}']"),
    )?;
    wait("suggested tree restores last page", || {
        ui.snapshot().unwrap().current == Some(waiting)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(browser
        .tree_states()?
        .iter()
        .any(|t| t.root.root() == waiting && t.lifecycle.is_open()));
    ui.dispatch(ChromeIntent::PinTree {
        tree: tree(ui, waiting),
        pinned: true,
    });
    assert!(browser.snapshot()?.visits.iter().any(|v| v.id == page));
    println!("PASS three suggestion kinds, reasons and hover actions, frozen keys across two refreshes, painted-key activation, new-entry ranking, Keep page/tree, persisted cross, path activation and Keep, tree restoration and no backfill.");
    Ok(())
}
