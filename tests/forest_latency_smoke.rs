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
        for t in 0..2000 {
            for p in 0..10 {
                let n = t * 10 + p;
                tx.execute("INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?,?,?,?,?,?)",rusqlite::params![id(n), (p>0).then(|| id(n-1)), format!("http://site{}.localhost:9/p/{t}/{p}",p%4),format!("Project {t} page {p}"),1_790_000_000_000i64+n as i64,1_790_000_000_000i64+n as i64])?;
            }
            tx.execute("INSERT INTO tree_state(root_id,last_visit_id,name,place,position,pinned,placed_at) VALUES(?,?,?,?,?,?,?)",rusqlite::params![id(t*10),id(t*10+9),format!("Project {t}"),if t<4 {"tab"} else if t==4 {"shelf"} else {"history"},(t<5).then_some(t as i64),t<4,1_790_000_000_000i64+t as i64])?;
        }
        tx.execute("UPDATE session SET current_id=?", [id(9)])?;
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
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 900);
    ui.window.show_all();
    ready(&ui)?;
    pump(1000);
    for round in 0..2 {
        focus_window()?;
        chrome(
            &ui,
            r#"(() => {
          globalThis.painted=null;
          const check=()=>{
            const forest=document.querySelector('#forest');
            const ready=!forest.hidden && S.forest?.view==='history' && document.querySelectorAll('#forest-list .forest-row').length===20 && S.forest.count;
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
        key("Escape")?;
        pump(500);
    }
    ui.window.close();
    Ok(())
}
