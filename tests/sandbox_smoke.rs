use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use anyhow::Result;
use canopy::ui::Ui;
use gtk::prelude::*;
use webkit2gtk::{WebContextExt, WebViewExt};

mod common;
use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let body = "<!doctype html><title>Sandbox fixture</title><style>body{background:rgb(12,180,75);margin:0}button{margin:80px;padding:20px}</style><button id='probe' onclick=\"this.textContent='Sandbox click received'\">Test sandbox input</button>";
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
    ui.window.close();
    Ok(())
}
