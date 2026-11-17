use anyhow::Result;
use canopy::ui::Ui;
use gtk::prelude::*;
#[path = "../tests/common/mod.rs"]
mod common;
use common::*;
fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::copy(
        std::env::args().nth(1).unwrap(),
        dir.path().join("forest.sqlite3"),
    )?;
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 900);
    ui.window.show_all();
    ready(&ui)?;
    pump(500);
    chrome(
        &ui,
        &format!(
            "globalThis.profiling = {}",
            std::env::var_os("SURFACE_PROFILE").is_some()
        ),
    )?;
    chrome(
        &ui,
        r#"(() => {
      globalThis.timings=[];
      for (const name of (profiling ? ['renderStart','renderForest','patchChildren','fitForest'] : [])) {
        const f=window[name]; window[name]=(...a)=>{const t=performance.now();const r=f(...a);timings.push([name,performance.now()-t]);return r;};
      }
      globalThis.measure=kind=>{
        globalThis.started=performance.now();globalThis.painted=null;
        const observer=new MutationObserver(()=>{
          const el=document.querySelector(kind==='start'?'#start-content':'#forest-nodes');
          if (!el.children.length || document.querySelector(kind==='start'?'#start':'#forest').hidden) return;
          observer.disconnect();requestAnimationFrame(()=>requestAnimationFrame(()=>{painted=performance.now()-started;}));
        });
        observer.observe(document.body,{subtree:true,childList:true,attributes:true});
        if(kind==='start') canopy.newTree(); else canopy.toggleForest();
        return true;
      };return true;
    })()"#,
    )?;
    for round in 0..2 {
        chrome(&ui, "measure('start')")?;
        wait("start painted", || {
            chrome(&ui, "painted!==null").is_ok_and(|v| v == true)
        })?;
        println!("start {round} {}", chrome(&ui, "painted")?);
        pump(2200);
        chrome(&ui, "(() => { uncover(); return true; })()")?;
        pump(300);
        chrome(&ui, "measure('forest')")?;
        wait("forest painted", || {
            chrome(&ui, "painted!==null").is_ok_and(|v| v == true)
        })?;
        println!("forest {round} {}", chrome(&ui, "painted")?);
        println!("profile {}", chrome(&ui, "timings")?);
        chrome(&ui, "(() => { canopy.toggleForest(); return true; })()")?;
        pump(400);
    }
    Ok(())
}
