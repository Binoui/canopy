use anyhow::Result;
use canopy::{
    address::web_url,
    browser::{NavigationTarget, SourceContext, TreeId, VisitId},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use webkit2gtk::WebViewExt;
mod common;
use common::*;

fn tree(ui: &Ui, root: VisitId) -> TreeId {
    ui.pages
        .read(|b| b.tree_states())
        .unwrap()
        .into_iter()
        .find(|t| t.root.root() == root)
        .unwrap()
        .root
}

fn rename(ui: &Ui, root: VisitId, name: &str) {
    ui.dispatch(ChromeIntent::RenameTree(tree(ui, root), name.into()));
    pump(250);
}

fn press(ui: &Ui, selector: &str) -> Result<()> {
    hover_chrome(ui, selector)?;
    xdo(&["mousedown", "1"])?;
    pump(80);
    Ok(())
}

fn held_back(ui: &Ui) -> Result<()> {
    wait("a frame is held back while the button is down", || {
        chrome(ui, "canopy.state().deferred.includes('present')").is_ok_and(|held| held == true)
    })
}

fn release() -> Result<()> {
    xdo(&["mouseup", "1"])?;
    pump(600);
    Ok(())
}

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    std::env::set_var(
        "no_proxy",
        "localhost,127.0.0.1,127.0.0.2,127.0.0.3,127.0.0.4,127.0.0.5",
    );
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("0.0.0.0:0")?;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], listener.local_addr()?.port()));
    let release_icon = Arc::new(AtomicBool::new(false));
    let released = release_icon.clone();
    let icon =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 16, 16).unwrap();
    icon.fill(0x2050ffff);
    let png = icon.save_to_bufferv("png", &[])?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let released = released.clone();
            let png = png.clone();
            thread::spawn(move || {
                let mut request = [0; 4096];
                let Ok(n) = stream.read(&mut request) else {
                    return;
                };
                let request = String::from_utf8_lossy(&request[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                if path == "/delayed.png" {
                    while !released.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(5));
                    }
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", png.len());
                    let _ = stream.write_all(&png);
                    return;
                }
                let link = if path == "/with-icon" {
                    "<link rel='icon' href='/delayed.png'>"
                } else {
                    ""
                };
                let body = format!(
                    "<!doctype html><head>{link}<title>Reading {path}</title></head><body>fixture"
                );
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            });
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.resize(1100, 700);
    ui.window.show_all();
    ready(&ui)?;
    let mut roots = Vec::new();
    for name in ["alpha", "beta", "gamma"] {
        ui.pages.selected_page().navigate(
            web_url(&format!("http://{address}/{name}"))?,
            NavigationTarget::Root,
        )?;
        wait("root loaded", || {
            ui.snapshot()
                .unwrap()
                .current_visit()
                .is_some_and(|v| v.title == format!("Reading /{name}"))
                && !ui.pages.selected_page().native.is_loading()
        })?;
        roots.push(ui.snapshot()?.current.unwrap());
    }
    pump(500);
    let (alpha, beta, gamma) = (roots[0], roots[1], roots[2]);
    press(&ui, &format!(".tab[data-id='{alpha}']"))?;
    for title in ["First refresh", "Second refresh", "Last refresh"] {
        rename(&ui, gamma, title);
    }
    held_back(&ui)?;
    let held = !text(&ui, "#tab-row").contains("Last refresh");
    release()?;
    let tab = ui.snapshot()?.current == Some(alpha);
    println!("tab click across three refreshes: {tab}");
    wait("latest deferred title", || {
        text(&ui, "#tab-row").contains("Last refresh")
    })?;
    println!("latest deferred title appears after release: true");

    hover_chrome(&ui, &format!(".tab[data-id='{beta}']"))?;
    xdo(&["click", "3"])?;
    wait("tab menu", || {
        visible(&ui, "#popover [data-tab-action='save']")
    })?;
    press(&ui, "#popover [data-tab-action='save']")?;
    rename(&ui, gamma, "Menu refresh");
    held_back(&ui)?;
    release()?;
    let menu = ui.pages.read(|b| b.tree_states())?.iter().any(|t| {
        t.root.root() == beta && matches!(t.place(), canopy::browser::Place::Shelf { .. })
    });
    println!("menu row click across refresh: {menu}");
    key("Escape")?;
    if !menu {
        ui.dispatch(ChromeIntent::PlaceTree {
            tree: tree(&ui, beta),
            to: canopy::browser::Move::Shelf(canopy::browser::Slot::First),
        });
        pump(300);
    }

    key("ctrl+t")?;
    wait("pinned card", || {
        visible(&ui, &format!("[data-start=tree][data-id='{beta}']"))
    })?;
    assert_eq!(chrome(&ui, "(() => { const saved=S.frame.start; const visits=S.frame.visits; S.frame.visits={}; const cards=BANDS.map(b=>b(saved)).join(''); S.frame.visits=visits; uncover(); canopy.start({start:saved}); canopy.newTree(); return cards.includes('tree-card') && !document.querySelector('#start-content').children.length && S.frame.start===null; })()")?, true);
    wait("fresh pinned card after reopen", || {
        visible(&ui, &format!("[data-start=tree][data-id='{beta}']"))
    })?;
    println!("PASS start cards carry their own icon URLs, hidden replies are ignored, and reopen clears stale cards.");
    press(&ui, &format!("[data-start=tree][data-id='{beta}']"))?;
    rename(&ui, beta, "Card refresh");
    chrome(
        &ui,
        "(() => { globalThis.__iconURLs=[...new Set([...document.querySelectorAll('.fav img')].map(i=>new URL(i.src).searchParams.get('u')))].slice(0,2); __iconURLs.forEach(u=>canopy.icons(u,1000)); return true; })()",
    )?;
    held_back(&ui)?;
    release()?;
    assert_eq!(
        chrome(
            &ui,
            "__iconURLs.length === 2 && __iconURLs.every(u=>S.revisions.get(iconSite(u))===1000)"
        )?,
        true
    );
    let card = ui.snapshot()?.current == Some(beta);
    println!("pinned card click across refresh: {card}");
    rename(&ui, gamma, "Immediate refresh");
    wait("unpressed frame", || {
        text(&ui, "#tab-row").contains("Immediate refresh")
    })?;
    println!("unpressed frame appears immediately: true");
    press(&ui, &format!(".tab[data-id='{alpha}']"))?;
    rename(&ui, gamma, "Focus recovery");
    ui.pages.selected_page().native.grab_focus();
    wait("focus loss releases deferred frame", || {
        text(&ui, "#tab-row").contains("Focus recovery")
    })?;
    release()?;
    println!("focus loss releases deferred frame: true");

    press(&ui, &format!(".tab[data-id='{alpha}']"))?;
    rename(&ui, gamma, "Missed release recovery");
    let start = chrome_point(&ui, &format!(".tab[data-id='{alpha}']"))?;
    let end = page_point(&ui, 500, 500)?;
    for step in 1..=25 {
        move_to(
            start.0 + (end.0 - start.0) * step / 25,
            start.1 + (end.1 - start.1) * step / 25,
        )?;
        pump(60);
    }
    wait("native tab drag before outside release", || {
        chrome(&ui, "!!document.querySelector('.tab.dragging')")
            .is_ok_and(|dragging| dragging == true)
    })?;
    release()?;
    wait("native drag end releases deferred frame", || {
        text(&ui, "#tab-row").contains("Missed release recovery")
    })?;
    hover_chrome(&ui, "#settings-button")?;
    println!("outside release does not freeze chrome: true");
    assert!(
        tab && menu && card && held,
        "refresh must preserve every held click and defer painting"
    );
    pump(400);
    assert_eq!(
        chrome(&ui, "!('forest' in S.frame) && S.forest === null")?,
        true
    );
    chrome(&ui, "(() => { globalThis.__node=document.querySelector('#strip-nodes .node'); globalThis.__edge=document.querySelector('#strip-edges path'); canopy.present(S.frame); return true; })()")?;
    assert_eq!(chrome(&ui, "!strip.anim && __node === document.querySelector('#strip-nodes .node') && __edge === document.querySelector('#strip-edges path')")?, true);
    println!("PASS unchanged frame preserves strip nodes and edges without animation.");
    chrome(&ui, "(() => { globalThis.__mutations=0; globalThis.__observer=new MutationObserver(records=>__mutations+=records.length); __observer.observe(document.querySelector('#strip-canvas'), {attributes:true,childList:true,subtree:true}); return true; })()")?;
    pump(400);
    assert_eq!(chrome(&ui, "(() => { __observer.disconnect(); return __mutations === 0 && !strip.anim && !strip.raf; })()")?, true);
    println!("PASS rendering clock stops without mutating an unchanged strip.");
    chrome(&ui, "(() => { globalThis.__images=[...document.querySelectorAll('.fav img')]; globalThis.__sources=__images.map(i=>i.src); globalThis.__address=new URL(__images[0].src).searchParams.get('u'); canopy.icons(__address,12345); return true; })()")?;
    assert_eq!(chrome(&ui, "__images.every((img,i)=>(img.src!==__sources[i]) === (iconSite(new URL(__sources[i]).searchParams.get('u'))===iconSite(__address)))")?, true);
    chrome(&ui, "(() => { globalThis.__moved=JSON.parse(JSON.stringify(S.frame)); __moved.strip.nodes.forEach(n=>n.x+=10); canopy.present(__moved); globalThis.__shape=[__node.style.left,__node.style.top,__node.style.width,__node.style.height]; globalThis.__transform=__node.style.transform; return true; })()")?;
    pump(400);
    assert_eq!(chrome(&ui, "JSON.stringify(__shape) === JSON.stringify([__node.style.left,__node.style.top,__node.style.width,__node.style.height]) && __node.style.transform !== __transform")?, true);
    println!("PASS strip motion changes transforms while layout properties stay fixed.");
    println!("PASS favicon revision changes only matching sites.");
    chrome(&ui, "(() => { canopy.toggleTree(); return true; })()")?;
    wait("zoomed tree", || visible(&ui, "#tree-sheet"))?;
    chrome(&ui, "(() => { globalThis.__sheetImage=document.querySelector('#tree-sheet .fav img'); globalThis.__sheetURL=new URL(__sheetImage.src).searchParams.get('u'); canopy.icons(__sheetURL,12346); return true; })()")?;
    assert_eq!(
        chrome(
            &ui,
            "new URL(__sheetImage.src).searchParams.get('r') === '12346'"
        )?,
        true
    );
    println!("PASS open zoomed-tree icon updates immediately.");
    chrome(&ui, "(() => { canopy.toggleForest(); return true; })()")?;
    wait("on-demand forest", || {
        chrome(&ui, "S.forest?.view === 'canopy'").is_ok_and(|v| v == true)
    })?;
    assert_eq!(chrome(&ui, "window.innerHeight === S.windowHeight && document.querySelector('#forest').getBoundingClientRect().height > 94")?, true);
    chrome(&ui, "(() => { globalThis.__forestNode=document.querySelector('#forest-canopy [data-id]'); canopy.toggleForest(); return true; })()")?;
    pump(100);
    chrome(&ui, "(() => { canopy.toggleForest(); return true; })()")?;
    wait("unchanged forest reopened", || {
        visible(&ui, "#forest-canopy [data-id]")
    })?;
    assert_eq!(
        chrome(
            &ui,
            "__forestNode === document.querySelector('#forest-canopy [data-id]')"
        )?,
        true
    );
    println!("PASS forest first appears at window height and unchanged reopen keeps its nodes.");
    rename(&ui, gamma, "Forest changed while open");
    wait("forest change", || {
        text(&ui, "#forest-canopy")
            .to_lowercase()
            .contains("forest changed while open")
    })?;
    chrome(&ui, "(() => { canopy.toggleForest(); return true; })()")?;
    rename(&ui, gamma, "Forest closed again");
    wait("closed forest frame", || {
        chrome(&ui, "!('forest' in S.frame) && S.forest === null").is_ok_and(|v| v == true)
    })?;
    println!("PASS forest is requested on open, follows changes, and leaves closed frames.");

    for (index, surface) in ["strip", "list", "tree", "forest"].into_iter().enumerate() {
        chrome(&ui, "(() => { uncover(); return true; })()")?;
        release_icon.store(false, Ordering::SeqCst);
        let origin = format!("http://127.0.0.{}:{}", index + 2, address.port());
        for page in 0..5 {
            let target = if page == 0 {
                NavigationTarget::Root
            } else {
                NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap())
            };
            let path = if page == 4 {
                "with-icon".to_owned()
            } else {
                format!("page-{page}")
            };
            ui.pages
                .selected_page()
                .navigate(web_url(&format!("{origin}/{path}"))?, target)?;
            wait("icon fixture page", || {
                ui.snapshot()
                    .unwrap()
                    .current_visit()
                    .is_some_and(|v| v.url.as_str() == format!("{origin}/{path}"))
            })?;
        }
        let parent = ui.snapshot()?.current.unwrap();
        canopy::browser::Browser::open(&directory.path().join("forest.sqlite3"))?.queue_link(
            parent,
            web_url(&format!("{origin}/waiting"))?,
            SourceContext {
                page_url: web_url(&format!("{origin}/with-icon"))?,
                citation_url: None,
                link_text: "Waiting".into(),
                excerpt: String::new(),
            },
        )?;
        rename(&ui, gamma, "Icon fixture refresh");
        match surface {
            "list" => {
                chrome(&ui, "(() => { showList(strip.target.nodes.find(n=>n.kind==='run').id); return true; })()")?;
            }
            "tree" => {
                chrome(&ui, "(() => { canopy.toggleTree(); return true; })()")?;
            }
            "forest" => {
                chrome(&ui, "(() => { canopy.toggleForest(); return true; })()")?;
            }
            _ => {}
        }
        pump(250);
        let start = Instant::now();
        release_icon.store(true, Ordering::SeqCst);
        let selector = match surface {
            "strip" => "#strip-nodes",
            "list" => "#strip-list",
            "tree" => "#tree-sheet",
            _ => "#forest-canopy",
        };
        let check = format!("(() => {{ const shown=[...document.querySelectorAll('{selector} .fav img')]; const images=[...document.querySelectorAll('.fav img')].filter(i=>new URL(i.src).searchParams.get('u').startsWith('{origin}/')); return shown.length >= 3 && images.length >= 3 && images.every(i=>i.complete && i.naturalWidth>0 && i.style.visibility!=='hidden'); }})()");
        wait("all site fallback images loaded", || {
            chrome(&ui, &check).is_ok_and(|v| v == true)
        })?;
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{surface} icon update exceeded one second"
        );
        println!(
            "PASS delayed site favicon refreshes {surface}, including borrowed icons, in {:?}.",
            start.elapsed()
        );
        if let Some(dir) = std::env::var_os("CANOPY_REFRESH_SHOTS").map(std::path::PathBuf::from) {
            std::fs::create_dir_all(&dir)?;
            for theme in ["dark", "light"] {
                chrome(&ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
                pump(200);
                assert!(std::process::Command::new("import")
                    .args(["-window", "root"])
                    .arg(dir.join(format!("{surface}-{theme}.png")))
                    .status()?
                    .success());
            }
        }
    }
    Ok(())
}
