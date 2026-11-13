use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use canopy::{address::web_url, browser::VisitState, ui::Ui};
use gdk::prelude::*;
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::Value;
use webkit2gtk::WebViewExt;

fn main() -> Result<()> {
    gtk::init().context("Ce test nécessite Xvfb avec un écran 1440x1000.")?;
    let fixture = Fixture::start()?;
    let directory = tempfile::tempdir()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ui.hide_address();
    pump(300);
    ui.engine
        .navigate(web_url(&fixture.url("/discussion"))?, true)?;
    loaded(&ui, "/discussion")?;
    let source_id = ui.snapshot()?.current.unwrap();
    let original_url = ui.snapshot()?.current_visit().unwrap().url.clone();

    pump(500);
    assert_eq!(
        ui.snapshot()?.visits.len(),
        1,
        "a page-owned timer click must not queue links"
    );
    click(&ui, "one", 1, true)?;
    wait("premier lien en attente", || queued(&ui) == 1)?;
    click(&ui, "two", 2, false)?;
    wait("deuxième lien en attente", || queued(&ui) == 2)?;
    click(&ui, "three", 1, false)?;
    wait("troisième lien en attente", || queued(&ui) == 3)?;
    wait("contexte des commentaires", || {
        ui.snapshot()
            .unwrap()
            .visits
            .iter()
            .filter(|v| v.state == VisitState::Queued)
            .all(|v| {
                v.source.as_ref().is_some_and(|s| {
                    s.excerpt.contains("commentaire source")
                        && s.citation_url
                            .as_ref()
                            .is_some_and(|u| u.fragment() == Some("comment-1"))
                })
            })
    })?;
    let snapshot = ui.snapshot()?;
    assert_eq!(snapshot.current, Some(source_id));
    assert!(snapshot
        .visits
        .iter()
        .skip(1)
        .all(|v| v.parent == Some(source_id)));
    assert_eq!(
        fixture.hits("/article-a") + fixture.hits("/article-b") + fixture.hits("/article-c"),
        0
    );
    assert_eq!(
        ui.engine.native.uri().unwrap().as_str(),
        original_url.as_str()
    );
    println!("PASS Ctrl-click, middle-click and target=_blank queue three contextual siblings without fetching or moving.");

    let article = snapshot
        .visits
        .iter()
        .find(|v| v.url.path() == "/article-a")
        .unwrap()
        .id;
    ui.engine.restore(article)?;
    loaded(&ui, "/article-a")?;
    wait("capture automatique du texte", || {
        !ui.search_hits("amber otter cache").unwrap().is_empty()
    })?;
    let saved = ui
        .search_hits("amber otter cache")?
        .into_iter()
        .find(|hit| hit.visit == article)
        .unwrap();
    assert_eq!(queued(&ui), 2);
    assert_eq!(ui.snapshot()?.visits.len(), 4);
    assert_eq!(ui.snapshot()?.current, Some(article));
    assert!(ui.search_hits("INPUT_SECRET")?.is_empty());
    assert!(ui.search_hits("TEXTAREA_SECRET")?.is_empty());
    assert!(ui.search_hits("EDITABLE_SECRET")?.is_empty());
    assert!(ui.search_hits("HIDDEN_LOGIN_SECRET")?.is_empty());
    assert!(ui.search_hits("SELECT_SECRET")?.is_empty());
    assert!(saved
        .source
        .as_ref()
        .unwrap()
        .excerpt
        .contains("commentaire source"));
    assert!(saved.captured_at.is_some());
    println!("PASS queued activation reuses its visit and captures article text; hidden login modal and all controls are excluded.");
    eval(&ui, "document.querySelector('main').insertAdjacentHTML('beforeend','<p>mossy lantern delayed article text</p>'); JSON.stringify(true)")?;
    wait("texte arrivé après Finished", || {
        !ui.search_hits("mossy lantern delayed").unwrap().is_empty()
    })?;
    println!("PASS a bounded follow-up captures article text arriving after Finished.");
    pump(200);
    if let Some(window) = ui.window.window() {
        let allocation = ui.window.allocation();
        let image = window
            .pixbuf(0, 0, allocation.width(), allocation.height())
            .context("capture impossible")?;
        image.savev("/tmp/canopy-mvp2.png", "png", &[])?;
    }

    let memory_search = named(&ui.window, "memory-search")
        .downcast::<gtk::SearchEntry>()
        .unwrap();
    memory_search.set_text("amber otter cache");
    wait("recherche de la capture", || {
        find_named(ui.window.upcast_ref(), "search-hit").is_some()
    })?;
    named(&ui.window, "search-hit")
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    wait("lecteur ouvert", || reader().is_some())?;
    let opened_reader = reader().unwrap();
    named(&opened_reader, "capture-restore-source")
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    wait("source restaurée", || {
        ui.snapshot().unwrap().current == Some(source_id) && reader().is_none()
    })?;
    loaded(&ui, "/discussion")?;
    assert!(memory_search.text().is_empty());
    assert_eq!(
        named(&ui.window, "navigation-mode")
            .downcast::<gtk::Stack>()
            .unwrap()
            .visible_child_name()
            .as_deref(),
        Some("tree")
    );
    assert_eq!(queued(&ui), 2);
    assert_eq!(ui.snapshot()?.current, Some(source_id));
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url, original_url);
    println!("PASS Source after phrase-search closes the reader, clears the filter and reveals the original parent plus two queued siblings.");

    click(&ui, "hash", 1, true)?;
    wait("fragment en attente", || queued(&ui) == 3)?;
    let fragment = ui
        .snapshot()?
        .visits
        .into_iter()
        .find(|visit| {
            visit.state == VisitState::Queued && visit.url.fragment() == Some("comment-1")
        })
        .unwrap()
        .id;
    ui.engine.restore(fragment)?;
    wait("activation du fragment", || {
        ui.snapshot().unwrap().current == Some(fragment) && !ui.engine.native.is_loading()
    })?;
    assert_eq!(queued(&ui), 2);
    ui.engine.restore(source_id)?;
    wait("retour au parent après fragment", || {
        ui.snapshot().unwrap().current == Some(source_id) && !ui.engine.native.is_loading()
    })?;
    ui.engine.open_citation(article)?;
    wait("commentaire ouvert par action native", || {
        ui.snapshot().unwrap().current_visit().is_some_and(|visit| {
            visit.parent == Some(source_id) && visit.url.fragment() == Some("comment-1")
        }) && !ui.engine.native.is_loading()
    })?;
    ui.engine.restore(source_id)?;
    wait("source inchangée après citation", || {
        ui.snapshot().unwrap().current == Some(source_id) && !ui.engine.native.is_loading()
    })?;
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url, original_url);
    println!("PASS queued fragments and native citation actions complete without changing the parent feed URL.");
    let before_anchor = ui.snapshot()?.visits.len();
    click(&ui, "hash", 1, false)?;
    wait("navigation dans le document", || {
        ui.engine
            .native
            .uri()
            .is_some_and(|uri| uri.ends_with("#comment-1"))
    })?;
    assert_eq!(ui.snapshot()?.visits.len(), before_anchor);
    click(&ui, "one", 1, false)?;
    loaded(&ui, "/article-a")?;
    assert_eq!(ui.snapshot()?.visits.len(), before_anchor + 1);
    assert_eq!(
        ui.snapshot()?.current_visit().unwrap().parent,
        Some(source_id)
    );
    println!("PASS a same-document anchor does not poison the following navigation.");

    let automatic = named(&ui.window, "automatic-capture")
        .downcast::<gtk::CheckButton>()
        .unwrap();
    automatic.set_active(false);
    ui.engine.restore(source_id)?;
    loaded(&ui, "/discussion")?;
    eval(&ui, "(() => {document.getElementById('remember').textContent='luminous notebook memory saved while automatic capture is off';const r=document.createRange();r.selectNodeContents(document.getElementById('remember'));const s=getSelection();s.removeAllRanges();s.addRange(r);return JSON.stringify(true)})()")?;
    named(&ui.window, "remember-selection")
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    wait("sélection mémorisée", || {
        ui.search_hits("luminous notebook memory")
            .unwrap()
            .iter()
            .any(|hit| hit.capture_id.is_some())
    })?;
    let selection = ui
        .search_hits("luminous notebook memory")?
        .into_iter()
        .find_map(|hit| hit.capture_id)
        .unwrap();
    assert!(selection > 0);
    eval(&ui, "(() => {document.getElementById('public-start').textContent='cerulean satellite ';const r=document.createRange();r.setStart(document.getElementById('public-start').firstChild,0);r.setEnd(document.getElementById('public-end').firstChild,13);const s=getSelection();s.removeAllRanges();s.addRange(r);return JSON.stringify(true)})()")?;
    named(&ui.window, "remember-selection")
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    wait("sélection partiellement publique", || {
        !ui.search_hits("cerulean satellite").unwrap().is_empty()
    })?;
    assert!(ui.search_hits("SOURCE_EDIT_SECRET")?.is_empty());
    eval(&ui, "(() => {const r=document.createRange();r.selectNodeContents(document.getElementById('editable'));const s=getSelection();s.removeAllRanges();s.addRange(r);return JSON.stringify(true)})()")?;
    named(&ui.window, "remember-selection")
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    pump(150);
    assert!(ui.search_hits("SOURCE_EDIT_SECRET")?.is_empty());
    println!("PASS explicit selections work with automatic capture disabled; mixed and editable ranges do not leak controls.");

    ui.engine
        .navigate(web_url(&fixture.url("/no-capture"))?, true)?;
    loaded(&ui, "/no-capture")?;
    pump(100);
    assert!(ui.search_hits("automatic off sentinel")?.is_empty());
    automatic.set_active(true);
    wait("capture réactivée", || {
        !ui.search_hits("automatic off sentinel").unwrap().is_empty()
    })?;
    ui.engine.navigate(web_url(&fixture.url("/login"))?, true)?;
    loaded(&ui, "/login")?;
    pump(150);
    assert!(ui.search_hits("VISIBLE_LOGIN_SENTINEL")?.is_empty());
    automatic.set_active(false);
    println!("PASS capture setting controls new saves and a rendered password form suppresses automatic capture.");

    let expected = ui.snapshot()?;
    ui.window.close();
    drop(ui);
    pump(100);
    drop(fixture);
    let reopened = Ui::new(directory.path())?;
    reopened.window.show_all();
    reopened.hide_address();
    assert_eq!(reopened.snapshot()?, expected);
    assert!(!reopened.engine.capture_enabled()?);
    let hits = reopened.search_hits("amber otter cache")?;
    assert!(hits.iter().any(|hit| hit.capture_id == saved.capture_id));
    let entry = named(&reopened.window, "memory-search")
        .downcast::<gtk::SearchEntry>()
        .unwrap();
    entry.set_text("amber otter cache");
    wait("résultats de recherche visibles", || {
        find_named(reopened.window.upcast_ref(), "search-hit").is_some()
    })?;
    pump(200);
    named(&reopened.window, "search-hit")
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    wait("lecteur de capture hors ligne", || reader().is_some())?;
    let reader = reader().unwrap();
    let text = named(&reader, "capture-text")
        .downcast::<gtk::TextView>()
        .unwrap();
    let buffer = text.buffer().unwrap();
    assert!(buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .unwrap()
        .contains("amber otter cache"));
    assert!(!text.is_editable());
    assert!(named(&reader, "capture-source")
        .downcast::<gtk::Label>()
        .unwrap()
        .text()
        .contains("commentaire source"));
    assert_eq!(queued(&reopened), 2);
    assert_eq!(reopened.snapshot()?, expected);
    pump(200);
    if let Some(window) = reader.window() {
        let allocation = reader.allocation();
        let image = window
            .pixbuf(0, 0, allocation.width(), allocation.height())
            .context("capture du lecteur impossible")?;
        image.savev("/tmp/canopy-mvp2-reader.png", "png", &[])?;
    }
    reader.close();
    entry.set_text("luminous notebook memory");
    pump(200);
    let selected = reopened.search_hits("luminous notebook memory")?;
    assert!(selected.iter().any(|hit| hit.capture_id == Some(selection)));
    reopened.window.close();
    println!("PASS restart offline: body-phrase search opens saved plaintext with source/date and two queued siblings; no page load is needed.");
    Ok(())
}

fn reader() -> Option<gtk::Window> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.widget_name() == "capture-reader")
}

fn queued(ui: &Ui) -> usize {
    ui.snapshot()
        .unwrap()
        .visits
        .iter()
        .filter(|visit| visit.state == VisitState::Queued)
        .count()
}

fn loaded(ui: &Ui, path: &str) -> Result<()> {
    wait(&format!("chargement {path}"), || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == path && !v.title.is_empty())
            && !ui.engine.native.is_loading()
    })
}

fn named(window: &impl IsA<gtk::Widget>, name: &str) -> gtk::Widget {
    find_named(window.as_ref(), name).unwrap_or_else(|| panic!("missing widget {name}"))
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

fn eval(ui: &Ui, script: &str) -> Result<Value> {
    let output = Rc::new(RefCell::new(None));
    let result = output.clone();
    ui.engine.native.evaluate_javascript(
        script,
        None,
        None,
        None::<&gio::Cancellable>,
        move |value| {
            *result.borrow_mut() = Some(value.map(|value| value.to_str().to_string()));
        },
    );
    wait("JavaScript fixture", || output.borrow().is_some())?;
    let text = output.borrow_mut().take().unwrap()?;
    Ok(serde_json::from_str(&text)?)
}

fn click(ui: &Ui, id: &str, button: u8, control: bool) -> Result<()> {
    let script = format!("(() => {{const r=document.getElementById({}).getBoundingClientRect();return JSON.stringify({{x:r.x+r.width/2,y:r.y+r.height/2}})}})()",serde_json::to_string(id)?);
    let point = eval(ui, &script)?;
    let (x, y) = ui
        .engine
        .native
        .translate_coordinates(
            &ui.window,
            point["x"].as_f64().unwrap() as i32,
            point["y"].as_f64().unwrap() as i32,
        )
        .context("position native de la webview")?;
    let (_, wx, wy) = ui.window.window().unwrap().origin();
    let search = std::process::Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "Canopy"])
        .output()?;
    let ids = String::from_utf8(search.stdout)?;
    let id = ids.lines().last().context("fenêtre X11")?;
    let mut command = std::process::Command::new("xdotool");
    command.args([
        "windowfocus",
        id,
        "mousemove",
        "--sync",
        &(x + wx).to_string(),
        &(y + wy).to_string(),
    ]);
    if control {
        command.args(["keydown", "Control_L"]);
    }
    command.args(["click", &button.to_string()]);
    if control {
        command.args(["keyup", "Control_L"]);
    }
    if !command.status()?.success() {
        bail!("xdotool click failed");
    }
    pump(80);
    Ok(())
}

fn wait(label: &str, condition: impl Fn() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(12);
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

fn pump(ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

struct Fixture {
    address: std::net::SocketAddr,
    stopped: Arc<AtomicBool>,
    requests: Arc<Mutex<BTreeMap<String, usize>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = stopped.clone();
        let counts = requests.clone();
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut bytes = [0; 8192];
                let n = stream.read(&mut bytes).unwrap_or(0);
                let request = String::from_utf8_lossy(&bytes[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                *counts.lock().unwrap().entry(path.to_owned()).or_default() += 1;
                let body = page(path);
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Ok(Self {
            address,
            stopped,
            requests,
            worker: Some(worker),
        })
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
    fn hits(&self, path: &str) -> usize {
        *self.requests.lock().unwrap().get(path).unwrap_or(&0)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn page(path: &str) -> String {
    let content = match path {
        "/discussion" => {
            r##"<main><h1>Une discussion à explorer</h1><article id="comment-1" data-permalink="/discussion#comment-1"><p>Un commentaire source explique pourquoi ces trois articles valent le détour.</p><a id="one" href="/article-a">Comprendre le cache</a><a id="two" href="/article-b">Une deuxième piste</a><a id="three" href="/article-c" target="_blank">Une troisième piste</a><p id="remember">violet comet selection kept for tomorrow</p></article><a id="hash" href="#comment-1">Revenir au commentaire</a><div id="mixed"><span id="public-start">public satellite </span><span contenteditable="true" id="editable">SOURCE_EDIT_SECRET</span><span id="public-end">public finish</span></div></main><script>if(!sessionStorage.getItem('probe-done')){sessionStorage.setItem('probe-done','1');setTimeout(()=>document.getElementById('three').click(),250)}</script>"##
        }
        "/article-a" => {
            r#"<main><h1>Comprendre le cache</h1><p>amber otter cache invalidation is the phrase worth remembering.</p><p>Ce texte doit rester disponible après la fermeture du navigateur.</p><input value="INPUT_SECRET"><textarea>TEXTAREA_SECRET</textarea><select><option>SELECT_SECRET</option></select><div contenteditable="true">EDITABLE_SECRET</div><div hidden><input type="password" value="HIDDEN_LOGIN_SECRET"></div></main>"#
        }
        "/article-b" => "<main><h1>Deuxième article</h1><p>Encore en attente.</p></main>",
        "/article-c" => "<main><h1>Troisième article</h1><p>Encore en attente.</p></main>",
        "/no-capture" => "<main><h1>Capture désactivée</h1><p>automatic off sentinel</p></main>",
        "/login" => {
            "<main><h1>Connexion</h1><p>VISIBLE_LOGIN_SENTINEL</p><input type='password'></main>"
        }
        _ => "<main><h1>Fixture</h1></main>",
    };
    format!("<!doctype html><html lang='fr'><meta charset='utf-8'><title>{path}</title><style>body{{font:18px system-ui;color:#294331;margin:34px}}a{{display:block;margin:16px 0}}article{{max-width:750px;padding:18px;background:#f2f5ed}}p{{line-height:1.6}}input,textarea,select{{max-width:100px}}</style>{content}</html>")
}
