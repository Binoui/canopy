use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use canopy::{
    address::web_url,
    browser::{NavigationTarget, VisitState},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

mod common;

use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let fixture = Fixture::start()?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    pump(300);
    ui.engine.navigate(
        web_url(&fixture.url("/discussion"))?,
        NavigationTarget::Root,
    )?;
    loaded(&ui, "/discussion")?;
    let source_id = ui.snapshot()?.current.unwrap();
    let original_url = ui.snapshot()?.current_visit().unwrap().url.clone();

    pump(500);
    assert_eq!(
        ui.snapshot()?.visits.len(),
        1,
        "a page-owned timer click must not queue links"
    );
    page_click(&ui, "one", 1, true)?;
    wait("first queued link", || queued(&ui) == 1)?;
    page_click(&ui, "two", 2, false)?;
    wait("second queued link", || queued(&ui) == 2)?;
    page_click(&ui, "three", 1, false)?;
    wait("third queued link", || queued(&ui) == 3)?;
    wait("comment context", || {
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
    wait("automatic text capture", || {
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
    wait("text arriving after Finished", || {
        !ui.search_hits("mossy lantern delayed").unwrap().is_empty()
    })?;
    println!("PASS a bounded follow-up captures article text arriving after Finished.");
    open_strip(&ui)?;
    move_to_widget(&ui, &node(&ui, article))?;
    let back = named(&ui.window, "card-source");
    wait("source context on the current page's card", || {
        back.is_visible()
    })?;
    assert!(label_texts(&named(&ui.window, "card"))
        .iter()
        .any(|text| text.contains("commentaire source")));
    pump(150);
    click(&ui, &back)?;
    wait("Back to source from the card", || {
        ui.snapshot().unwrap().current == Some(source_id) && !ui.engine.native.is_loading()
    })?;
    assert_eq!(queued(&ui), 2);
    ui.engine.restore(article)?;
    loaded(&ui, "/article-a")?;
    println!("PASS the current page's card shows its source excerpt and Back to source restores the parent.");
    save_window(&ui, "/tmp/canopy-chrome-reading.png")?;

    search_saved_text(&ui, "amber otter cache")?;
    wait("reader open", || reader().is_some())?;
    let opened_reader = reader().unwrap();
    named(&opened_reader, "capture-restore-source")
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    wait("source restored", || {
        ui.snapshot().unwrap().current == Some(source_id) && reader().is_none()
    })?;
    loaded(&ui, "/discussion")?;
    assert!(!named(&ui.window, "palette").is_visible());
    assert_eq!(queued(&ui), 2);
    for sibling in ui
        .snapshot()?
        .visits
        .iter()
        .filter(|v| v.state == VisitState::Queued)
    {
        wait("queued sibling in the strip", || {
            node(&ui, sibling.id).is_visible()
        })?;
    }
    assert_eq!(ui.snapshot()?.current, Some(source_id));
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url, original_url);
    println!("PASS Back to source after a palette phrase search closes the reader and the palette and shows the parent with its two queued siblings in the strip.");

    page_click(&ui, "hash", 1, true)?;
    wait("queued fragment", || queued(&ui) == 3)?;
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
    wait("fragment activation", || {
        ui.snapshot().unwrap().current == Some(fragment) && !ui.engine.native.is_loading()
    })?;
    assert_eq!(queued(&ui), 2);
    ui.engine.restore(source_id)?;
    wait("back to the parent after the fragment", || {
        ui.snapshot().unwrap().current == Some(source_id) && !ui.engine.native.is_loading()
    })?;
    ui.engine.open_citation(article)?;
    wait("comment opened by the native action", || {
        ui.snapshot().unwrap().current_visit().is_some_and(|visit| {
            visit.parent == Some(source_id) && visit.url.fragment() == Some("comment-1")
        }) && !ui.engine.native.is_loading()
    })?;
    ui.engine.restore(source_id)?;
    wait("source unchanged after the citation", || {
        ui.snapshot().unwrap().current == Some(source_id) && !ui.engine.native.is_loading()
    })?;
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url, original_url);
    println!("PASS queued fragments and native citation actions complete without changing the parent feed URL.");
    let before_anchor = ui.snapshot()?.visits.len();
    page_click(&ui, "hash", 1, false)?;
    wait("same-document navigation", || {
        ui.engine
            .native
            .uri()
            .is_some_and(|uri| uri.ends_with("#comment-1"))
    })?;
    assert_eq!(ui.snapshot()?.visits.len(), before_anchor);
    page_click(&ui, "one", 1, false)?;
    loaded(&ui, "/article-a")?;
    assert_eq!(ui.snapshot()?.visits.len(), before_anchor + 1);
    assert_eq!(
        ui.snapshot()?.current_visit().unwrap().parent,
        Some(source_id)
    );
    println!("PASS a same-document anchor does not poison the following navigation.");

    set_remember_pages(&ui, false)?;
    assert!(!ui.engine.capture_enabled()?);
    ui.engine.restore(source_id)?;
    loaded(&ui, "/discussion")?;
    eval(&ui, "(() => {document.getElementById('remember').textContent='luminous notebook memory saved while automatic capture is off';const r=document.createRange();r.selectNodeContents(document.getElementById('remember'));const s=getSelection();s.removeAllRanges();s.addRange(r);return JSON.stringify(true)})()")?;
    remember_selection(&ui)?;
    wait("selection remembered", || {
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
    remember_selection(&ui)?;
    wait("partly public selection", || {
        !ui.search_hits("cerulean satellite").unwrap().is_empty()
    })?;
    assert!(ui.search_hits("SOURCE_EDIT_SECRET")?.is_empty());
    eval(&ui, "(() => {const r=document.createRange();r.selectNodeContents(document.getElementById('editable'));const s=getSelection();s.removeAllRanges();s.addRange(r);return JSON.stringify(true)})()")?;
    remember_selection(&ui)?;
    pump(150);
    assert!(ui.search_hits("SOURCE_EDIT_SECRET")?.is_empty());
    println!("PASS explicit selections work with automatic capture disabled; mixed and editable ranges do not leak controls.");

    ui.engine.navigate(
        web_url(&fixture.url("/no-capture"))?,
        NavigationTarget::Root,
    )?;
    loaded(&ui, "/no-capture")?;
    pump(100);
    assert!(ui.search_hits("automatic off sentinel")?.is_empty());
    set_remember_pages(&ui, true)?;
    wait("capture turned back on", || {
        !ui.search_hits("automatic off sentinel").unwrap().is_empty()
    })?;
    ui.engine
        .navigate(web_url(&fixture.url("/login"))?, NavigationTarget::Root)?;
    loaded(&ui, "/login")?;
    pump(150);
    assert!(ui.search_hits("VISIBLE_LOGIN_SENTINEL")?.is_empty());
    set_remember_pages(&ui, false)?;
    println!("PASS capture setting controls new saves and a rendered password form suppresses automatic capture.");

    let expected = ui.snapshot()?;
    ui.window.close();
    drop(ui);
    pump(100);
    drop(fixture);
    let reopened = Ui::new(directory.path())?;
    reopened.window.show_all();
    pump(300);
    assert_eq!(reopened.snapshot()?, expected);
    assert!(!reopened.engine.capture_enabled()?);
    let hits = reopened.search_hits("amber otter cache")?;
    assert!(hits.iter().any(|hit| hit.capture_id == saved.capture_id));
    search_saved_text(&reopened, "amber otter cache")?;
    wait("offline reader", || reader().is_some())?;
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
            .context("reader capture")?;
        image.savev("/tmp/canopy-chrome-reader.png", "png", &[])?;
    }
    reader.close();
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
    wait(&format!("{path} loaded"), || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == path && !v.title.is_empty())
            && !ui.engine.native.is_loading()
    })
}

/// Types `query` into the palette and clicks its first saved-text result.
fn search_saved_text(ui: &Ui, query: &str) -> Result<()> {
    key("ctrl+k")?;
    let entry = named(&ui.window, "palette-entry")
        .downcast::<gtk::Entry>()
        .unwrap();
    wait("palette focus", || entry.has_focus())?;
    type_text(query)?;
    wait("saved text result", || {
        find_named(ui.window.upcast_ref(), "palette-saved").is_some()
    })?;
    pump(150);
    click(ui, &named(&ui.window, "palette-saved"))
}

fn open_memory_menu(ui: &Ui) -> Result<gtk::Popover> {
    let popover = popover_of(ui, "memory-menu");
    if !popover.is_visible() {
        click(ui, &named(&ui.window, "memory-menu"))?;
        wait("Memory menu", || popover.is_visible())?;
        pump(150);
    }
    Ok(popover)
}

fn set_remember_pages(ui: &Ui, on: bool) -> Result<()> {
    let popover = open_memory_menu(ui)?;
    let switch = named(&popover, "automatic-capture")
        .downcast::<gtk::Switch>()
        .unwrap();
    if switch.is_active() != on {
        click(ui, &switch)?;
        wait("Remember pages toggled", || {
            ui.engine.capture_enabled().unwrap() == on
        })?;
    }
    key("Escape")?;
    wait("Memory menu closed", || !popover.is_visible())
}

fn remember_selection(ui: &Ui) -> Result<()> {
    let popover = open_memory_menu(ui)?;
    click(ui, &named(&popover, "remember-selection"))?;
    wait("Memory menu closed", || !popover.is_visible())
}

fn page_click(ui: &Ui, id: &str, button: u8, control: bool) -> Result<()> {
    let (x, y) = element_point(ui, id)?;
    focus_window()?;
    move_to(x, y)?;
    let button = button.to_string();
    let mut args = vec![];
    if control {
        args.extend(["keydown", "Control_L"]);
    }
    args.extend(["click", button.as_str()]);
    if control {
        args.extend(["keyup", "Control_L"]);
    }
    xdo(&args)?;
    pump(80);
    Ok(())
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
