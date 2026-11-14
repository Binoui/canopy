use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::Result;
use canopy::{address::web_url, browser::NavigationTarget, ui::Ui};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

mod common;

use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    let config = directory.path().join("config");
    std::fs::create_dir_all(&config)?;
    std::fs::write(
        config.join("user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Saved files\"\n",
    )?;
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", &config);
    let downloads = home.join("Saved files");
    start(directory.path())?;
    let fixture = Fixture::start()?;
    let ui = Ui::new(&directory.path().join("data"))?;
    ui.window.show_all();
    ready(&ui)?;
    ui.engine
        .navigate(web_url(&fixture.url("/page"))?, NavigationTarget::Root)?;
    loaded(&ui, "/page")?;
    let page = ui.snapshot()?;
    let unchanged = || -> Result<()> {
        let now = ui.snapshot()?;
        assert_eq!(now.current, page.current, "the cursor stays on the page");
        assert_eq!(now.visits, page.visits, "a download creates no visit");
        assert_eq!(
            ui.engine.native.uri().as_deref(),
            Some(fixture.url("/page").as_str())
        );
        Ok(())
    };
    assert!(!downloads.exists());

    page_click(&ui, "report")?;
    wait("Saved report.txt toast", || {
        toast_text(&ui).as_deref() == Some("Saved report.txt in Saved files.")
    })?;
    assert_eq!(std::fs::read(downloads.join("report.txt"))?, payload());
    assert_eq!(files(&downloads), ["report.txt"]);
    assert_eq!(files(directory.path()), ["config", "data", "home"]);
    unchanged()?;
    println!("PASS clicking an attachment saves its bytes under a sanitized name in the XDG download folder, created on demand, and leaves the cursor alone.");

    page_click(&ui, "report")?;
    wait("Saved report (1).txt toast", || {
        toast_text(&ui).as_deref() == Some("Saved report (1).txt in Saved files.")
    })?;
    assert_eq!(std::fs::read(downloads.join("report (1).txt"))?, payload());
    assert_eq!(std::fs::read(downloads.join("report.txt"))?, payload());
    unchanged()?;
    println!(
        "PASS a second download of the same name gets a new name and leaves the first file intact."
    );

    page_click(&ui, "blob")?;
    wait("unshowable type saved", || {
        toast_text(&ui).as_deref() == Some("Saved data in Saved files.")
    })?;
    assert_eq!(std::fs::read(downloads.join("data"))?, payload());
    unchanged()?;
    println!("PASS a link to a type WebKit cannot show is saved instead of refused.");

    right_click(&ui, "report")?;
    pump(400);
    key("d")?;
    wait("context menu download saved", || {
        toast_text(&ui).as_deref() == Some("Saved report (2).txt in Saved files.")
    })?;
    assert_eq!(std::fs::read(downloads.join("report (2).txt"))?, payload());
    unchanged()?;
    println!("PASS the page context menu's Download Linked File saves the file.");

    right_click(&ui, "pic")?;
    pump(400);
    key("v")?;
    wait("Save Image As saved", || {
        toast_text(&ui).as_deref() == Some("Saved pic.png in Saved files.")
    })?;
    assert_eq!(std::fs::read(downloads.join("pic.png"))?, png());
    unchanged()?;
    println!("PASS the page context menu's Save Image As saves the image.");

    let before = files(&downloads);
    page_click(&ui, "broken")?;
    wait("Downloading toast", || {
        toast_text(&ui).as_deref() == Some("Downloading broken.bin…")
    })?;
    fixture.release.store(true, Ordering::Relaxed);
    wait("failure toast", || {
        toast_text(&ui).as_deref()
            == Some(
                "Could not download broken.bin. The connection closed before the whole file arrived.",
            )
    })?;
    pump(300);
    assert_eq!(files(&downloads), before, "no partial file is left behind");
    unchanged()?;
    open_memory_menu(&ui)?;
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('#popover .menu-error')].map((e) => e.innerText)"
        )?,
        serde_json::json!([
            "Could not download broken.bin. The connection closed before the whole file arrived."
        ]),
        "no download is reported as a page that failed to load"
    );
    key("Escape")?;
    wait("Memory menu closed", || !visible(&ui, "#popover"))?;
    println!("PASS the user is told when a download starts and when it fails, a failed download leaves no partial file, and the only error recorded is that failure.");

    page_click(&ui, "next")?;
    loaded(&ui, "/next")?;
    let next = ui.snapshot()?;
    let visit = next.current_visit().unwrap();
    assert_eq!(next.visits.len(), page.visits.len() + 1);
    assert_eq!(visit.parent, page.current);
    assert!(
        visit
            .source
            .as_ref()
            .is_some_and(|source| source.link_text == "Next chapter"),
        "the page still records where its links come from: {:?}",
        visit.source
    );
    println!("PASS after downloads the page keeps working: its next link becomes a child with its source context.");
    Ok(())
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

fn page_click(ui: &Ui, id: &str) -> Result<()> {
    click(ui, id, "1")
}

fn right_click(ui: &Ui, id: &str) -> Result<()> {
    click(ui, id, "3")
}

fn click(ui: &Ui, id: &str, button: &str) -> Result<()> {
    let (x, y) = element_point(ui, id)?;
    focus_window()?;
    move_to(x, y)?;
    xdo(&["click", button])?;
    pump(80);
    Ok(())
}

fn files(directory: &Path) -> Vec<String> {
    let names: BTreeSet<String> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.into_iter().collect()
}

fn payload() -> Vec<u8> {
    (0..200_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

fn png() -> Vec<u8> {
    b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x08\x00\x00\x00\x08\x08\x02\x00\x00\x00\x4b\x6d\x29\xdc\x00\x00\x00\x11\x49\x44\x41\x54\x78\xda\x63\xd0\xc8\x73\xc3\x8a\x18\x86\x96\x04\x00\x76\x83\x37\x01\x66\x82\x8b\x14\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82".to_vec()
}

struct Fixture {
    address: std::net::SocketAddr,
    stopped: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let (stop, released) = (stopped.clone(), release.clone());
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut bytes = [0; 8192];
                let n = stream.read(&mut bytes).unwrap_or(0);
                let request = String::from_utf8_lossy(&bytes[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                match path {
                    "/report" => respond(
                        &mut stream,
                        "text/plain\r\nContent-Disposition: attachment; filename=\"../../report.txt\"",
                        &payload(),
                    ),
                    "/data" => respond(&mut stream, "application/x-canopy-unknown", &payload()),
                    "/pic.png" => respond(&mut stream, "image/png", &png()),
                    "/broken" => {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"broken.bin\"\r\nContent-Length: 400000\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&payload()[..65_536]);
                        let _ = stream.flush();
                        while !released.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(5));
                        }
                    }
                    _ => respond(&mut stream, "text/html; charset=utf-8", page(path).as_bytes()),
                }
            }
        });
        Ok(Self {
            address,
            stopped,
            release,
            worker: Some(worker),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
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

fn respond(stream: &mut TcpStream, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn page(path: &str) -> String {
    let (title, content) = match path {
        "/page" => (
            "Release notes",
            r#"<article id="notes" data-permalink="/page#notes"><p>The files for this release.</p><a id="report" href="/report">Download the report</a><a id="blob" href="/data">Raw data</a><a id="broken" href="/broken">A file that breaks halfway</a><a id="next" href="/next">Next chapter</a><img id="pic" src="/pic.png" width="64" height="64" alt="Cover"></article>"#,
        ),
        "/next" => (
            "Next chapter",
            "<main><p>The page after the downloads.</p></main>",
        ),
        _ => ("Fixture", "<main></main>"),
    };
    format!("<!doctype html><html lang='en'><meta charset='utf-8'><title>{title}</title><style>body{{font:18px system-ui;margin:34px}}a{{display:block;margin:16px 0;padding:8px 0}}</style><h1>{title}</h1>{content}</html>")
}
