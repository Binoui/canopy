use anyhow::{bail, Result};
use canopy::{
    browser::{Browser, ThemeMode},
    chrome::{ChromeFrame, Favicons, HtmlChrome},
    forest::ForestProjection,
};
use gtk::prelude::*;
use javascriptcore::ValueExt;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use webkit2gtk::WebViewExt;
fn pump() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
    std::thread::sleep(Duration::from_millis(1));
}
fn eval(chrome: &HtmlChrome, script: &str) -> Result<serde_json::Value> {
    let result = Rc::new(RefCell::new(None));
    let out = result.clone();
    chrome
        .native()
        .evaluate_javascript(script, None, None, None::<&gio::Cancellable>, move |v| {
            *out.borrow_mut() = Some(v.map(|v| v.to_str().to_string()))
        });
    let deadline = Instant::now() + Duration::from_secs(30);
    while result.borrow().is_none() {
        if Instant::now() > deadline {
            bail!("eval timeout");
        }
        pump();
    }
    let value = result.borrow_mut().take().unwrap()?;
    Ok(serde_json::from_str(&value)?)
}
fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let b = Browser::open(std::path::Path::new(&path))?;
    if std::env::args().any(|a| a == "--init") {
        return Ok(());
    }
    canopy::init_desktop()?;
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_default_size(1240, 900);
    let fixed = gtk::Fixed::new();
    window.add(&fixed);
    window.show_all();
    let context = webkit2gtk::WebContext::default().unwrap();
    use webkit2gtk::WebContextExt;
    let icons = Favicons::new(&context.favicon_database().unwrap());
    let chrome = HtmlChrome::mount(&fixed, icons)?;
    chrome.resize(1240., 900.);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !chrome.is_ready() {
        if Instant::now() > deadline {
            bail!("ready timeout");
        }
        pump();
    }
    let phases = std::env::var_os("FRAME_PROBE_PHASES").is_some();
    let title = std::env::var_os("FRAME_PROBE_TITLE").is_some();
    if phases || title {
        eval(
            &chrome,
            r#"(()=>{
      const raf = requestAnimationFrame;
      globalThis.trace = null;
      window.requestAnimationFrame = cb => {
        if (trace) trace.rafRequests++;
        return raf(t => { if (trace) trace.rafCallbacks++; cb(t); });
      };
      const post = window.ipc.postMessage.bind(window.ipc);
      window.ipc.postMessage = msg => { if (trace) trace.messages.push({at:performance.now(), message:msg}); return post(msg); };
      const present = canopy.present;
      canopy.present = f => {
        globalThis.frame=f; globalThis.done=false;
        globalThis.trace={received:performance.now(), rafRequests:0, rafCallbacks:0, messages:[], incoming:[{name:"present",at:performance.now()}]};
        if (globalThis.changeTitle) f.tabs.find(t => t.active).title = `Visible update ${globalThis.sequence++}`;
        present(f);
        trace.applied=performance.now();
        requestAnimationFrame(t => {
          trace.first=performance.now(); trace.firstTick=t;
          requestAnimationFrame(t => {
            trace.second=performance.now(); trace.secondTick=t; globalThis.done=true;
          });
        });
      };
      for (const name of ['start', 'forest', 'strip', 'viewport', 'icons']) {
        const call = canopy[name];
        canopy[name] = (...args) => {
          if (trace) trace.incoming.push({name, at:performance.now()});
          return call(...args);
        };
      }
      return 'true';
    })()"#,
        )?;
    } else {
        eval(&chrome, "(()=>{const p=canopy.present; canopy.present=f=>{globalThis.frame=f;globalThis.done=false;p(f);requestAnimationFrame(()=>requestAnimationFrame(()=>globalThis.done=true));};return 'true';})()")?;
    }
    if title {
        eval(
            &chrome,
            "globalThis.changeTitle=true;globalThis.sequence=0; 'true'",
        )?;
    }
    let idle_ms = std::env::var("FRAME_PROBE_IDLE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    for i in 0..7 {
        let frame = ChromeFrame {
            forest: ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)?,
            bookmarks: b.bookmarks()?,
            declined: b.declined()?,
            loaded: Default::default(),
            live: Default::default(),
            files: vec![],
            capture_enabled: false,
            theme: ThemeMode::Light,
        };
        if title {
            use std::io::Write;
            println!("begin {i}");
            std::io::stdout().flush()?;
        }
        let start = Instant::now();
        chrome.present(frame);
        let build_ms = start.elapsed().as_secs_f64() * 1000.;
        loop {
            if eval(&chrome, "JSON.stringify(globalThis.done)")? == true {
                break;
            }
            if start.elapsed() > Duration::from_secs(30) {
                bail!("paint timeout");
            }
        }
        let ms = start.elapsed().as_secs_f64() * 1000.;
        println!(
            "sample {i} bytes {} ms {ms:.3} rust_ms {build_ms:.3}",
            eval(
                &chrome,
                "JSON.stringify(new TextEncoder().encode(JSON.stringify(globalThis.frame)).length)"
            )?
        );
        if phases || title {
            println!(
                "phase {}",
                eval(&chrome, "JSON.stringify(globalThis.trace)")?
            );
        }
        let until = Instant::now() + Duration::from_millis(idle_ms);
        while Instant::now() < until {
            pump();
        }
    }
    Ok(())
}
