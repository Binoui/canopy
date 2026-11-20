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
            db.execute("INSERT INTO captures(visit_id,document_url,title,body,captured_at,kind) VALUES(?1,?2,?3,?4,?5,'selection')", rusqlite::params![id.to_string(),url,title,"Saved notes about this page. The complete text remains searchable in Saved text. This preview wraps across two lines while the named branches remain visible below it.",serial])?;
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
            "INSERT INTO tree_state(root_id,last_visit_id,place,position,pinned,placed_at) VALUES(?1,?2,'tab',(SELECT COALESCE(MAX(position),-1)+1 FROM tree_state WHERE place='tab'),0,(SELECT created_at FROM visits WHERE id=?1))",
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
        wait("previous toast closed", || !visible(&ui, "#toast"))?;
        chrome(&ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        pump(150);
        ui.pages.selected_page().focus();
        key("shift+Tab")?;
        check(&ui, "document.hasFocus() && document.activeElement !== document.body && document.activeElement !== document.documentElement")?;
        key("F6")?;
        assert!(ui.pages.selected_page().native.has_focus());
        key("F6")?;
        check(&ui, "document.activeElement.matches('#strip-nodes [aria-current=page]') && !document.getElementById('strip-list').hidden")?;
        check(&ui, "getComputedStyle(document.activeElement).outlineColor === getComputedStyle(document.body).color && getComputedStyle(document.activeElement).outlineStyle === 'solid'")?;
        check(
            &ui,
            "parseFloat(getComputedStyle(document.activeElement).outlineOffset) > 0",
        )?;
        capture(&shots, "strip-focus", theme)?;
        key("Tab")?;
        check(&ui, "document.activeElement.id === 'memory-dot'")?;
        key("F6")?;
        key("F6")?;
        key("Left")?;
        key("Right")?;
        check(
            &ui,
            "document.activeElement.getAttribute('aria-current') === 'page'",
        )?;
        key("Left")?;
        key("Down")?;
        check(&ui, "document.activeElement.matches('#strip-list button')")?;
        check(&ui, "[...document.querySelectorAll('#strip-list [data-id]')].some(el=>el.getAttribute('aria-label').includes('1 branch page, 1 waiting')) && [...document.querySelectorAll('#strip-list [data-id]')].every(el=>!/(waiting.*waiting|1 branch pages)/.test(el.getAttribute('aria-label')))")?;
        capture(&shots, "list-focus", theme)?;
        key("Home")?;
        check(&ui, "document.activeElement.matches('.bl-head[role=group]') && document.activeElement.getAttribute('aria-label').startsWith(document.activeElement.querySelector('b:not([aria-hidden])').textContent) && parseFloat(getComputedStyle(document.activeElement).borderRadius) > 0 && parseFloat(getComputedStyle(document.activeElement).outlineOffset) < 0")?;
        capture(&shots, "list-header-ring", theme)?;
        key("F6")?;
        assert!(ui.pages.selected_page().native.has_focus());
        key("F6")?;
        key("Left")?;
        key("Down")?;
        key("F6")?;
        assert!(ui.pages.selected_page().native.has_focus());
        key("F6")?;
        key("Left")?;
        key("Down")?;
        key("Escape")?;
        check(&ui, "document.activeElement.matches('#strip-nodes button') && document.getElementById('strip-list').hidden")?;
        key("Menu")?;
        check(
            &ui,
            "document.activeElement.hasAttribute('data-node-action')",
        )?;
        capture(&shots, "item-menu", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.matches('#strip-nodes button')")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('#strip-list button')")?;
        key("shift+F10")?;
        check(&ui, "document.activeElement.matches('[data-node-action]')")?;
        capture(&shots, "node-menu", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.matches('#strip-list button')")?;
        key("Escape")?;
        ui.window.resize(1400, 800);
        pump(250);
        focus(&ui, "#tab-row [aria-selected=true]")?;
        key("Home")?;
        check(
            &ui,
            "document.activeElement === document.querySelector('#tab-row .tab:not([hidden])')",
        )?;
        key("Right")?;
        check(&ui, "document.activeElement === [...document.querySelectorAll('#tab-row .tab:not([hidden])')][1]")?;
        key("Left")?;
        key("End")?;
        check(&ui, "document.activeElement === [...document.querySelectorAll('#tab-row .tab:not([hidden])')].at(-1) && document.querySelectorAll('#tab-row [tabindex=\"0\"]').length === 1 && parseFloat(getComputedStyle(document.activeElement).outlineOffset) < 0")?;
        capture(&shots, "tab-ring", theme)?;
        ui.window.resize(820, 800);
        pump(250);
        focus(&ui, "#tab-row [aria-selected=true]")?;
        key("Menu")?;
        key("Down")?;
        key("Down")?;
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
        check(&ui, "document.activeElement.dataset.tabAction === 'pin'")?;
        key("Return")?;
        wait("pin state in accessible name", || {
            chrome(&ui, "document.querySelector('#tab-row [aria-selected=true]').getAttribute('aria-label').includes('pinned')").is_ok_and(|v|v==true)
        })?;
        check(&ui, "document.activeElement === window.keptTab")?;
        key("shift+F10")?;
        wait("Unpin menu focused", || {
            chrome(&ui, "document.activeElement.dataset.tabAction").is_ok_and(|v| v == "unpin")
        })
        .map_err(|error| {
            anyhow::anyhow!(
                "{error}; focus and menu {:?}",
                chrome(
                    &ui,
                    "({focus:document.activeElement.tagName,id:document.activeElement.dataset.id,menu:S.popover?.kind})"
                )
            )
        })?;
        key("Return")?;
        wait("tab unpinned", || {
            chrome(&ui, "S.frame.tabs.find(t=>t.active)?.pinned").is_ok_and(|v| v == false)
        })?;
        ui.window.resize(480, 800);
        wait("tab overflow at narrow width", || {
            visible(&ui, "#tab-overflow")
        }).map_err(|error|anyhow::anyhow!("{error}; tabs and fit {:?}",chrome(&ui,"({width:innerWidth,tabs:S.frame.tabs,fit:S.frame.tabFit,room:$('tabs-wrap')?.clientWidth})")))?;
        focus(&ui, "#tab-overflow")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('#popover button')")?;
        check(
            &ui,
            "document.getElementById('tab-overflow').getAttribute('aria-expanded') === 'true'",
        )?;
        capture(&shots, "overflow", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'tab-overflow'")?;
        ui.window.resize(820, 800);
        pump(250);
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
        check(
            &ui,
            "document.getElementById('settings-button').getAttribute('aria-expanded') === 'true'",
        )?;
        capture(&shots, "settings", theme)?;
        key("End")?;
        check(&ui, "document.activeElement.id === 'recent-errors'")?;
        key("Up")?;
        check(&ui, "document.activeElement.id === 'show-trees-help'")?;
        key("Up")?;
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
        key("F6")?;
        assert!(ui.pages.selected_page().native.has_focus());
        key("F6")?;
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
        check(
            &ui,
            "getComputedStyle(document.getElementById('palette-input')).outlineStyle === 'none'",
        )?;
        key("Down")?;
        capture(&shots, "palette", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'search-pill'")?;
        ui.window.resize(500, 800);
        settle(&ui)?;
        ui.pages.selected_page().focus();
        key("F6")?;
        let steps = chrome(
            &ui,
            "S.strip.trunk.length - S.strip.trunk.indexOf(current()) - 1",
        )?
        .as_u64()
        .unwrap();
        for _ in 0..steps {
            key("Right")?;
        }
        check(
            &ui,
            "document.activeElement.matches('.node.overflow[aria-haspopup=dialog]')",
        )?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('#popover [data-id]') && document.activeElement.getAttribute('aria-label').includes('waiting')")?;
        check(
            &ui,
            "document.querySelector('.node.overflow').getAttribute('aria-expanded') === 'true'",
        )?;
        capture(&shots, "waiting", theme)?;
        key("Menu")?;
        key("Escape")?;
        check(
            &ui,
            "document.activeElement.matches('#popover.waiting [data-id]')",
        )?;
        key("Escape")?;
        check(&ui, "document.activeElement.matches('.node.overflow')")?;
        ui.window.resize(820, 800);
        settle(&ui)?;
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
        check(
            &ui,
            "document.activeElement.matches('.canopy-item[data-current=true][data-page]')",
        )?;
        key("shift+Tab")?;
        check(&ui, "document.activeElement.closest('#forest') !== null")?;
        capture(&shots, "forest", theme)?;
        key("Escape")?;
        check(&ui, "document.activeElement.id === 'forest-button'")?;
        focus(&ui, "#memory-dot")?;
        key("Return")?;
        check(
            &ui,
            "document.getElementById('memory-dot').getAttribute('aria-expanded') === 'true'",
        )?;
        key("End")?;
        key("Return")?;
        wait("reader takes focus", || {
            chrome(&ui, "document.activeElement.matches('#reader button')").is_ok_and(|v| v == true)
        })?;
        check(
            &ui,
            "$('popover').getAttribute('aria-label') === 'Saved text'",
        )?;
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
            "document.getElementById('toast-status').textContent === 'Keyboard status' && document.getElementById('toast-status').getAttribute('role') === 'status' && getComputedStyle(document.getElementById('toast-status')).display !== 'none'",
        )?;
        capture(&shots, "status", theme)?;
        ui.chrome
            .toast(canopy::chrome::Toast::Error("Keyboard error".into()));
        wait("error alert announced", || {
            error_chrome(&ui, "$('toast-alert').textContent === 'Keyboard error' && $('toast-alert').getAttribute('role') === 'alert' && getComputedStyle($('toast-alert')).display !== 'none'").is_ok_and(|v| v == true)
        })?;
        check(
            &ui,
            "document.getElementById('toast-status').textContent === 'Keyboard status'",
        )?;
        capture(&shots, "alert", theme)?;
        key("Escape")?;
        wait("error toast dismissed", || !visible(&ui, "#error-toast"))?;
        ui.pages.selected_page().focus();
        key("ctrl+w")?;
        wait("settle shortcut toast", || {
            text(&ui, "#toast").contains("Undo (Ctrl+Shift+T)")
        })?;
        check(&ui, "document.querySelector('#toast button').textContent === 'Undo' && document.getElementById('toast-status').textContent.includes('Undo (Ctrl+Shift+T)')")?;
        capture(&shots, "settle-shortcut", theme)?;
        wait("settle toast gone", || !visible(&ui, "#toast"))?;
        key("ctrl+shift+t")?;
        wait("reopen after toast", || {
            ui.snapshot().is_ok_and(|s| s.current == Some(*selected))
        })?;
        ui.pages.place_tree(
            ui.pages
                .read(|b| {
                    canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)
                })?
                .tree_of(trees[4].2)
                .unwrap(),
            canopy::browser::Move::History,
        )?;
        wait("other tree settled", || {
            text(&ui, "#toast").contains("small")
        })?;
        key("ctrl+shift+t")?;
        wait("reopen other tree at last page", || {
            ui.snapshot().is_ok_and(|s| s.current == Some(trees[4].2))
        })?;
        ui.pages.go(Go::Visit(*selected))?;
        settle(&ui)?;
        key("ctrl+t")?;
        wait("start visible", || visible(&ui, "#start"))?;
        chrome(&ui, "treeAct(S.frame.tabs[0].id,'save')")?;
        wait("pinned card", || visible(&ui, "[data-start=tree]"))?;
        focus(&ui, "[data-start=tree]")?;
        key("Menu")?;
        check(&ui, "document.activeElement.dataset.tabAction === 'open'")?;
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
        let after_page = trees[3].2;
        ui.pages.go(Go::Visit(after_page))?;
        wait("page with waiting children", || {
            ui.snapshot().is_ok_and(|s| s.current == Some(after_page))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        settle(&ui)?;
        ui.pages.selected_page().focus();
        key("F6")?;
        key("Right")?;
        check(&ui, "document.activeElement.matches('.node.after') && document.activeElement.getAttribute('aria-label').includes('4 read pages')")?;
        capture(&shots, "after-count", theme)?;
        key("Down")?;
        check(&ui, "document.activeElement.matches('#strip-list [data-id]') && !document.activeElement.getAttribute('aria-label').includes('waiting')")?;
        key("Escape")?;
        check(&ui, "document.activeElement.matches('.node.after')")?;
        key("Left")?;
        check(
            &ui,
            "document.activeElement.getAttribute('aria-current') === 'page'",
        )?;
        let files_page = trees[0].2;
        ui.pages.go(Go::Visit(files_page))?;
        wait("page with files", || {
            ui.snapshot().is_ok_and(|s| s.current == Some(files_page))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        ui.pages.selected_page().focus();
        key("F6")?;
        settle(&ui)?;
        focus(&ui, "#strip-canvas .leaves")?;
        key("Return")?;
        check(&ui, "document.activeElement.matches('[data-file]')")?;
        check(&ui, "document.querySelector('#strip-canvas .leaves[aria-expanded=true]').getAttribute('aria-haspopup') === 'menu'")?;
        capture(&shots, "files", theme)?;
        key("Escape")?;
        check(
            &ui,
            "document.activeElement.matches('#strip-canvas .leaves')",
        )?;
        check(&ui, "['tab-overflow','settings-button','memory-dot'].every(id=>document.getElementById(id).getAttribute('aria-haspopup') === 'menu' && document.getElementById(id).getAttribute('aria-expanded') === 'false') && document.getElementById('tab-overflow').getAttribute('aria-label') === 'More trees'")?;
        check(&ui, "[...document.querySelectorAll('.fav b')].every(el=>el.getAttribute('aria-hidden') === 'true') && document.querySelector('#strip-nodes [aria-current=page]').nextElementSibling.id === 'memory-dot'")?;
        ui.pages.go(Go::Visit(*selected))?;
        pump(200);
        ui.pages.selected_page().focus();
        hover_chrome(&ui, "#strip-nodes [aria-current=page]")?;
        wait("pointer list", || visible(&ui, "#strip-list"))?;
        click_chrome(&ui, "#strip-list .bl-head")?;
        move_to(700, 600)?;
        wait("pointer list closes after click and leave", || {
            !visible(&ui, "#strip-list")
        })?;
        ui.pages.selected_page().focus();
        key("F6")?;
        check(&ui, "document.hasFocus() && document.activeElement.matches('#strip-nodes [aria-current=page]')")?;
        hover_chrome(&ui, "#strip-nodes [aria-current=page]")?;
        move_to(700, 600)?;
        pump(350);
        check(&ui, "!document.getElementById('strip-list').hidden")?;
        key("Down")?;
        hover_chrome(&ui, "#strip-list .bl-head")?;
        move_to(700, 600)?;
        pump(350);
        check(&ui, "!document.getElementById('strip-list').hidden")?;
        key("Escape")?;
        ui.pages.selected_page().focus();
    }
    println!("PASS real keys enter and leave strip lists, node menus, tab Rename and Pin, settings, palette, waiting, forest, reader and pinned menus in both themes; tab focus survives frames.");
    check(&ui, "document.getElementById('tab-row').getAttribute('role') === 'tablist' && document.querySelectorAll('#tab-row [aria-selected=true]').length === 1")?;
    check(&ui, "[...document.querySelectorAll('#strip-nodes .node:not(.overflow), #strip-list [data-id]')].every(el=>!/(\\b0 branch|\\b0 waiting|\\b1 branch pages|waiting.*waiting)/.test(el.getAttribute('aria-label')))")?;
    println!("PASS DOM names and states expose tab selection, page location, branch counts, waiting, switches, theme, palette selection and live toast roles.");
    ui.window.close();
    Ok(())
}

fn check(ui: &Ui, expression: &str) -> Result<()> {
    if chrome(ui, expression)? != true {
        anyhow::bail!("{expression}: {}", chrome(ui, "JSON.stringify({active:document.activeElement.outerHTML.slice(0,600),tabs:[...document.querySelectorAll('#tab-row .tab')].map(e=>({text:e.textContent,hidden:e.hidden,active:e===document.activeElement})),focused:document.hasFocus(),menu:S.popover?.kind,overlay:S.overlay})")?);
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
        pump(450);
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
