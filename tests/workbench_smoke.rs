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
    let shots = std::env::var_os("CANOPY_WORKBENCH_SHOTS").map(std::path::PathBuf::from);
    let (_, ids, selected) = &trees[1];
    ui.pages.go(Go::Visit(*selected))?;
    wait("fixture selected", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(*selected))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    settle(&ui)?;
    for theme in ["dark", "light"] {
        let waiting = if theme == "dark" { ids[4] } else { ids[5] };
        chrome(&ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        pump(150);
        chrome(&ui, "(document.getElementById('zoom-pin').focus(),true)")?;
        key("ctrl+shift+e")?;
        wait("large tree opens List with row focus", || {
            chrome(
                &ui,
                "S.treeSide === 'list' && document.activeElement.matches('#tree-list [data-id]')",
            )
            .is_ok_and(|v| v == true)
        })?;
        assert_eq!(chrome(&ui, "(()=>{const a=document.activeElement.getBoundingClientRect(),b=document.getElementById('tree-list').getBoundingClientRect();return a.top>=b.top && a.bottom<=b.bottom})()")?, true);
        capture(&shots, "list", theme)?;
        key("Up")?;
        let focused = chrome(&ui, "document.activeElement.dataset.id")?;
        chrome(&ui, "(window.keptTreeNode=document.activeElement,true)")?;
        chrome(&ui, "canopy.present(S.frame)")?;
        assert_eq!(
            chrome(&ui, "document.activeElement === window.keptTreeNode")?,
            true
        );
        key("Down")?;
        assert_ne!(chrome(&ui, "document.activeElement.dataset.id")?, focused);
        chrome(&ui, &format!("(focusTree('{}'),true)", ids[0]))?;
        key("Left")?;
        wait("fold hides descendants", || {
            chrome(&ui, "S.frame.treeView.rows.length").is_ok_and(|v| v == 1)
        })?;
        assert!(text(&ui, "#tree-list").contains("pages ·"));
        capture(&shots, "folded", theme)?;
        key("Right")?;
        wait("unfold restores descendants", || {
            chrome(&ui, "S.frame.treeView.rows.length").is_ok_and(|v| v == ids.len())
        })?;
        type_text("fan page 5")?;
        wait("typed title filter", || {
            chrome(&ui, "S.frame.treeView.query").is_ok_and(|v| v == "fan page 5")
        })?;
        assert_eq!(chrome(&ui, "S.frame.treeView.rows.length")?, 2);
        capture(&shots, "filter", theme)?;
        chrome(&ui, "(document.getElementById('tree-filter').value='',document.getElementById('tree-filter').dispatchEvent(new Event('input')),true)")?;
        wait("filter cleared", || {
            chrome(&ui, "S.frame.treeView.query").is_ok_and(|v| v == "")
        })?;
        click_chrome(&ui, "#tree-waiting")?;
        wait("waiting filter", || {
            chrome(&ui, "S.frame.treeView.waitingOnly").is_ok_and(|v| v == true)
        })?;
        capture(&shots, "waiting", theme)?;
        chrome(&ui, &format!("(focusTree('{}'),true)", waiting))?;
        key("shift+F10")?;
        wait("keyboard node menu", || {
            visible(&ui, "[data-node-action='copy']")
        })?;
        capture(&shots, "menu", theme)?;
        click_chrome(&ui, "[data-node-action='copy']")?;
        assert!(visible(&ui, "#tree-sheet"));
        key("Delete")?;
        wait("Delete dismisses waiting page", || {
            ui.pages
                .read(|b| b.visit(waiting))
                .is_ok_and(|v| v.dismissed_at.is_some())
        })?;
        assert!(visible(&ui, "#tree-sheet"));
        assert_eq!(
            chrome(
                &ui,
                "document.activeElement.matches('#tree-list [data-id]')"
            )?,
            true
        );
        click_chrome(&ui, "#tree-waiting")?;
        wait("waiting filter cleared", || {
            chrome(&ui, "S.frame.treeView.waitingOnly").is_ok_and(|v| v == false)
        })?;
        click_chrome(&ui, "#tree-side [data-side='tree']")?;
        capture(&shots, "tree", theme)?;
        ui.window.resize(480, 800);
        pump(200);
        assert_eq!(chrome(&ui, "(()=>{const b=document.getElementById('tree-board'); b.scrollLeft=0;b.dispatchEvent(new WheelEvent('wheel',{deltaY:100,cancelable:true}));return b.scrollWidth>b.clientWidth && b.scrollLeft>0})()")?, true);
        ui.window.resize(820, 800);
        pump(100);
        key("Escape")?;
        assert_eq!(chrome(&ui, "document.activeElement.id")?, "zoom-pin");
    }
    println!("PASS List default, focus and return, arrows, folds, typing, Waiting only, keyboard menu, Delete, non-navigation stays open, and keyed nodes across frames.");
    ui.pages.settle_branch(ids[1])?;
    pump(200);
    key("ctrl+shift+e")?;
    wait("settled branch starts folded", || {
        chrome(
            &ui,
            &format!("S.frame.treeView.rows.find(r=>r.id==='{}')?.folded", ids[1]),
        )
        .is_ok_and(|v| v == true)
    })?;
    assert_eq!(
        chrome(
            &ui,
            &format!("S.frame.treeView.rows.some(r=>r.id==='{}')", ids[2])
        )?,
        false
    );
    for theme in ["dark", "light"] {
        chrome(&ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        pump(150);
        capture(&shots, "settled", theme)?;
    }
    chrome(&ui, &format!("(focusTree('{}'),true)", ids[1]))?;
    key("Right")?;
    wait("settled branch can unfold", || {
        chrome(
            &ui,
            &format!("S.frame.treeView.rows.some(r=>r.id==='{}')", ids[2]),
        )
        .is_ok_and(|v| v == true)
    })?;
    key("Return")?;
    wait("Enter opens and closes sheet", || {
        !visible(&ui, "#tree-sheet") && ui.snapshot().is_ok_and(|s| s.current == Some(ids[1]))
    })?;
    println!("PASS settled branch starts folded, unfolds without navigating, and Enter opens the selected page.");
    key("ctrl+shift+e")?;
    wait("sheet reopened", || visible(&ui, "#tree-sheet"))?;
    chrome(&ui, "(document.getElementById('tree-filter').focus(),true)")?;
    type_text("no such page")?;
    wait("empty filter", || visible(&ui, "#tree-empty"))?;
    for theme in ["dark", "light"] {
        chrome(&ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        pump(150);
        capture(&shots, "empty", theme)?;
    }
    chrome(&ui, "(document.getElementById('tree-filter').value='',document.getElementById('tree-filter').dispatchEvent(new Event('input')),true)")?;
    wait("clear final filter", || {
        chrome(&ui, "S.frame.treeView.query").is_ok_and(|v| v == "")
    })?;
    key("Escape")?;
    let small = trees[4].2;
    ui.pages.go(Go::Visit(small))?;
    wait("small tree selected", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(small))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    chrome(&ui, "(S.treeSide='tree',true)")?;
    key("ctrl+shift+e")?;
    wait("small tree remembers Tree side", || {
        chrome(&ui, "S.overlay==='tree' && S.treeSide==='tree'").is_ok_and(|v| v == true)
    })?;
    click_chrome(&ui, "#tree-side [data-side='list']")?;
    key("Escape")?;
    key("ctrl+shift+e")?;
    wait("small tree remembers List side", || {
        chrome(&ui, "S.overlay==='tree' && S.treeSide==='list'").is_ok_and(|v| v == true)
    })?;
    key("Escape")?;
    println!("PASS small trees remember the last side and the wheel pans an overflowing Tree at 480 pixels.");
    ui.window.close();
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
