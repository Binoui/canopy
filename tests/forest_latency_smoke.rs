use anyhow::Result;
use canopy::{browser::Browser, ui::Ui};
use gtk::prelude::*;
use std::time::Instant;
mod common;
use common::*;
fn main() -> Result<()> {
    if std::env::var_os("FOREST_LATENCY").is_none() {
        return Ok(());
    }
    let port = if std::env::var_os("FOREST_BIG").is_some() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                let body = "<!doctype html><title>Large tree page</title>Local benchmark page";
                let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type:text/html\r\nContent-Length:{}\r\nConnection:close\r\n\r\n{body}",body.len());
            }
        });
        port
    } else {
        9
    };
    let _runtime = private_runtime()?;
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("forest.sqlite3");
    if let Some(source) = std::env::var_os("FOREST_FIXTURE") {
        std::fs::copy(source, &database)?;
    } else {
        Browser::open(&dir.path().join("forest.sqlite3"))?;
        let mut db = rusqlite::Connection::open(dir.path().join("forest.sqlite3"))?;
        let tx = db.transaction()?;
        let id = |n: usize| ulid::Ulid::from(n as u128 + 1).to_string();
        let big = std::env::var("FOREST_BIG")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());
        let pages = big.unwrap_or(10);
        for t in 0..if big.is_some() { 1 } else { 2000 } {
            for p in 0..pages {
                let n = t * pages + p;
                tx.execute("INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?,?,?,?,?,?)",rusqlite::params![id(n), (p>0).then(|| id(n-1)), format!("http://site{}.localhost:{port}/p/{t}/{p}",p%4),format!("Project {t} page {p}"),1_790_000_000_000i64+n as i64,1_790_000_000_000i64+n as i64])?;
                if big.is_some() {
                    tx.execute(
                        "INSERT INTO visit_events(visit_id,kind,at) VALUES(?,'new',1)",
                        [id(n)],
                    )?;
                }
            }
            tx.execute("INSERT INTO tree_state(root_id,last_visit_id,name,place,position,pinned,placed_at) VALUES(?,?,?,?,?,?,?)",rusqlite::params![id(t*pages),id(t*pages+pages-1),format!("Project {t}"),if t<4 {"tab"} else if t==4 {"shelf"} else {"history"},(t<5).then_some(t as i64),t<4,1_790_000_000_000i64+t as i64])?;
        }
        tx.execute("UPDATE session SET current_id=?", [id(pages - 1)])?;
        tx.commit()?;
    }
    let db = rusqlite::Connection::open(&database)?;
    if let Some(destination) = std::env::var_os("FOREST_FIXTURE_OUT") {
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        std::fs::copy(&database, destination)?;
    }
    println!(
        "fixture trees={} pages={}",
        db.query_row("SELECT count(*) FROM tree_state", [], |r| r
            .get::<_, i64>(0))?,
        db.query_row("SELECT count(*) FROM visits", [], |r| r.get::<_, i64>(0))?
    );
    if std::env::var_os("FOREST_BIG").is_some() {
        let b = Browser::open(&database)?;
        for sample in 0..6 {
            let start = Instant::now();
            let shown = b.showings()?;
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            assert!(shown.times(ulid::Ulid::from(1_u128).to_string().parse()?) > 0);
            println!("showings query sample={sample} {elapsed:.3} ms");
        }
    }
    if let Ok(layout) = std::env::var("FOREST_LAYOUT") {
        let layout = canopy::chrome::CanopyLayout::parse(&layout).expect("valid observer layout");
        std::fs::write(dir.path().join("canopy-layout"), layout.name())?;
    }
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 900);
    ui.window.show_all();
    ready(&ui)?;
    pump(1000);
    if std::env::var_os("FOREST_PROFILE").is_some() {
        chrome(
            &ui,
            r#"(()=>{const paint=canopy.forest;canopy.forest=(...args)=>{const begin=performance.timeOrigin+performance.now();const result=paint(...args);globalThis.paintProfile={begin,end:performance.timeOrigin+performance.now()};return result;};return true;})()"#,
        )?;
    }
    for round in 0..2 {
        focus_window()?;
        chrome(
            &ui,
            r#"(() => {
          globalThis.painted=null;
          const check=()=>{
            const forest=document.querySelector('#forest');
            const ready=!forest.hidden && S.forest?.count && (S.forest.view==='history' ? document.querySelectorAll('#forest-list .forest-row').length===20 : S.forest.view==='canopy' && document.querySelector('#forest-canopy .canopy-item[data-page]'));
            if(!ready) { requestAnimationFrame(check); return; }
            requestAnimationFrame(()=>requestAnimationFrame(()=>{painted=performance.timeOrigin+performance.now();}));
          };
          requestAnimationFrame(check); return true;
        })()"#,
        )?;
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs_f64()
            * 1000.;
        let started = Instant::now();
        xdo(&[
            "key",
            "--clearmodifiers",
            if std::env::var_os("FOREST_HISTORY").is_some() {
                "ctrl+h"
            } else {
                "ctrl+e"
            },
        ])?;
        wait("forest paint", || {
            chrome(&ui, "painted!==null").is_ok_and(|v| v == true)
        })?;
        let elapsed = chrome(&ui, "painted")?.as_f64().unwrap() - wall;
        println!(
            "forest {} {:.3} ms (observed by {:.3} ms)",
            if round == 0 { "first" } else { "reopen" },
            elapsed,
            started.elapsed().as_secs_f64() * 1000.
        );
        println!(
            "payload {} bytes, rows {}",
            chrome(
                &ui,
                "new TextEncoder().encode(JSON.stringify(S.forest)).length"
            )?,
            chrome(&ui, "document.querySelectorAll('.forest-row').length")?
        );
        println!(
            "map items {}, DOM nodes {}",
            chrome(&ui, "S.forest.canopy?.items.length ?? 0")?,
            chrome(&ui, "$('forest-canopy').querySelectorAll('*').length")?
        );
        if std::env::var_os("FOREST_PROFILE").is_some() {
            println!(
                "paint profile {} completion {}",
                chrome(&ui, "paintProfile")?,
                chrome(&ui, "painted")?
            );
        }
        if round == 0 && std::env::var_os("FOREST_FIND").is_some() {
            for (label, letter, query) in [("first-letter", "a", "a"), ("next-letter", "g", "ag")] {
                chrome(
                    &ui,
                    &format!(
                        r#"(() => {{ globalThis.painted=null;
                    const check=()=>{{ if(S.forest?.query!=={query:?} || !(document.querySelector('#forest-list .forest-row') || !$('forest-empty').hidden)) {{requestAnimationFrame(check);return;}}
                    requestAnimationFrame(()=>requestAnimationFrame(()=>{{painted=performance.timeOrigin+performance.now();}})); }};
                    requestAnimationFrame(check);return true; }})()"#
                    ),
                )?;
                let wall = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs_f64()
                    * 1000.;
                let injection = Instant::now();
                xdo(&["key", "--clearmodifiers", letter])?;
                let injection_ms = injection.elapsed().as_secs_f64() * 1000.;
                wait("Find paint", || {
                    chrome(&ui, "painted!==null").is_ok_and(|v| v == true)
                })?;
                println!(
                    "find {label} {:.3} ms (input {:.3} ms)",
                    chrome(&ui, "painted")?.as_f64().unwrap() - wall,
                    injection_ms
                );
            }
            key("Escape")?;
        }
        if std::env::var_os("FOREST_ZOOM").is_some() {
            for (label, from, to, button) in [
                ("middle-near", "middle", "near", "4"),
                ("near-middle", "near", "middle", "5"),
                ("middle-far", "middle", "far", "5"),
                ("far-middle", "far", "middle", "4"),
                ("near-wheel", "near", "near", "4"),
            ] {
                click_chrome(&ui, &format!("#canopy-zoom [data-level={from}]"))?;
                wait("zoom setup", || {
                    chrome(&ui, "mapView.drawn").is_ok_and(|v| v == from)
                })?;
                let factor = match label {
                    "middle-near" => 1.54,
                    "near-middle" => 1.56,
                    "middle-far" => 0.81,
                    "far-middle" => 0.79,
                    _ => 1.9,
                };
                chrome(
                    &ui,
                    &format!("(zoomAround(focusedPoint(),{factor},true),true)"),
                )?;
                pump(100);
                hover_chrome(&ui, "#forest-canopy [data-current=true][data-page]")?;
                chrome(
                    &ui,
                    &format!(
                        r#"(()=>{{globalThis.painted=null;const previous=mapView.k;const check=()=>{{if(mapView.drawn!=={to:?} || mapView.k===previous || mapView.asked){{requestAnimationFrame(check);return;}} requestAnimationFrame(()=>requestAnimationFrame(()=>{{painted=performance.timeOrigin+performance.now()}}));}};requestAnimationFrame(check);return true;}})()"#
                    ),
                )?;
                let wall = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs_f64()
                    * 1000.;
                let injection = Instant::now();
                xdo(&["click", button])?;
                let input = injection.elapsed().as_secs_f64() * 1000.;
                wait("wheel paint", || {
                    chrome(&ui, "painted!==null").is_ok_and(|v| v == true)
                })?;
                println!(
                    "zoom {label} {:.3} ms (input {input:.3} ms)",
                    chrome(&ui, "painted")?.as_f64().unwrap() - wall
                );
            }
        }
        key("Escape")?;
        pump(500);
    }
    if std::env::var_os("FOREST_BIG").is_some() {
        key("ctrl+e")?;
        wait("large map", || visible(&ui, "#forest-canopy [data-page]"))?;
        click_chrome(&ui, "#canopy-zoom [data-level=near]")?;
        wait("large Near", || {
            chrome(&ui, "mapView.drawn").is_ok_and(|v| v == "near")
        })?;
        for _ in 0..8 {
            if chrome(&ui, "!!document.activeElement.closest('.canopy-item')")? == true {
                break;
            }
            key("Tab")?;
        }
        assert_eq!(
            chrome(&ui, "!!document.activeElement.closest('.canopy-item')")?,
            true
        );
        key("Left")?;
        let target = chrome(
            &ui,
            "canopyValue().items[+document.activeElement.dataset.index].go",
        )?
        .as_str()
        .unwrap()
        .to_owned();
        key("Return")?;
        wait("large map keyboard activation", || {
            !visible(&ui, "#forest")
                && ui
                    .snapshot()
                    .is_ok_and(|s| s.current.is_some_and(|id| id.to_string() == target))
        })?;
        key("ctrl+e")?;
        wait("large reopen", || {
            visible(&ui, "#forest-canopy [data-page]")
        })?;
        key("Menu")?;
        click_chrome(&ui, "[data-node-action=copy]")?;
        assert!(visible(&ui, "#forest"));
        let (x, y, w, h) = rect(&ui, "#forest-canopy")?;
        let x = (x + w - 80.) as i32;
        let y = (y + h - 20.) as i32;
        move_to(x, y)?;
        xdo(&["mousedown", "1"])?;
        wait("large held button", || {
            chrome(&ui, "canopy.state().buttons").is_ok_and(|v| v == 1)
        })?;
        for step in 1..=8 {
            move_to(x + step * 5, y)?;
        }
        xdo(&["mouseup", "1"])?;
        pump(100);
        assert!(visible(&ui, "#forest"));
        println!("PASS 500-page tree accepts native keys, activation, menus and stepped drag after all three zooms.");
    }
    ui.window.close();
    Ok(())
}
