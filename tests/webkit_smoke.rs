use std::{
    cell::RefCell,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use canopy::{
    address::web_url,
    browser::{Snapshot, VisitState},
    ui::Ui,
};
use gdk::prelude::*;
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use webkit2gtk::WebViewExt;

fn main() -> Result<()> {
    gtk::init().context("Ce test nécessite un affichage. Utilise xvfb-run -a.")?;
    let fixture = Fixture::start()?;
    let directory = tempfile::tempdir()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ui.hide_address();
    pump(Duration::from_millis(350));

    let address = find_named(ui.window.upcast_ref(), "address")
        .context("champ adresse absent")?
        .downcast::<gtk::Entry>()
        .unwrap();
    ui.open_address(true);
    address.set_text(&fixture.url("/a"));
    address.emit_activate();
    loaded(&ui, "/a", 1)?;
    let a = ui.snapshot()?.current.unwrap();
    click(&ui, "b")?;
    loaded(&ui, "/b", 2)?;
    let b = ui.snapshot()?.current.unwrap();

    let outline = find_named(ui.window.upcast_ref(), "outline")
        .unwrap()
        .downcast::<gtk::TreeView>()
        .unwrap();
    outline.row_activated(
        &gtk::TreePath::from_string("0"),
        &outline.column(0).unwrap(),
    );
    loaded(&ui, "/a", 2)?;
    click(&ui, "c")?;
    loaded(&ui, "/c", 3)?;
    let c = ui.snapshot()?.current.unwrap();
    let snapshot = ui.snapshot()?;
    assert_eq!(
        snapshot
            .visits
            .iter()
            .filter(|v| v.parent == Some(a))
            .map(|v| v.id)
            .collect::<Vec<_>>(),
        vec![b, c]
    );
    println!("PASS vrais liens A → B, sélection A dans l'arbre, lien C ; B et C sont sœurs.");

    ui.engine.native.grab_focus();
    pump(Duration::from_millis(100));
    let output = std::process::Command::new("xdotool")
        .args(["search", "--name", "Canopy"])
        .output()?;
    let ids = String::from_utf8(output.stdout)?;
    let window_id = ids.lines().last().context("fenêtre X11 introuvable")?;
    let status = std::process::Command::new("xdotool")
        .args(["windowfocus", window_id, "key", "ctrl+l"])
        .status()?;
    assert!(status.success());
    wait("Ctrl+L depuis la page web", || address.has_focus())?;
    assert_eq!(address.text().as_str(), fixture.url("/c"));
    ui.hide_address();
    println!("PASS Ctrl+L ouvre le champ depuis la webview focalisée.");

    pump(Duration::from_millis(200));
    if let Some(window) = ui.window.window() {
        let allocation = ui.window.allocation();
        let pixbuf = window
            .pixbuf(0, 0, allocation.width(), allocation.height())
            .context("capture de fenêtre impossible")?;
        pixbuf.savev("/tmp/canopy-mvp1.png", "png", &[])?;
        println!("Capture réelle de la fenêtre /tmp/canopy-mvp1.png");
    }

    ui.engine.reload()?;
    wait("début du rechargement", || ui.engine.native.is_loading())?;
    loaded(&ui, "/c", 3)?;
    assert_eq!(ui.snapshot()?.current, Some(c));
    println!("PASS rechargement sans nouveau nœud.");

    ui.engine
        .navigate(web_url(&fixture.url("/redirect"))?, false)?;
    loaded(&ui, "/final", 4)?;
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url.path(), "/final");
    println!("PASS redirection HTTP 302 enregistrée une seule fois avec URL finale.");

    let before_failure = ui.snapshot()?;
    let unused_port = TcpListener::bind("127.0.0.1:0")?;
    let port = unused_port.local_addr()?.port();
    drop(unused_port);
    let failed_url = format!("http://127.0.0.1:{port}/unreachable");
    let notice = find_named(ui.window.upcast_ref(), "notice")
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    ui.engine.navigate(web_url(&failed_url)?, false)?;
    wait("échec de connexion", || {
        notice.is_visible()
            && notice.text().contains("Impossible de charger")
            && !ui.engine.native.is_loading()
    })?;
    pump(Duration::from_millis(150));
    assert_eq!(ui.snapshot()?, before_failure);
    println!("PASS échec réseau sans faux nœud ni changement de curseur.");

    ui.engine.restore(a)?;
    loaded(&ui, "/a", 4)?;
    trusted_click(&ui, "blank")?;
    wait("lien target=_blank en attente", || {
        ui.snapshot().unwrap().visits.len() == 5
    })?;
    let pending = ui
        .snapshot()?
        .visits
        .into_iter()
        .find(|visit| visit.state == VisitState::Queued)
        .context("queued target=_blank")?;
    assert_eq!(pending.parent, Some(a));
    assert_eq!(ui.snapshot()?.current, Some(a));
    ui.engine.restore(pending.id)?;
    loaded(&ui, "/d", 5)?;
    assert_eq!(ui.snapshot()?.current, Some(pending.id));
    println!("PASS lien target=_blank gardé en attente puis activé sans doublon.");

    ui.engine.navigate(web_url(&fixture.url("/b"))?, false)?;
    ui.engine.navigate(web_url(&fixture.url("/c"))?, true)?;
    loaded(&ui, "/c", 6)?;
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    println!("PASS requêtes rapprochées coalescées ; le dernier nouvel arbre reste racine.");

    let expected: Snapshot = ui.snapshot()?;
    ui.window.close();
    drop(ui);
    pump(Duration::from_millis(100));
    let reopened = Ui::new(directory.path())?;
    reopened.window.show_all();
    reopened.hide_address();
    assert_eq!(reopened.snapshot()?, expected);
    reopened.resume()?;
    loaded(&reopened, "/c", 6)?;
    assert_eq!(reopened.snapshot()?, expected);
    reopened.window.close();
    println!("PASS réouverture SQLite, URL rechargée, titres et branches inchangés.");
    Ok(())
}

fn loaded(ui: &Ui, path: &str, count: usize) -> Result<()> {
    wait(&format!("chargement {path} avec {count} visites"), || {
        let snapshot = ui.snapshot().unwrap();
        snapshot.visits.len() == count
            && snapshot
                .current_visit()
                .is_some_and(|visit| visit.url.path() == path && !visit.title.is_empty())
            && !ui.engine.native.is_loading()
    })
    .with_context(|| {
        format!(
            "état observé {:?}, URI native {:?}",
            ui.snapshot(),
            ui.engine.native.uri()
        )
    })
}

fn click(ui: &Ui, id: &str) -> Result<()> {
    let result = Rc::new(RefCell::new(None));
    let completed = result.clone();
    ui.engine.native.evaluate_javascript(
        &format!("document.getElementById('{id}').click()"),
        None,
        None,
        None::<&gio::Cancellable>,
        move |value| {
            *completed.borrow_mut() = Some(value.map(|_| ()));
        },
    );
    wait("exécution du clic sur lien", || result.borrow().is_some())?;
    let value = result.borrow_mut().take().unwrap();
    value?;
    Ok(())
}

fn trusted_click(ui: &Ui, id: &str) -> Result<()> {
    let result = Rc::new(RefCell::new(None));
    let completed = result.clone();
    let script = format!("(() => {{const r=document.getElementById({}).getBoundingClientRect();return JSON.stringify({{x:r.x+r.width/2,y:r.y+r.height/2}})}})()", serde_json::to_string(id)?);
    ui.engine.native.evaluate_javascript(
        &script,
        None,
        None,
        None::<&gio::Cancellable>,
        move |value| {
            *completed.borrow_mut() = Some(value.map(|value| value.to_str().to_string()));
        },
    );
    wait("coordonnées du lien", || result.borrow().is_some())?;
    let value: Value = serde_json::from_str(&result.borrow_mut().take().unwrap()?)?;
    let (x, y) = ui
        .engine
        .native
        .translate_coordinates(
            &ui.window,
            value["x"].as_f64().unwrap() as i32,
            value["y"].as_f64().unwrap() as i32,
        )
        .context("position de la webview")?;
    let (_, wx, wy) = ui.window.window().unwrap().origin();
    let output = std::process::Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "Canopy"])
        .output()?;
    let ids = String::from_utf8(output.stdout)?;
    let window = ids.lines().last().context("fenêtre X11")?;
    let status = std::process::Command::new("xdotool")
        .args([
            "windowfocus",
            window,
            "mousemove",
            "--sync",
            &(x + wx).to_string(),
            &(y + wy).to_string(),
            "click",
            "1",
        ])
        .status()?;
    assert!(status.success());
    Ok(())
}

fn wait(label: &str, condition: impl Fn() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        if condition() {
            return Ok(());
        }
        if Instant::now() > deadline {
            bail!("Délai dépassé pour {label}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn pump(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn find_named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.widget_name() == name {
        return Some(widget.clone());
    }
    widget
        .downcast_ref::<gtk::Container>()?
        .children()
        .iter()
        .find_map(|child| find_named(child, name))
}

struct Fixture {
    address: std::net::SocketAddr,
    stopped: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let handle = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buffer = [0; 8192];
                let size = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..size]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let response = if path == "/redirect" {
                    "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                } else {
                    let title = match path {
                        "/a" => "Départ de recherche",
                        "/b" => "Piste B",
                        "/c" => "Piste C",
                        "/d" => "Lien dans une autre fenêtre",
                        "/final" => "Destination après redirection",
                        _ => "Fixture",
                    };
                    let body = format!("<!doctype html><html lang='fr'><meta charset='utf-8'><title>{title}</title><style>body{{font:18px system-ui;max-width:720px;margin:64px auto;color:#294331}}a{{display:inline-block;margin:8px 12px 8px 0;color:#356a48}}h1{{font-weight:550}}p{{line-height:1.7;color:#667568}}</style><h1>{title}</h1><p>Une page de recherche pour explorer plusieurs pistes et retrouver le chemin parcouru.</p><a id='b' href='/b'>Explorer la piste B</a><a id='c' href='/c'>Explorer la piste C</a><a id='blank' href='/d' target='_blank'>Ouvrir une autre piste</a><a id='same' href='/a'>Revisiter cette URL</a></html>");
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Ok(Self {
            address,
            stopped,
            thread: Some(handle),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
