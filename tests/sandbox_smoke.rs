#[path = "common/http.rs"]
mod http;
use std::{io::Write, net::TcpListener, thread};

use anyhow::Result;
use canopy::ui::Ui;
use gtk::prelude::*;
use webkit2gtk::{WebContextExt, WebViewExt};

mod common;
use common::*;

/// Six seconds of solid red, H.264 in MP4, 640x360. Made with
/// `ffmpeg -f lavfi -i color=c=red:s=640x360:r=30:d=6 -c:v libx264 -pix_fmt yuv420p -g 30 -movflags +faststart`.
const RED_VIDEO: &[u8] = include_bytes!("fixtures/red-h264.mp4");

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let Ok(request) = http::request(&mut stream) else {
                continue;
            };
            let request = String::from_utf8_lossy(&request);
            if request.starts_with("GET /red.mp4") {
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", RED_VIDEO.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(RED_VIDEO);
                continue;
            }
            let body = if request.starts_with("GET /video") {
                "<!doctype html><title>Video fixture</title><body style='margin:0;background:rgb(0,0,255)'><video id='video' src='/red.mp4' width='640' height='360' muted loop autoplay playsinline></video>"
            } else {
                "<!doctype html><title>Sandbox fixture</title><style>body{background:rgb(12,180,75);margin:0}button{margin:80px;padding:20px}</style><button id='probe' onclick=\"this.textContent='Sandbox click received'\">Test sandbox input</button>"
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(response.as_bytes());
        }
    });
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    let page = ui.pages.selected_page().native.context().unwrap();
    let chrome_context = ui.chrome.native().context().unwrap();
    assert_ne!(
        page, chrome_context,
        "page and chrome have separate contexts"
    );
    for (name, context) in [("page", page), ("chrome", chrome_context)] {
        assert!(context.is_sandbox_enabled(), "{name} sandbox enabled");
        println!("PASS {name} context reports sandbox enabled.");
    }
    wait("chrome font loaded", || {
        chrome(&ui, "document.fonts.status === 'loaded' && [...document.fonts].some(f => f.family === 'Inter' && f.status === 'loaded')")
            .is_ok_and(|value| value == true)
    })?;
    key("ctrl+l")?;
    type_text(&format!("http://{address}/"))?;
    key("Return")?;
    wait("local page loaded", || {
        !ui.pages.selected_page().native.is_loading()
            && ui.pages.selected_page().native.title().as_deref() == Some("Sandbox fixture")
    })?;
    pump(250);
    let (x, y) = element_point(&ui, "probe")?;
    click_at(x, y)?;
    assert_eq!(
        eval(
            &ui,
            "JSON.stringify(document.getElementById('probe').textContent)"
        )?,
        "Sandbox click received"
    );
    let (x, y) = ui
        .pages
        .selected_page()
        .native
        .translate_coordinates(&ui.window, 500, 400)
        .unwrap();
    let pixel = ui.window.window().unwrap().pixbuf(x, y, 1, 1).unwrap();
    let bytes = pixel.read_pixel_bytes();
    assert_eq!(
        &bytes[..3],
        &[12, 180, 75],
        "the fixture painted in the real window"
    );
    println!("PASS sandboxed local page paints and receives a real pointer click; canopy chrome and Inter font load.");
    key("ctrl+l")?;
    type_text(&format!("http://{address}/video"))?;
    key("Return")?;
    wait("video page loaded", || {
        ui.pages.selected_page().native.title().as_deref() == Some("Video fixture")
    })?;
    wait("video playing or refused", || {
        eval(&ui, "(v => !!v && (v.currentTime > 0.5 || v.error !== null))(document.getElementById('video'))")
            .is_ok_and(|value| value == true)
    })?;
    if eval(&ui, "document.getElementById('video').error !== null")? == true {
        println!("SKIP this machine has no H.264 decoder for the sandboxed page.");
    } else {
        pump(250);
        let (x, y) = ui
            .pages
            .selected_page()
            .native
            .translate_coordinates(&ui.window, 320, 180)
            .unwrap();
        let pixel = ui.window.window().unwrap().pixbuf(x, y, 1, 1).unwrap();
        let bytes = pixel.read_pixel_bytes();
        assert!(
            bytes[0] > 200 && bytes[1] < 60 && bytes[2] < 60,
            "a red video paints red in the real window, got {:?}",
            &bytes[..3]
        );
        println!("PASS sandboxed page plays H.264 video in its true colours.");
    }
    ui.window.close();
    Ok(())
}
