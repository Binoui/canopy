use anyhow::Result;
use canopy::{
    browser::{Browser, VisitId},
    pages::Go,
    ui::Ui,
};
use gtk::prelude::*;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    thread,
};
use webkit2gtk::WebViewExt;
mod common;
use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("[::]:0")?;
    let port = listener.local_addr()?.port();
    let db_path = directory.path().join("forest.sqlite3");
    drop(Browser::open(&db_path)?);
    let db = rusqlite::Connection::open(&db_path)?;
    let file_path = directory.path().join("strip-download.txt");
    std::fs::write(&file_path, "saved")?;
    let mut titles = HashMap::new();
    let mut trees = Vec::new();
    let mut serial = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64
        - 86_400_000;
    for (shape, depth, children) in [
        ("chain", 8, 0),
        ("fan", 3, 12),
        ("trail", 34, 0),
        ("thread", 1, 18),
        ("small", 2, 0),
    ] {
        let mut ids = Vec::new();
        for i in 0..depth + children {
            let id: VisitId = ulid::Ulid::new().to_string().parse()?;
            let parent = if i == 0 {
                None
            } else if shape == "fan" && i == 3 {
                Some(ids[1])
            } else if i < depth {
                ids.last().copied()
            } else {
                Some(ids[0])
            };
            let path = format!("/{shape}/{i}");
            let title = if shape == "chain" && i == 6 {
                "Tokio tutorial · Framing".to_owned()
            } else if shape == "chain" {
                format!("Tokio tutorial · Spawning {i}")
            } else if shape == "fan" && i == 1 {
                "Ask HN · What are you building this month?".to_owned()
            } else if shape == "fan" && i == 2 {
                "Ask HN · What are you building next month?".to_owned()
            } else {
                format!("Tutorial · {shape} page {i}")
            };
            titles.insert(path.clone(), title.clone());
            let host = if shape == "trail" {
                format!("site{}", i % 3)
            } else {
                "docs".into()
            };
            let url = format!("http://{host}.localhost:{port}{path}");
            let waiting = i >= depth && (shape != "thread" || i < depth + 14);
            serial += 1000;
            db.execute("INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?1,?2,?3,?4,?5,?6)", rusqlite::params![id.to_string(), parent.map(|id: VisitId| id.to_string()), url, title, serial, (!waiting).then_some(serial)])?;
            db.execute("INSERT INTO captures(visit_id,document_url,title,body,captured_at,kind) VALUES(?1,?2,?3,?4,?5,'selection')", rusqlite::params![id.to_string(),url,title,"Saved notes about this page. The complete text remains searchable in Memory. This preview wraps across two lines while the named branches remain visible below it.",serial])?;
            if (shape == "chain" && [2, 3].contains(&i)) || (shape == "trail" && i == 2) {
                db.execute(
                    "INSERT INTO downloads(visit_id,path,size,saved_at) VALUES(?1,?2,5,?3)",
                    rusqlite::params![id.to_string(), file_path.to_str().unwrap(), serial],
                )?;
            }
            ids.push(id);
        }
        let current = ids[depth - 1];
        db.execute(
            "INSERT INTO tree_state(root_id,last_visit_id) VALUES(?1,?2)",
            rusqlite::params![ids[0].to_string(), current.to_string()],
        )?;
        trees.push((shape, ids, current));
    }
    db.execute(
        "UPDATE session SET current_id=?1 WHERE singleton=1",
        [trees[0].2.to_string()],
    )?;
    db.execute("INSERT INTO bookmarks(url,title,folder,position,added_at) VALUES('https://example.org','Keyboard bookmark',NULL,0,0)", [])?;
    drop(db);
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0; 4096];
            let Ok(n) = stream.read(&mut request) else {
                continue;
            };
            let text = String::from_utf8_lossy(&request[..n]);
            let path = text.split_whitespace().nth(1).unwrap_or("/");
            let title = titles.get(path).map_or("Fixture", String::as_str);
            let body = format!("<!doctype html><title>{title}</title><body style='font:20px sans-serif;padding:60px;background:#eee;color:#333'><h1>{title}</h1><p>Reading the local fixture.</p>");
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.resize(820, 800);
    ui.window.show_all();
    ready(&ui)?;
    let shots = std::env::var_os("CANOPY_KEYBOARD_SHOTS").map(std::path::PathBuf::from);
    let (_, _, selected) = &trees[1];
    ui.pages.go(Go::Visit(*selected))?;
    wait("fixture selected", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(*selected))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    settle(&ui)?;
    for theme in ["dark", "light"] {
        chrome(&ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        pump(150);
        key("F6")?;
        check(&ui, "document.activeElement.matches('#strip-nodes [aria-current=page]') && !document.getElementById('strip-list').hidden")?;
        check(&ui, "getComputedStyle(document.activeElement).outlineColor === getComputedStyle(document.body).color && getComputedStyle(document.activeElement).outlineStyle === 'solid'")?;
        check(
            &ui,
            "parseFloat(getComputedStyle(document.activeElement).outlineOffset) > 0",
        )?;
        capture(&shots, "strip-focus", theme)?;
        key("Left")?;
        key("Down")?;
        check(&ui, "document.activeElement.matches('#strip-list button')")?;
        capture(&shots, "list-focus", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.matches('#strip-nodes button') && document.getElementById('strip-list').hidden")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('#strip-list button')")?;
        key("shift+F10")?;
        check(&ui, "document.activeElement.matches('[data-node-action]')")?;
        capture(&shots, "node-menu", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.matches('#strip-list button')")?;
        key("Escape")?;
        focus(&ui, "#tab-row [aria-selected=true]")?;
        key("Menu")?;
        check(&ui, "document.activeElement.dataset.tabAction === 'rename'")?;
        capture(&shots, "tab-menu", theme)?;
        key("Return")?;
        check(&ui, "document.activeElement.id === 'tree-name'")?;
        capture(&shots, "rename", theme)?;
        key("ctrl+a")?;
        type_text(&format!("Keyboard {theme}"))?;
        key("Return")?;
        wait("rename applied", || {
            text(&ui, "#tab-row").contains(&format!("Keyboard {theme}"))
        })?;
        check(&ui, "document.activeElement.matches('#tab-row [role=tab]')")?;
        chrome(&ui, "(window.keptTab=document.activeElement,true)")?;
        chrome(&ui, "canopy.present(S.frame)")?;
        check(&ui, "document.activeElement === window.keptTab")?;
        key("shift+F10")?;
        check(&ui, "document.activeElement.dataset.tabAction === 'rename'")?;
        key("Down")?;
        check(&ui, "document.activeElement.dataset.tabAction === 'pin'")?;
        key("Return")?;
        wait("pin state in accessible name", || {
            chrome(&ui, "document.querySelector('#tab-row [aria-selected=true]').getAttribute('aria-label').includes('pinned')").is_ok_and(|v|v==true)
        })?;
        key("shift+F10")?;
        key("Down")?;
        key("Return")?;
        focus(&ui, "#tab-overflow")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('#popover button')")?;
        capture(&shots, "overflow", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'tab-overflow'")?;
        ui.chrome.offer_logins(1, &["Keyboard one", "Keyboard two"]);
        pump(100);
        focus(&ui, "#login-chip")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('[data-login]')")?;
        capture(&shots, "logins", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'login-chip'")?;
        ui.chrome.offer_logins(2, &[]);
        ui.chrome.offer_save(Some(&canopy::chrome::SaveOffer {
            id: 3,
            site: "docs.localhost".into(),
            username: "Keyboard one".into(),
            update: false,
            keeper: "test vault".into(),
        }));
        pump(100);
        focus(&ui, "#login-chip")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('[data-save]')")?;
        capture(&shots, "save-login", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'login-chip'")?;
        ui.chrome.offer_save(None);
        focus(&ui, "#settings-button")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('#theme-mode button')")?;
        check(&ui, &format!("document.querySelector('#theme-mode [data-mode={theme}]').getAttribute('aria-pressed') === 'true'"))?;
        capture(&shots, "settings", theme)?;
        key("End")?;
        check(
            &ui,
            "document.activeElement.getAttribute('role') === 'switch'",
        )?;
        let before = chrome(&ui, "document.activeElement.getAttribute('aria-checked')")?;
        key("space")?;
        assert_ne!(
            chrome(&ui, "document.activeElement.getAttribute('aria-checked')")?,
            before
        );
        key("space")?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'settings-button'")?;
        focus(&ui, "#search-pill")?;
        key("Return")?;
        wait("palette options", || {
            chrome(
                &ui,
                "!!document.querySelector('#palette-results [role=option]')",
            )
            .is_ok_and(|v| v == true)
        })?;
        check(&ui, "document.activeElement.getAttribute('role') === 'combobox' && document.getElementById(document.activeElement.getAttribute('aria-activedescendant')).getAttribute('aria-selected') === 'true'")?;
        key("Down")?;
        capture(&shots, "palette", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'search-pill'")?;
        focus(&ui, "#waiting-pin")?;
        check(
            &ui,
            "document.activeElement.getAttribute('aria-label').includes('waiting, next')",
        )?;
        key("Down")?;
        check(&ui, "document.activeElement.matches('#popover [data-id]') && document.activeElement.getAttribute('aria-label').includes('waiting')")?;
        capture(&shots, "waiting", theme)?;
        key("Menu")?;
        key("Escape")?;
        check(
            &ui,
            "document.activeElement.matches('#popover.waiting [data-id]')",
        )?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'waiting-pin'")?;
        focus(&ui, "#zoom-pin")?;
        key("Return")?;
        wait("outline focus", || {
            chrome(
                &ui,
                "document.activeElement.getAttribute('role') === 'treeitem'",
            )
            .is_ok_and(|v| v == true)
        })?;
        check(&ui, "document.activeElement.getAttribute('aria-current') === 'page' && document.activeElement.getAttribute('aria-level') === '3'")?;
        capture(&shots, "outline", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'zoom-pin'")?;
        focus(&ui, "#forest-button")?;
        key("Return")?;
        check(&ui, "document.activeElement.id === 'forest-titles'")?;
        key("shift+Tab")?;
        check(&ui, "document.activeElement.closest('#forest') !== null")?;
        check(&ui, "[...document.querySelectorAll('.forest-node')].every(el=>!S.frame.visits[el.dataset.id].live || el.getAttribute('aria-label').includes('open in memory'))")?;
        capture(&shots, "forest", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'forest-button'")?;
        focus(&ui, "#memory-dot")?;
        key("Return")?;
        key("End")?;
        key("Return")?;
        wait("reader takes focus", || {
            chrome(&ui, "document.activeElement.matches('#reader button')").is_ok_and(|v| v == true)
        })?;
        capture(&shots, "reader", theme)?;
        key("shift+Tab")?;
        check(&ui, "document.activeElement.hasAttribute('data-capture')")?;
        key("Return")?;
        wait("saved text focused", || {
            chrome(&ui, "document.activeElement.matches('#reader .reader-close') && !!document.getElementById('capture-text')").is_ok_and(|v|v==true)
        })?;
        capture(&shots, "saved-text", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'memory-dot'")?;
        chrome(&ui, "canopy.toast('info', 'Keyboard status')")?;
        check(
            &ui,
            "document.getElementById('toast').getAttribute('role') === 'status'",
        )?;
        capture(&shots, "status", theme)?;
        chrome(&ui, "canopy.toast('error', 'Keyboard error')")?;
        check(
            &ui,
            "document.getElementById('toast').getAttribute('role') === 'alert'",
        )?;
        capture(&shots, "alert", theme)?;
        key("ctrl+t")?;
        wait("start visible", || visible(&ui, "#start"))?;
        chrome(
            &ui,
            "intent('pin-tree', {id:S.frame.tabs[0].id,pinned:true})",
        )?;
        wait("pinned card", || visible(&ui, "[data-start=tree]"))?;
        focus(&ui, "[data-start=tree]")?;
        key("Menu")?;
        check(&ui, "document.activeElement.dataset.tabAction === 'pin'")?;
        capture(&shots, "pinned-menu", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.dataset.start === 'tree'")?;
        focus(&ui, "[data-start=bookmark]")?;
        key("shift+F10")?;
        check(
            &ui,
            "document.activeElement.hasAttribute('data-remove-bookmark')",
        )?;
        capture(&shots, "bookmark-menu", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.dataset.start === 'bookmark'")?;
        chrome(&ui, "(setSurface('reading'),true)")?;
        let files_page = trees[0].2;
        ui.pages.go(Go::Visit(files_page))?;
        wait("page with files", || {
            ui.snapshot().is_ok_and(|s| s.current == Some(files_page))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        key("F6")?;
        focus(&ui, "#strip-canvas .leaves")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('[data-file]')")?;
        capture(&shots, "files", theme)?;
        key("Escape")?;
        check(
            &ui,
            "document.activeElement.matches('#strip-canvas .leaves')",
        )?;
        ui.pages.go(Go::Visit(*selected))?;
        pump(200);
    }
    println!("PASS real keys enter and leave strip lists, node menus, tab Rename and Pin, settings, palette, waiting, forest, reader and pinned menus in both themes; tab focus survives frames.");
    check(&ui, "document.getElementById('tab-row').getAttribute('role') === 'tablist' && document.querySelectorAll('#tab-row [aria-selected=true]').length === 1")?;
    check(&ui, "[...document.querySelectorAll('#strip-list [data-id]')].every(el=>el.getAttribute('aria-label').includes('branch pages'))")?;
    println!("PASS DOM names and states expose tab selection, page location, branch counts, waiting, switches, theme, palette selection and live toast roles.");
    ui.window.close();
    Ok(())
}

fn check(ui: &Ui, expression: &str) -> Result<()> {
    if chrome(ui, expression)? != true {
        anyhow::bail!("{expression}: {}", chrome(ui, "JSON.stringify({active:document.activeElement.outerHTML.slice(0,600),focused:document.hasFocus(),menu:S.popover?.kind,overlay:S.overlay})")?);
    }
    Ok(())
}
fn focus(ui: &Ui, selector: &str) -> Result<()> {
    chrome(
        ui,
        &format!(
            "(document.querySelector({}).focus(),true)",
            serde_json::to_string(selector)?
        ),
    )?;
    Ok(())
}
fn capture(dir: &Option<std::path::PathBuf>, name: &str, theme: &str) -> Result<()> {
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir)?;
        pump(200);
        anyhow::ensure!(
            std::process::Command::new("import")
                .args(["-window", "root"])
                .arg(dir.join(format!("{name}-{theme}.png")))
                .status()?
                .success(),
            "composed screenshot"
        );
    }
    Ok(())
}
