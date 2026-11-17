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
            let title = format!("Tutorial · {shape} page {i}");
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
                    assert_eq!(text(&ui, ".node.parent .item-label"), "chain page 6");
                }
                "fan" => {
                    assert_eq!(text(&ui, ".branch-count"), "12");
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
                key("Escape")?;
            }
            if *shape == "chain" {
                item_list(&ui, ".node.parent")?;
                assert!(text(&ui, "#strip-list").contains("Tutorial · chain page 6"));
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
            leave_strip(&ui)?;
        }
        if *shape == "fan" {
            item_list(&ui, selector)?;
            click_chrome(&ui, "#strip-list .more")?;
            wait("more opens tree view", || visible(&ui, "#tree-sheet"))?;
            key("Escape")?;
        }
        if *shape == "thread" {
            item_list(&ui, selector)?;
            let first = format!("#strip-list .menu-row[data-id='{}']", ids[1]);
            hover_chrome(&ui, &first)?;
            xdo(&["mousedown", "1"])?;
            pump(80);
            chrome(&ui, "(canopy.present(S.frame),true)")?;
            pump(100);
            wait("held list frame", || {
                chrome(&ui, "canopy.state().deferred.includes('present')").is_ok_and(|v| v == true)
            })?;
            xdo(&["mouseup", "1"])?;
            wait("held row opens queued page", || {
                ui.snapshot().is_ok_and(|s| s.current == Some(ids[1]))
            })?;
        }
        println!("PASS {shape} folding, named list, counts, six-row cap, no gap, pointer travel and stable rows in both themes.");
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
