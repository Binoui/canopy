use anyhow::Result;
use canopy::{
    browser::{Browser, ThemeMode, VisitId},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;
mod common;
use common::*;
fn open(ui: &Ui) -> Result<()> {
    key("ctrl+e")?;
    wait("Canopy", || visible(ui, "#forest-canopy .canopy-item"))?;
    Ok(())
}
fn level(ui: &Ui, name: &str) -> Result<()> {
    click_chrome(ui, &format!("#canopy-zoom [data-level={name}]"))?;
    wait(&format!("zoom preset {name}"), || {
        chrome(ui, "mapView.drawn").is_ok_and(|v| v == name)
    })?;
    Ok(())
}
fn page(ui: &Ui, id: VisitId) -> Result<String> {
    let selector = format!("#forest-canopy [data-page='{id}']");
    chrome(
        ui,
        &format!("(focusItem(canopyValue().items.find(i=>i.pages?.includes('{id}')).k),true)"),
    )?;
    pump(50);
    Ok(selector)
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
            let body = "<!doctype html><title>Local conversation and test</title>Local page";
            let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type:text/html\r\nContent-Length:{}\r\nConnection:close\r\n\r\n{body}",body.len());
        }
    });
    let path = dir.path().join("forest.sqlite3");
    let mut b = Browser::open(&path)?;
    let roots = b.add_roots(
        &(0..5)
            .map(|n| url::Url::parse(&format!("http://hub{n}.localhost:{port}/{n}")).unwrap())
            .collect::<Vec<_>>(),
    )?;
    let trees: Vec<_> = b.tree_states()?.into_iter().map(|t| t.root).collect();
    b.place_tree(
        trees[3],
        canopy::browser::Move::Shelf(canopy::browser::Slot::First),
    )?;
    b.place_tree(trees[4], canopy::browser::Move::History)?;
    let db = rusqlite::Connection::open(&path)?;
    let mut ids = vec![roots[0]];
    for n in 1..=8 {
        let id: VisitId = ulid::Ulid::new().to_string().parse()?;
        db.execute(
            "INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?,?,?,?,?,?)",
            rusqlite::params![
                id.to_string(),
                ids.last().unwrap().to_string(),
                format!("http://test.localhost:{port}/{n}"),
                format!("Question {n}"),
                n,
                (n <= 6).then_some(n)
            ],
        )?;
        if n <= 6 {
            db.execute(
                "INSERT INTO visit_events(visit_id,kind,at) VALUES(?,'new',1)",
                [id.to_string()],
            )?;
        }
        ids.push(id);
    }
    db.execute(
        "INSERT INTO visit_events(visit_id,kind,at) VALUES(?,'return',1)",
        [roots[0].to_string()],
    )?;
    drop(db);
    drop(b);
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 800);
    ui.window.show_all();
    ready(&ui)?;
    ui.dispatch(ChromeIntent::Restore(ids[6]));
    wait("deep page", || {
        ui.snapshot().is_ok_and(|s| s.current == Some(ids[6]))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    open(&ui)?;
    assert_eq!(text(&ui, "#forest-button"), "Canopy");
    assert_eq!(chrome(&ui, "mapView.drawn")?, "middle");
    assert_eq!(
        chrome(
            &ui,
            "canopyValue().items.filter(i=>i.kind==='tree').map(i=>i.id)"
        )?,
        serde_json::json!(roots[..3]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>())
    );
    assert_eq!(
        chrome(&ui, "document.activeElement.dataset.current")?,
        "true"
    );
    println!("PASS Canopy opens at Middle, current first, tabs only, named counts, current focus.");
    let hub = page(&ui, roots[0])?;
    click_chrome(&ui, &hub)?;
    wait("hub click", || {
        !visible(&ui, "#forest") && ui.snapshot().is_ok_and(|s| s.current == Some(roots[0]))
    })?;
    open(&ui)?;
    assert_eq!(chrome(&ui, "S.forest.canopy.back.id")?, ids[6].to_string());
    click_chrome(&ui, "#canopy-back")?;
    wait("Back deep", || {
        !visible(&ui, "#forest") && ui.snapshot().is_ok_and(|s| s.current == Some(ids[6]))
    })?;
    open(&ui)?;
    let hub = page(&ui, roots[0])?;
    click_chrome(&ui, &hub)?;
    wait("hub again", || !visible(&ui, "#forest"))?;
    open(&ui)?;
    level(&ui, "far")?;
    let run = page(&ui, ids[6])?;
    click_chrome(&ui, &run)?;
    wait("folded return", || {
        !visible(&ui, "#forest") && ui.snapshot().is_ok_and(|s| s.current == Some(ids[6]))
    })?;
    println!("PASS owner journey deep to hub to Back and folded run returns to the deep page.");
    open(&ui)?;
    level(&ui, "near")?;
    assert_eq!(chrome(&ui,"canopyValue().items.filter(i=>i.pages?.length).filter(i=>i.pages.includes(S.frame.current)).length")?.as_u64(),Some(1));
    let current = page(&ui, ids[6])?;
    for button in ["5", "4"] {
        for _ in 0..12 {
            chrome(
                &ui,
                &format!(
                    r#"(()=>{{const item=canopyValue().items.find(i=>i.pages?.includes('{}')),node=mapView.nodes.get(item.k),r=node.el.getBoundingClientRect(),v=$('forest-canopy').getBoundingClientRect();globalThis.proof={{page:'{}',f:[.58,.47],at:[r.left-v.left+r.width*.58,r.top-v.top+r.height*.47]}};return true;}})()"#,
                    ids[6], ids[6]
                ),
            )?;
            let at = chrome(&ui, "proof.at")?;
            let v = rect(&ui, "#forest-canopy")?;
            move_to(
                (v.0 + at[0].as_f64().unwrap()).round() as i32,
                (v.1 + at[1].as_f64().unwrap()).round() as i32,
            )?;
            xdo(&["click", button])?;
            pump(80);
            let error=chrome(&ui,r#"(()=>{const i=canopyValue().items.find(i=>i.pages?.includes(proof.page)),r=mapView.nodes.get(i.k).el.getBoundingClientRect(),v=$('forest-canopy').getBoundingClientRect();return Math.hypot(r.left-v.left+r.width*proof.f[0]-proof.at[0],r.top-v.top+r.height*proof.f[1]-proof.at[1]);})()"#)?.as_f64().unwrap();
            assert!(error <= 1., "fractional wheel anchor error {error}");
        }
    }
    println!("PASS real wheel keeps an off-centre page point within 1 CSS px across both thresholds both ways.");
    let _ = current;
    level(&ui, "middle")?;
    let target = page(&ui, ids[6])?;
    let (x, y) = chrome_point(&ui, &target)?;
    move_to(x, y)?;
    let pan = chrome(&ui, "mapView.pan")?;
    xdo(&["mousedown", "1"])?;
    wait("held map button", || {
        chrome(&ui, "canopy.state().buttons").is_ok_and(|v| v == 1)
    })?;
    ui.dispatch(ChromeIntent::RenameTree(
        trees[0],
        "A refreshed conversation".into(),
    ));
    wait("held forest paint", || {
        chrome(&ui, "canopy.state().deferred.includes('forest')").is_ok_and(|v| v == true)
    })?;
    for n in 1..=8 {
        move_to(x + n * 5, y + 2)?;
    }
    xdo(&["mouseup", "1"])?;
    pump(200);
    assert!(visible(&ui, "#forest"));
    assert_eq!(ui.snapshot()?.current, Some(ids[6]));
    assert!(
        (chrome(&ui, "mapView.pan[0]")?.as_f64().unwrap() - pan[0].as_f64().unwrap() - 40.).abs()
            < 2.
    );
    println!("PASS real stepped drag holds refresh, moves the map and suppresses activation.");
    page(&ui, ids[6])?;
    key("Left")?;
    assert!(
        chrome(
            &ui,
            "document.activeElement.closest('#forest-canopy')!==null"
        )? == true
    );
    for _ in 0..10 {
        key("Tab")?;
        assert!(chrome(&ui, "document.activeElement.closest('#forest')!==null")? == true);
    }
    key("plus")?;
    key("minus")?;
    let target = page(&ui, roots[0])?;
    hover_chrome(&ui, &target)?;
    pump(500);
    assert!(visible(&ui, "#canopy-tip"));
    key("shift+F10")?;
    wait("page menu", || visible(&ui, "#popover [data-node-action]"))?;
    key("Escape")?;
    let label = format!("#forest-canopy [data-id='{}']", roots[0]);
    chrome(&ui, &format!("(focusItem('t:{}'),true)", roots[0]))?;
    hover_chrome(&ui, &label)?;
    xdo(&["click", "3"])?;
    wait("tree menu", || visible(&ui, "[data-tab-action=pin]"))?;
    click_chrome(&ui, "[data-tab-action=pin]")?;
    assert!(visible(&ui, "#forest"));
    println!(
        "PASS arrows, Tab trap, plus/minus, in-map hover, page and tree menus keep map usable."
    );
    std::fs::create_dir_all(".cache/shots/canopy-view")?;
    for width in [1920, 728, 390] {
        ui.window.resize(width, 900);
        pump(200);
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            ui.dispatch(ChromeIntent::SetTheme(mode));
            pump(100);
            for zoom in ["far", "middle", "near"] {
                println!("capture width={width} theme={mode:?} zoom={zoom}");
                level(&ui, zoom)?;
                ui.window.resize(width + 1, 900);
                pump(50);
                ui.window.resize(width, 900);
                pump(100);
                save_window(
                    &ui,
                    &format!(
                        ".cache/shots/canopy-view/smoke-{width}-{}-{zoom}.png",
                        if mode == ThemeMode::Dark {
                            "dark"
                        } else {
                            "light"
                        }
                    ),
                )?;
            }
        }
    }
    key("Escape")?;
    assert!(!visible(&ui, "#forest"));
    key("ctrl+h")?;
    wait("History", || visible(&ui, "#forest-history"))?;
    key("ctrl+e")?;
    wait("History to Canopy", || visible(&ui, "#forest-canopy"))?;
    key("ctrl+e")?;
    assert!(!visible(&ui, "#forest"));
    println!("PASS responsive dark/light captures, Escape, Ctrl+H and Ctrl+E transitions.");
    ui.window.close();
    Ok(())
}
