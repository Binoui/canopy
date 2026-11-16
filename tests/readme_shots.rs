//! Takes the README's screenshots from a small made-up web served on this computer.
//!
//! Does nothing unless `CANOPY_README_SHOTS` names the folder to write to.

use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use anyhow::{Context, Result};
use canopy::{
    address::web_url,
    browser::{NavigationTarget, VisitId},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::{WebViewExt, WebsiteDataManagerExt};

mod common;
mod readme_site;

use common::*;

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let Ok(folder) = std::env::var("CANOPY_README_SHOTS") else {
        println!("SKIP set CANOPY_README_SHOTS to a folder to take the README's screenshots.");
        return Ok(());
    };
    std::fs::create_dir_all(&folder)?;
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home)?;
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
    std::env::set_var("XDG_DATA_HOME", home.join("share"));
    start(directory.path())?;
    let proxy = serve()?;
    let ui = Ui::new(&directory.path().join("data"))?;
    ui.window.show_all();
    ready(&ui)?;
    // Every made-up site resolves to the local server, so pages carry real-looking addresses.
    let mut settings = webkit2gtk::NetworkProxySettings::new(Some(&proxy), &[]);
    ui.pages
        .selected_page()
        .native
        .website_data_manager()
        .context("data manager")?
        .set_network_proxy_settings(webkit2gtk::NetworkProxyMode::Custom, Some(&mut settings));
    let film = Film::new(&format!("{folder}/fork"))?;
    build(&ui, &film)?;
    for theme in ["dark", "light"] {
        chrome(&ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
        wait("theme", || {
            chrome(&ui, "canopy.state().dark").is_ok_and(|v| v == (theme == "dark"))
        })?;
        pump(300);
        let shot = |name: &str| save_window(&ui, &format!("{folder}/{name}-{theme}.png"));
        rest(&ui)?;
        open_strip(&ui)?;
        let thread = visit(&ui, "/t/4821")?;
        hover_chrome(&ui, &node(thread))?;
        wait("card", || visible(&ui, "#card"))?;
        pump(900);
        shot("card")?;
        rest(&ui)?;
        wait("card gone", || !visible(&ui, "#card"))?;
        pump(400);
        let current = ui.snapshot()?.current.context("current page")?;
        click_chrome(&ui, &format!(".leaves[data-id='{current}']"))?;
        wait("files", || visible(&ui, "#popover .file-row"))?;
        // Off the leaf, so its tooltip is not in the picture.
        hover_chrome(&ui, "#popover .file-row")?;
        pump(700);
        shot("files")?;
        key("Escape")?;
        rest(&ui)?;
        key("ctrl+shift+e")?;
        wait("tree view", || visible(&ui, "#tree-sheet"))?;
        chrome(&ui, "(setTreeSide('tree'), true)")?;
        pump(400);
        shot("tree")?;
        key("Escape")?;
        key("ctrl+e")?;
        wait("forest", || visible(&ui, "#forest"))?;
        pump(500);
        shot("forest")?;
        key("Escape")?;
        key("ctrl+k")?;
        wait("palette", || chrome_focused(&ui, "#palette-input"))?;
        type_text("akadama")?;
        pump(700);
        shot("palette")?;
        key("Escape")?;
        key("ctrl+t")?;
        wait("new-tree page", || visible(&ui, "#start"))?;
        rest(&ui)?;
        pump(500);
        shot("start")?;
        key("Escape")?;
        pump(200);
    }
    println!("Saved the README's screenshots in {folder}.");
    Ok(())
}

/// Parks the pointer over the page and waits for the strip to close.
fn rest(ui: &Ui) -> Result<()> {
    let (x, y) = page_point(ui, 900, 640)?;
    move_to(x, y)?;
    wait("strip closed", || strip_height(ui) == 52)?;
    pump(350);
    Ok(())
}

fn current(ui: &Ui) -> Result<VisitId> {
    ui.snapshot()?.current.context("current page")
}

/// The first page whose address ends with `path`.
fn visit(ui: &Ui, path: &str) -> Result<VisitId> {
    ui.snapshot()?
        .visits
        .iter()
        .find(|v| v.url.path() == path)
        .map(|v| v.id)
        .with_context(|| format!("no page at {path}"))
}

fn back_to(ui: &Ui, id: VisitId) -> Result<()> {
    ui.pages.selected_page().restore(id)?;
    wait("restored page", || {
        ui.snapshot().unwrap().current == Some(id) && !ui.pages.selected_page().native.is_loading()
    })?;
    pump(150);
    Ok(())
}

fn download(ui: &Ui, link: &str) -> Result<()> {
    let before = ui.downloads()?.len();
    let (x, y) = element_point(ui, link)?;
    focus_window()?;
    move_to(x, y)?;
    xdo(&["click", "1"])?;
    wait(link, || ui.downloads().unwrap().len() == before + 1)?;
    pump(150);
    Ok(())
}

fn name_tree(ui: &Ui, root: VisitId, name: &str, pin: bool) -> Result<()> {
    let tree = ui
        .pages
        .read(|b| b.tree_states())?
        .into_iter()
        .map(|state| state.root)
        .find(|tree| tree.root() == root)
        .context("tree")?;
    ui.pages.rename_tree(tree, name)?;
    if pin {
        ui.pages.set_pinned(tree, true)?;
    }
    Ok(())
}

/// Browses the made-up sites the way a person would: three trees, forks, waiting pages, files.
fn build(ui: &Ui, film: &Film) -> Result<()> {
    go(ui, "http://ferrous.example/book/iterators", None)?;
    let rust = current(ui)?;
    go(ui, "http://ferrous.example/book/closures", Some(rust))?;
    name_tree(ui, rust, "Iterators", true)?;

    go(ui, "http://slowrail.example/routes/tokyo-kyoto", None)?;
    let trip = current(ui)?;
    go(
        ui,
        "http://templegardens.example/kyoto/november",
        Some(trip),
    )?;
    let gardens = current(ui)?;
    go(
        ui,
        "http://templegardens.example/kyoto/daitoku-ji",
        Some(gardens),
    )?;
    back_to(ui, gardens)?;
    go(
        ui,
        "http://templegardens.example/kyoto/omiya-bonsai",
        Some(gardens),
    )?;
    name_tree(ui, trip, "Kyoto in November", true)?;

    go(ui, "http://bonsaiwiki.example/wiki/Repotting", None)?;
    let root = current(ui)?;
    go(ui, "http://bonsaiwiki.example/wiki/Akadama", Some(root))?;
    let akadama = current(ui)?;
    go(
        ui,
        "http://koen-nursery.example/soil/akadama-14l",
        Some(akadama),
    )?;
    back_to(ui, root)?;
    go(ui, "http://bonsaiwiki.example/wiki/Juniper", Some(root))?;
    back_to(ui, root)?;
    name_tree(ui, root, "Repotting the juniper", true)?;
    // Filmed: a link is followed, three more are left waiting, another is followed, and the
    // strip shows what the tree has become.
    rest(ui)?;
    film.hold(ui, 900)?;
    film.click(ui, "thread", "1")?;
    wait("thread", || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == "/t/4821")
            && !ui.pages.selected_page().native.is_loading()
    })?;
    film.hold(ui, 1100)?;
    for link in ["pruning", "wrong", "pots"] {
        film.click(ui, link, "2")?;
        film.hold(ui, 500)?;
    }
    let thread = current(ui)?;
    film.click(ui, "log", "1")?;
    wait("log", || {
        ui.snapshot().unwrap().current != Some(thread)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    film.hold(ui, 1000)?;
    let (left, _, width, _) = rect(ui, "#strip-clip")?;
    let (x, y) = page_point(ui, (left + width / 2.) as i32, 68)?;
    film.glide(ui, x, y)?;
    film.hold(ui, 1300)?;
    let (x, y) = chrome_point(ui, &node(thread))?;
    film.glide(ui, x, y)?;
    film.hold(ui, 2600)?;
    rest(ui)?;
    eval(ui, "document.getElementById('calendar').scrollIntoView({block:'center'}); JSON.stringify(true)")?;
    pump(100);
    download(ui, "calendar")?;
    download(ui, "mix")?;
    eval(ui, "scrollTo(0, 0); JSON.stringify(true)")?;

    for (address, title) in [
        ("http://bonsaiwiki.example/", "Bonsai Wiki"),
        ("http://rootbound.example/", "Rootbound"),
        ("http://mossandwire.example/", "Moss & Wire"),
        ("http://koen-nursery.example/", "Kōen Nursery"),
        ("http://ferrous.example/book/iterators", "The Ferrous Book"),
        ("http://kilnlog.example/glazes/ash", "Kiln Log"),
        ("http://weather.example/", "Skywatch"),
        ("http://mail.example/", "Postbox"),
        ("http://paperlane.example/", "Paper Lane"),
        ("http://tinytracker.example/", "Tiny Tracker"),
        ("http://slowrail.example/", "Slowrail"),
        ("http://templegardens.example/", "Temple Gardens"),
    ] {
        ui.pages.bookmark(&web_url(address)?, title)?;
    }
    wait("toasts gone", || !visible(ui, "#toast"))?;
    Ok(())
}

/// Loads `address` as a new tree, or under `parent`, and waits for it.
fn go(ui: &Ui, address: &str, parent: Option<VisitId>) -> Result<()> {
    let url = web_url(address)?;
    let target = parent.map_or(NavigationTarget::Root, NavigationTarget::ChildOf);
    ui.pages.selected_page().navigate(url.clone(), target)?;
    wait(address, || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url == url)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    pump(150);
    Ok(())
}

/// Starts the local server that plays every made-up site, and returns its proxy address.
fn serve() -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = format!("http://{}", listener.local_addr()?);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            thread::spawn(move || -> std::io::Result<()> {
                let mut request = [0; 8192];
                let size = stream.read(&mut request)?;
                let request = String::from_utf8_lossy(&request[..size]);
                // A proxy is asked for the whole address: GET http://host/path HTTP/1.1
                let target = request.split_whitespace().nth(1).unwrap_or("/");
                let rest = target.strip_prefix("http://").unwrap_or(target);
                let (host, path) = match rest.find('/') {
                    Some(at) => (&rest[..at], &rest[at..]),
                    None => (rest, "/"),
                };
                let (status, kind, extra, body) = match readme_site::answer(host, path) {
                    Some(answer) => ("200 OK", answer.kind, answer.header, answer.body),
                    None => ("404 Not Found", "text/plain", "", b"Not here.".to_vec()),
                };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
                stream.write_all(&body)
            });
        }
    });
    Ok(address)
}

/// Numbered frames of the window, with the pointer drawn in, for the README's short film.
struct Film {
    folder: String,
    count: std::cell::Cell<u32>,
    pointer: gtk::gdk_pixbuf::Pixbuf,
    at: std::cell::Cell<(i32, i32)>,
}

impl Film {
    fn new(folder: &str) -> Result<Self> {
        std::fs::create_dir_all(folder)?;
        let loader = gtk::gdk_pixbuf::PixbufLoader::new();
        loader.write(b"<svg xmlns='http://www.w3.org/2000/svg' width='22' height='30'><path d='M2 2v22l5.6-5.2 4 9 3.6-1.6-4-8.8H19z' fill='#111' stroke='#fff' stroke-width='1.6' stroke-linejoin='round'/></svg>")?;
        loader.close()?;
        Ok(Self {
            folder: folder.to_owned(),
            count: Default::default(),
            pointer: loader.pixbuf().context("pointer drawing")?,
            at: std::cell::Cell::new((900, 640)),
        })
    }

    /// Saves one frame. The pointer is drawn where this film last moved it.
    fn frame(&self, ui: &Ui) -> Result<()> {
        let window = ui.window.window().context("realized window")?;
        let a = ui.window.allocation();
        let frame = window
            .pixbuf(0, 0, a.width(), a.height())
            .context("window capture")?;
        let (_, ox, oy) = window.origin();
        let (x, y) = self.at.get();
        let (x, y) = (
            (x - ox).clamp(0, a.width() - 23),
            (y - oy).clamp(0, a.height() - 31),
        );
        self.pointer.composite(
            &frame,
            x,
            y,
            22,
            30,
            f64::from(x),
            f64::from(y),
            1.,
            1.,
            gtk::gdk_pixbuf::InterpType::Bilinear,
            255,
        );
        self.count.set(self.count.get() + 1);
        frame.savev(
            format!("{}/{:04}.png", self.folder, self.count.get()),
            "png",
            &[("compression", "1")],
        )?;
        Ok(())
    }

    /// Films for `ms`, at the film's twelve frames a second.
    fn hold(&self, ui: &Ui, ms: u64) -> Result<()> {
        let start = std::time::Instant::now();
        let first = self.count.get();
        while start.elapsed().as_millis() < u128::from(ms) {
            let due = u128::from(self.count.get() - first) * 1000 / 12;
            if start.elapsed().as_millis() >= due {
                self.frame(ui)?;
                pump(2);
            } else {
                pump(4);
            }
        }
        Ok(())
    }

    /// Moves the pointer to a screen point in small eased steps, filming on the way.
    fn glide(&self, ui: &Ui, x: i32, y: i32) -> Result<()> {
        let (fx, fy) = self.at.get();
        for step in 1..=9 {
            let t = f64::from(step) / 9.;
            let t = t * t * (3. - 2. * t);
            let at = (
                fx + (f64::from(x - fx) * t) as i32,
                fy + (f64::from(y - fy) * t) as i32,
            );
            move_to(at.0, at.1)?;
            self.at.set(at);
            self.hold(ui, 80)?;
        }
        Ok(())
    }

    /// Glides to a link of the page and clicks it with the given mouse button.
    fn click(&self, ui: &Ui, link: &str, button: &str) -> Result<()> {
        let (x, y) = element_point(ui, link)?;
        focus_window()?;
        self.glide(ui, x, y)?;
        self.hold(ui, 250)?;
        xdo(&["click", button])?;
        self.hold(ui, 250)?;
        Ok(())
    }
}
