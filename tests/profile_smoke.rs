#[path = "common/http.rs"]
mod http;
use std::{
    cell::RefCell,
    io::Write,
    net::TcpListener,
    path::Path,
    process::Command,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use canopy::{
    address::web_url,
    browser::NavigationTarget,
    pages::{DownloadEvent, PageEvent},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::{WebContextExt, WebViewExt};
use wry::{WebViewBuilderExtUnix, WebViewExtUnix};

mod common;
use common::*;

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--write-profile") {
        return write_profile(Path::new(&args[2]), &args[3]);
    }
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    let config = directory.path().join("config");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&config)?;
    std::fs::write(
        config.join("user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Saved files\"\n",
    )?;
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", &config);
    std::env::set_var("XDG_DATA_HOME", directory.path().join("xdg-data"));
    std::env::set_var("XDG_CACHE_HOME", directory.path().join("xdg-cache"));
    let fixture = Fixture::start()?;
    let data = directory.path().join("data");
    let profile = data.join("profile");
    // Separate processes rule out a live WebKit context satisfying the migration gate.
    let status = Command::new(std::env::current_exe()?)
        .arg("--write-profile")
        .arg(&profile)
        .arg(fixture.url("/seed"))
        .status()?;
    assert!(status.success(), "old wry profile writer failed");
    let cookies = std::fs::read_to_string(profile.join("cookies"))?;
    assert!(cookies.contains("canopy_persistent\told-cookie"));
    let files = inventory(&profile, &profile)?;
    assert!(files.iter().any(|file| file.contains("localstorage")));
    assert!(files.iter().any(|file| file.contains("favicons/")));
    println!("wry profile files: {}", files.join(", "));

    start(directory.path())?;
    let ui = Ui::new(&data)?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages
        .selected_page()
        .navigate(web_url(&fixture.url("/read"))?, NavigationTarget::Root)?;
    wait("old localStorage in native page", || {
        ui.pages.selected_page().native.title().as_deref() == Some("old-storage")
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(fixture.cookie_received.load(Ordering::Relaxed));
    assert_eq!(
        eval(
            &ui,
            "JSON.stringify(localStorage.getItem('canopy-profile'))"
        )?,
        "old-storage"
    );
    println!("PASS a persistent HTTP cookie written by the old wry construction reaches the server through the real native Ui.");
    println!("PASS localStorage written by the old wry construction survives process exit and is read by the real native Ui. Only old-to-new compatibility is required.");

    key("ctrl+equal")?;
    key("ctrl+equal")?;
    key("ctrl+equal")?;
    assert_eq!(ui.pages.selected_page().native.zoom_level(), 1.5);
    eval(&ui, "JSON.stringify((window.mouseEvents = [], ['mousedown', 'mouseup'].forEach(kind => document.addEventListener(kind, event => { window.mouseEvents.push([event.type, event.button, event.buttons, event.clientX, event.clientY]); event.preventDefault(); })), true))")?;
    let (x, y) = screen_point(&ui, &ui.pages.selected_page().native, 90, 90)?;
    focus_window()?;
    move_to(x, y)?;
    xdo(&["click", "8"])?;
    xdo(&["click", "9"])?;
    wait("page mouse events", || {
        eval(&ui, "JSON.stringify(window.mouseEvents.length)").is_ok_and(|value| value == 4)
    })?;
    assert_eq!(
        eval(&ui, "JSON.stringify(window.mouseEvents)")?,
        serde_json::json!([
            ["mousedown", 3, 8, 60, 60],
            ["mouseup", 3, 0, 60, 60],
            ["mousedown", 4, 16, 60, 60],
            ["mouseup", 4, 0, 60, 60],
        ])
    );
    assert_eq!(
        ui.pages.selected_page().native.uri().as_deref(),
        Some(fixture.url("/read").as_str())
    );
    key("ctrl+0")?;
    assert_eq!(ui.pages.selected_page().native.zoom_level(), 1.);
    println!("PASS back and forward mouse buttons reach page JavaScript at zoomed CSS coordinates and respect preventDefault.");

    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    ui.pages.observe(move |event| {
        if let PageEvent::Download(event) = event {
            received.borrow_mut().push(event);
        }
    });
    let download = ui
        .pages
        .selected_page()
        .native
        .download_uri(&fixture.url("/file"));
    wait("one completed download", || {
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, DownloadEvent::Saved(_)))
    })?;
    pump(500);
    let events = events.borrow();
    assert_eq!(events.len(), 2, "exactly one start and one terminal event");
    assert!(matches!(&events[0], DownloadEvent::Started(name) if name == "one.txt"));
    let DownloadEvent::Saved(saved) = &events[1] else {
        anyhow::bail!("download did not finish")
    };
    assert_eq!(std::fs::read(saved)?, b"one download");
    assert_eq!(std::fs::read_dir(home.join("Saved files"))?.count(), 1);
    drop(download);
    println!(
        "PASS one download creates one file and emits exactly one Started and one Saved event."
    );
    Ok(())
}

fn write_profile(profile: &Path, url: &str) -> Result<()> {
    std::fs::create_dir_all(profile)?;
    start(profile)?;
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    let fixed = gtk::Fixed::new();
    window.add(&fixed);
    let mut context = wry::WebContext::new(Some(profile.to_owned()));
    let view = wry::WebViewBuilder::new_with_web_context(&mut context)
        .with_back_forward_navigation_gestures(false)
        .with_html(OLD_HOME)
        .build_gtk(&fixed)?;
    let native = view.webview();
    #[allow(deprecated)]
    native
        .context()
        .context("wry context")?
        .set_favicon_database_directory(Some(&profile.join("favicons").to_string_lossy()));
    window.show_all();
    native.load_uri(url);
    wait("wry writes localStorage", || {
        native.title().as_deref() == Some("seeded") && !native.is_loading()
    })?;
    native.load_html("<!doctype html><title>Closed</title>", None);
    wait("old document closed", || {
        native.title().as_deref() == Some("Closed") && !native.is_loading()
    })?;
    // WebKit's network and storage processes persist asynchronously.
    wait("persistent cookie file", || {
        std::fs::read_to_string(profile.join("cookies"))
            .is_ok_and(|text| text.contains("canopy_persistent\told-cookie"))
    })?;
    pump(2000);
    drop(view);
    drop(native);
    drop(context);
    unsafe { window.destroy() };
    pump(1000);
    Ok(())
}

fn inventory(root: &Path, directory: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(inventory(root, &path)?);
        } else {
            files.push(path.strip_prefix(root)?.to_string_lossy().into_owned());
        }
    }
    files.sort();
    Ok(files)
}

struct Fixture {
    address: std::net::SocketAddr,
    cookie_received: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let cookie_received = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let received = cookie_received.clone();
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let Ok(request) = http::request(&mut stream) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (headers, body) = match path {
                    "/seed" => (
                        "Content-Type: text/html\r\nSet-Cookie: canopy_persistent=old-cookie; Max-Age=86400; Path=/; HttpOnly\r\n",
                        "<!doctype html><script>localStorage.setItem('canopy-profile','old-storage');document.title='seeded'</script>",
                    ),
                    "/read" => {
                        received.store(request.lines().any(|line| line.to_ascii_lowercase().starts_with("cookie:") && line.contains("canopy_persistent=old-cookie")), Ordering::Relaxed);
                        ("Content-Type: text/html\r\n", "<!doctype html><script>document.title=localStorage.getItem('canopy-profile') || 'missing'</script>")
                    }
                    "/file" => ("Content-Type: text/plain\r\nContent-Disposition: attachment; filename=one.txt\r\n", "one download"),
                    _ => ("Content-Type: text/html\r\n", "<!doctype html><title>Fixture</title>"),
                };
                let response = format!("HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Ok(Self {
            address,
            cookie_received,
            stopped,
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

const OLD_HOME: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><title>Canopy</title>
<style>body{margin:0;background:#f6f5f0;color:#29362e;font:18px system-ui;display:grid;place-content:center;height:100vh}main{max-width:520px;padding:40px}.mark{font-size:56px;color:#53795b}h1{font-weight:550;font-size:36px;letter-spacing:-1px}p{line-height:1.7;color:#647168}kbd{font:14px system-ui;border:1px solid #ccd3c8;border-radius:6px;padding:4px 9px;background:white}.small{font-size:14px;margin-top:36px}</style>
<main><div class="mark">⌘</div><h1>A trail starts here.</h1><p>Open a page, follow a link, then come back to explore another direction. Your branches stay within reach.</p><p class="small"><kbd>Ctrl</kbd> + <kbd>K</kbd> to search or go to an address.</p></main></html>"#;
