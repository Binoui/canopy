use anyhow::Result;
use canopy::{
    browser::{Browser, VisitId},
    pages::Go,
    ui::Ui,
};
use gtk::prelude::*;
use serde_json::json;
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
    ] {
        let mut ids = Vec::new();
        for i in 0..depth + children {
            let id: VisitId = ulid::Ulid::new().to_string().parse()?;
            let parent = if i == 0 {
                None
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
            "INSERT INTO tree_state(root_id,last_visit_id,tab_order) VALUES(?1,?2,?3)",
            rusqlite::params![ids[0].to_string(), current.to_string(), trees.len()],
        )?;
        trees.push((shape, ids, current));
    }
    db.execute(
        "UPDATE session SET current_id=?1 WHERE singleton=1",
        [trees[0].2.to_string()],
    )?;
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
    let shots = std::env::var_os("CANOPY_STRIP_SHOTS").map(std::path::PathBuf::from);
    if let Some(path) = &shots {
        std::fs::create_dir_all(path)?;
    }
    for width in [1240, 820] {
        ui.window.resize(width, 800);
        pump(400);
        let shots = shots.as_ref().map(|path| path.join(width.to_string()));
        if let Some(path) = &shots {
            std::fs::create_dir_all(path)?;
        }
        for (shape, ids, selected) in &trees {
            ui.pages.go(Go::Visit(*selected))?;
            wait("fixture selected", || {
                ui.snapshot().is_ok_and(|s| s.current == Some(*selected))
                    && !ui.pages.selected_page().native.is_loading()
            })?;
            settle(&ui)?;
            let selector = match *shape {
                "chain" => ".node.run",
                "fan" => ".node.root",
                "trail" => ".node.earlier",
                _ => ".node.after",
            };
            for theme in ["dark", "light"] {
                chrome(&ui, &format!("(intent('theme', {{mode:'{theme}'}}),true)"))?;
                wait("persisted theme", || {
                    chrome(&ui, "canopy.state().dark").is_ok_and(|v| v == (theme == "dark"))
                })?;
                leave_strip(&ui)?;
                if let Some(path) = &shots {
                    save_window(
                        &ui,
                        path.join(format!("{shape}-{theme}-rest.png"))
                            .to_str()
                            .unwrap(),
                    )?;
                }
                item_list(&ui, selector)?;
                if *shape == "fan" {
                    wait("saved preview in list head", || visible(&ui, ".bl-snippet"))?;
                    assert!(text(&ui, ".bl-snippet").contains("Saved notes"));
                    assert_eq!(
                    chrome(
                        &ui,
                        "getComputedStyle(document.querySelector('.bl-snippet')).webkitLineClamp"
                    )?,
                    "2"
                );
                }
                assert_eq!(strip_height(&ui), 52);
                assert_eq!(chrome(&ui, "[...document.querySelectorAll('.node.run,.node.earlier')].every(e=>!e.hasAttribute('title'))")?, true);
                assert_eq!(chrome(&ui, "[...document.querySelectorAll('#strip-list .menu-detail,#strip-list .more')].every(e=>!/(^| · )0 (waiting|read|more)/.test(e.textContent))")?, true);
                assert_eq!(chrome(&ui, "[...document.querySelectorAll('#strip-list .menu-row[data-id]')].every(e=>e.title===S.frame.visits[e.dataset.id].label && e.getAttribute('aria-label').startsWith(e.title) && !/(\\b0 branch|\\b0 waiting|\\b1 branch pages|waiting.*waiting)/.test(e.getAttribute('aria-label')) && !e.querySelector('.grow').textContent.includes('tutorial ·'))")?, true);
                assert!(
                    chrome(
                        &ui,
                        "document.querySelectorAll('#strip-list .menu-row[data-id]').length <= 6"
                    )? == true
                );
                let (_, y, _, h) = rect(&ui, selector)?;
                let (_, ly, _, _) = rect(&ui, "#strip-list")?;
                let bridge = chrome(
                    &ui,
                    &format!(
                    "parseFloat(getComputedStyle(document.querySelector({}), '::after').height)",
                    serde_json::to_string(selector)?
                ),
                )?
                .as_f64()
                .unwrap();
                assert!(
                    ly >= y + h && ly <= y + h + bridge + 1.,
                    "list touches the item's hover bridge"
                );
                let (x, _) = chrome_point(&ui, selector)?;
                let (_, bridge_y) =
                    screen_point(&ui, &ui.chrome.native(), 0, ((y + h + ly) / 2.) as i32)?;
                move_to(x, bridge_y)?;
                pump(350);
                assert!(
                    visible(&ui, "#strip-list"),
                    "slow travel through the bridge keeps the list"
                );
                hover_chrome(&ui, "#strip-list .bl-head")?;
                pump(500);
                assert!(visible(&ui, "#strip-list"));
                chrome(&ui, "(window.keptRow=document.querySelector('#strip-list .menu-row[data-id]'),canopy.present(S.frame),true)")?;
                settle(&ui)?;
                assert!(
                chrome(
                    &ui,
                    "window.keptRow === document.querySelector('#strip-list .menu-row[data-id]')"
                )? == true
            );
                if let Some(path) = &shots {
                    save_window(
                        &ui,
                        path.join(format!("{shape}-{theme}-list.png"))
                            .to_str()
                            .unwrap(),
                    )?;
                    let output = std::process::Command::new("import")
                        .args(["-window", "root"])
                        .arg(path.join(format!("{shape}-{theme}-composed.png")))
                        .status()?;
                    anyhow::ensure!(output.success(), "composed capture");
                }
                match *shape {
                    "chain" => {
                        assert!(text(&ui, "#strip-list").contains("5 pages on docs.localhost"));
                        assert_eq!(chrome(&ui, "(()=>{const e=document.querySelector('.node.parent .item-label');return e.scrollWidth<=e.clientWidth})()")?, true);
                        assert_eq!(chrome(&ui, "(()=>{const e=document.querySelector('.node.run'),l=e.querySelector('.item-label');return l.scrollWidth<=l.clientWidth && e.getBoundingClientRect().right-l.getBoundingClientRect().right>=9})()")?, true);
                        assert_eq!(
                            text(&ui, "#strip-list .menu-row[data-id] .grow"),
                            "Spawning 1"
                        );
                        assert_eq!(text(&ui, ".node.parent .item-label"), "Framing");
                    }
                    "fan" => {
                        assert_eq!(text(&ui, ".branch-count"), "12");
                        assert_eq!(text(&ui, ".node.parent .item-label"), "this month?");
                        assert!(visible(&ui, ".branch-count.waiting"));
                        assert!(text(&ui, "#strip-list .more").contains("6 more"));
                    }
                    "trail" => {
                        assert!(text(&ui, "#strip-list").contains("earlier pages"));
                        assert_eq!(chrome(&ui,"document.querySelector('#strip-list .menu-row[data-id]').dataset.id === S.strip.nodes.find(n=>n.kind==='earlier').ids.at(-1)")?, true);
                    }
                    _ => {
                        assert_eq!(text(&ui, ".node.after .read-count"), "4");
                        assert_eq!(text(&ui, ".node.after .wait-count"), "14");
                        assert!(text(&ui, "#strip-list").contains("8 more waiting · 4 read"));
                        assert_eq!(chrome(&ui,"[...document.querySelectorAll('#strip-list .menu-row[data-id]')].map(e=>e.dataset.id)")?, json!(ids[1..7].iter().map(ToString::to_string).collect::<Vec<_>>()));
                    }
                }
                let (away_x, away_y) = page_point(&ui, 800, 600)?;
                move_to(away_x, away_y)?;
                for _ in 0..3 {
                    pump(40);
                    chrome(&ui, "(canopy.present(S.frame),true)")?;
                }
                wait("frames do not cancel dismissal", || {
                    !visible(&ui, "#strip-list")
                })?;
                if ["chain", "trail"].contains(shape) {
                    let item_key = chrome(
                        &ui,
                        &format!(
                            "document.querySelector({}).dataset.id",
                            serde_json::to_string(selector)?
                        ),
                    )?;
                    let leaf = format!(".leaves[data-id='{}']", item_key.as_str().unwrap());
                    assert!(visible(&ui, &leaf));
                    click_chrome(&ui, &leaf)?;
                    wait("capsule files", || visible(&ui, "#popover .file-row"))?;
                    assert_eq!(
                        chrome(
                            &ui,
                            "document.querySelectorAll('#popover .file-row').length"
                        )?,
                        if *shape == "chain" { 2 } else { 1 }
                    );
                    if *shape == "chain" {
                        assert_eq!(text(&ui, "#popover .menu-heading"), "2 files from 2 pages");
                    }
                    key("Escape")?;
                }
                if *shape == "chain" {
                    item_list(&ui, ".node.parent")?;
                    assert!(text(&ui, "#strip-list").contains("Tokio tutorial · Framing"));
                    if let Some(path) = &shots {
                        save_window(
                            &ui,
                            path.join(format!("parent-{theme}-list.png"))
                                .to_str()
                                .unwrap(),
                        )?;
                    }
                }
                open_list(&ui)?;
                wait("current saved preview", || {
                    visible(&ui, "#strip-list .bl-snippet")
                })?;
                if let Some(path) = &shots {
                    save_window(
                        &ui,
                        path.join(format!("{shape}-{theme}-current-list.png"))
                            .to_str()
                            .unwrap(),
                    )?;
                }
                if *shape == "thread" {
                    assert_eq!(chrome(&ui, "[...document.querySelectorAll('#strip-list .menu-row[data-id]')].map(e=>e.dataset.id)")?, json!(ids[15..19].iter().rev().map(ToString::to_string).collect::<Vec<_>>()));
                    assert!(!visible(&ui, "#strip-list .more"));
                }
                leave_strip(&ui)?;
                ui.pages.selected_page().focus();
                key("F6")?;
                wait("F6 opens list", || visible(&ui, "#strip-list"))?;
                let (x, y) = page_point(&ui, 500, 600)?;
                xdo(&["mousemove", &x.to_string(), &y.to_string(), "click", "1"])?;
                wait("page focus dismisses keyboard list", || {
                    !visible(&ui, "#strip-list")
                })?;
                key("F6")?;
                wait("F6 reopens list", || visible(&ui, "#strip-list"))?;
                chrome(
                    &ui,
                    "(document.querySelector('#settings-button').focus(),true)",
                )?;
                wait("focus outside item dismisses list", || {
                    !visible(&ui, "#strip-list")
                })?;
                key("F6")?;
                wait("F6 from the header returns to the page", || {
                    ui.pages.selected_page().native.has_focus()
                })?;
                key("F6")?;
                wait("F6 opens list before focus clears", || {
                    visible(&ui, "#strip-list")
                })?;
                chrome(&ui, "(document.activeElement.blur(),true)")?;
                wait("cleared focus dismisses list", || {
                    !visible(&ui, "#strip-list")
                })?;
                leave_strip(&ui)?;
            }
            if *shape == "trail" {
                for side in ["tree", "list"] {
                    chrome(&ui, &format!("(S.treeSide='{side}',true)"))?;
                    item_list(&ui, selector)?;
                    click_chrome(&ui, "#strip-list .more")?;
                    wait("earliest hidden page visible", || {
                        chrome(&ui, &format!("(()=>{{const body=document.querySelector('#tree-{0}'),e=body.querySelector('[data-id=\"{1}\"]');if(!e)return false;const a=e.getBoundingClientRect(),b=body.getBoundingClientRect();return a.left>=b.left && a.right<=b.right && a.top>=b.top && a.bottom<=b.bottom}})()", if side=="tree" {"board"} else {"list"}, ids[1])).is_ok_and(|v|v==true)
                    })?;
                    key("Escape")?;
                }
            }
            if *shape == "fan" {
                item_list(&ui, selector)?;
                click_chrome(&ui, "#strip-list .more")?;
                wait("more opens tree view", || visible(&ui, "#tree-sheet"))?;
                key("Escape")?;
            }
            if *shape == "thread" && width == 820 {
                item_list(&ui, selector)?;
                let first = format!("#strip-list .menu-row[data-id='{}']", ids[1]);
                hover_chrome(&ui, &first)?;
                xdo(&["mousedown", "1"])?;
                pump(80);
                chrome(&ui, "(canopy.present(S.frame),true)")?;
                pump(100);
                wait("held list frame", || {
                    chrome(&ui, "canopy.state().deferred.includes('present')")
                        .is_ok_and(|v| v == true)
                })?;
                xdo(&["mouseup", "1"])?;
                wait("held row opens queued page", || {
                    ui.snapshot().is_ok_and(|s| s.current == Some(ids[1]))
                })?;
            }
            println!("PASS {shape} folding, named list, counts, six-row cap, no gap, pointer travel and stable rows in both themes.");
        }
    }
    open_list(&ui)?;
    assert_eq!(text(&ui, "#strip-list [data-act=reload]"), "Reload");
    let before_reload = ui.snapshot()?;
    eval(&ui, "window.listReloadMarker=true;JSON.stringify(true)")?;
    click_chrome(&ui, "#strip-list [data-act=reload]")?;
    wait("list reloads the current document", || {
        eval(&ui, "JSON.stringify(window.listReloadMarker === undefined)").is_ok_and(|v| v == true)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert_eq!(ui.snapshot()?.current, before_reload.current);
    assert_eq!(ui.snapshot()?.visits.len(), before_reload.visits.len());
    open_list(&ui)?;
    chrome(&ui,"(window.keptAction=document.querySelector('#strip-list [data-act=branch]'),canopy.present(S.frame),true)")?;
    assert!(
        chrome(
            &ui,
            "window.keptAction === document.querySelector('#strip-list [data-act=branch]')"
        )? == true
    );
    click_chrome(&ui, "#strip-list [data-act=branch]")?;
    wait("list branch opens scoped palette", || {
        visible(&ui, "#palette-context")
    })?;
    key("Escape")?;
    println!("PASS list actions persist across frames, New branch opens the palette, more opens the tree, and a held waiting row survives a refresh.");
    Ok(())
}
