#[path = "common/http.rs"]
mod http;
use anyhow::{bail, Result};
use canopy::{
    browser::{Browser, NavigationTarget},
    pages::{DownloadEvent, Go, PageEvent, Pages},
};
use gtk::prelude::*;
use std::{
    cell::RefCell,
    io::Write,
    net::TcpListener,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use webkit2gtk::WebViewExt;

fn main() -> Result<()> {
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
    canopy::init_desktop()?;
    let browser = Browser::open(&directory.path().join("forest.sqlite3"))?;
    let fixed = gtk::Fixed::new();
    let pages = Pages::mount(&fixed, browser, &directory.path().join("profile"))?;
    let weak = pages.selected_page().native.downgrade();
    let favicons = pages.favicons();
    pages.release_selected_page();
    wait("favicons_release_page", || weak.upgrade().is_none())?;
    drop(favicons);
    println!("PASS favicons_release_page while the cache and context are alive.");

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let release = Arc::new(AtomicBool::new(false));
    let released = release.clone();
    let server = thread::spawn(move || loop {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let request = http::request(&mut stream).unwrap();
        if String::from_utf8_lossy(&request).contains("GET /file ") {
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=held.bin\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n").unwrap();
            stream.write_all(&vec![42; 65536]).unwrap();
            stream.flush().unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while !released.load(Ordering::Relaxed) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            stream.write_all(&vec![42; 65536]).unwrap();
            break;
        } else {
            let body = "<!doctype html><title>Source</title><p>Source page</p>";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    pages.mount_selected_page(&fixed)?;
    pages.go(Go::Address {
        url: format!("http://{address}/page").parse()?,
        target: NavigationTarget::Root,
    })?;
    wait("source committed", || {
        pages.selected_page().native.title().as_deref() == Some("Source")
            && !pages.selected_page().native.is_loading()
    })?;
    let page = pages.read(|b| b.snapshot())?.current.unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    pages.observe(move |event| {
        if let PageEvent::Download(event) = event {
            received.borrow_mut().push(event);
        }
    });
    pages
        .selected_page()
        .native
        .download_uri(&format!("http://{address}/file"));
    wait("download started", || {
        events
            .borrow()
            .iter()
            .any(|e| matches!(e, DownloadEvent::Started(_)))
    })?;
    let weak = pages.selected_page().native.downgrade();
    pages.release_selected_page();
    wait("download_outlives_source_view", || weak.upgrade().is_none())?;
    release.store(true, Ordering::Relaxed);
    wait("download saved", || {
        events
            .borrow()
            .iter()
            .any(|e| matches!(e, DownloadEvent::Saved(_)))
    })?;
    server.join().unwrap();
    pump();
    let events = events.borrow();
    assert_eq!(events.len(), 2, "one start and one completion");
    let DownloadEvent::Saved(path) = &events[1] else {
        panic!("expected saved")
    };
    assert_eq!(std::fs::read(path)?, vec![42; 131072]);
    let downloads = pages.read(|b| b.downloads())?;
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].visit, page);
    assert_eq!(downloads[0].path, *path);
    println!("PASS download_outlives_source_view finalizes the source before completion, saves the full file, records its original visit and reports once.");
    Ok(())
}

fn pump() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}
fn wait(label: &str, condition: impl Fn() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        pump();
        if condition() {
            return Ok(());
        }
        if Instant::now() > deadline {
            bail!("Timed out waiting for {label}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}
