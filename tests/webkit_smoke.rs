use std::{
    io::{Read, Write},
    net::TcpListener,
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
    browser::{NavigationTarget, Snapshot, VisitState},
    ui::Ui,
};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

mod common;

use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let icon = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 16, 16)
        .context("favicon pixels")?;
    icon.fill(0x2050ffff);
    let fixture = Fixture::start(icon.save_to_bufferv("png", &[])?)?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    assert!(visible(&ui, "#start"));
    assert!(ui.snapshot()?.visits.is_empty());
    assert!(text(&ui, "#start").contains("as your forest grows"));
    assert_eq!(
        chrome(&ui, "canopy.state().dark")?,
        true,
        "a new forest starts dark"
    );
    println!("PASS a new forest starts in the dark theme.");
    {
        use canopy::{
            cookies::{Cookie, CookieDomain, Inbox, SameSite},
            secret::Secret,
        };
        use std::{cell::RefCell, rc::Rc};
        use webkit2gtk::{CookieManagerExt, WebContextExt, WebsiteDataManagerExt};
        let inbox = Inbox::new(directory.path());
        let cookie = |domain: &str, name: &str| Cookie {
            domain: CookieDomain::parse(domain).unwrap(),
            path: "/".into(),
            name: name.into(),
            value: Secret::new("fake-cookie".into()),
            expires: None,
            secure: false,
            http_only: true,
            same_site: SameSite::Lax,
        };
        inbox.post(&[
            cookie(".example.test", "domain"),
            cookie("example.test", "host"),
        ])?;
        wait("UI applies cookie inbox", || {
            inbox.collect().is_ok_and(|b| b.is_empty())
        })?;
        let manager = ui
            .pages
            .selected_page()
            .native
            .context()
            .unwrap()
            .website_data_manager()
            .unwrap()
            .cookie_manager()
            .unwrap();
        let received = Rc::new(RefCell::new(None));
        let result = received.clone();
        manager.cookies(
            "http://sub.example.test/",
            None::<&gio::Cancellable>,
            move |cookies| {
                *result.borrow_mut() = Some(cookies);
            },
        );
        wait("subdomain cookies", || received.borrow().is_some())?;
        let mut cookies = received.borrow_mut().take().unwrap()?;
        assert!(cookies
            .iter_mut()
            .any(|c| c.name().as_deref() == Some("domain")));
        assert!(!cookies
            .iter_mut()
            .any(|c| c.name().as_deref() == Some("host")));
        assert!(inbox.collect()?.is_empty());
        println!("PASS the running UI applies domain cookies to subdomains, keeps host cookies host-only, and accepts the inbox.");
    }

    key("ctrl+k")?;
    wait("palette focus", || chrome_focused(&ui, "#palette-input"))?;
    type_text(&fixture.url("/a"))?;
    key("Return")?;
    loaded(&ui, "/a", 1)?;
    let a = ui.snapshot()?.current.unwrap();
    assert!(!visible(&ui, "#palette"));
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    link(&ui, "b")?;
    loaded(&ui, "/b", 2)?;
    let b = ui.snapshot()?.current.unwrap();
    println!("PASS the palette plants a tree from a typed address; a real link adds B under A.");
    wait("the page's favicon on its node", || {
        icon_is_blue(&ui, &node(a))
    })?;
    println!("PASS WebKit's favicon database feeds the strip node of an opened page.");

    open_strip(&ui)?;
    click_chrome(&ui, &node(a))?;
    loaded(&ui, "/a", 2)?;
    link(&ui, "c")?;
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
    wait("page focus after a strip click", || {
        ui.pages.selected_page().native.has_focus()
    })?;
    println!(
        "PASS hovering opens the strip, clicking node A restores it and gives the page the keyboard, and C becomes B's sibling."
    );

    leave_strip(&ui)?;
    key("F6")?;
    wait("current node focus", || chrome_focused(&ui, &node(c)))?;
    key("Left")?;
    wait("parent node focus", || chrome_focused(&ui, &node(a)))?;
    key("Return")?;
    loaded(&ui, "/a", 3)?;
    link(&ui, "c")?;
    loaded(&ui, "/c", 4)?;
    let c = ui.snapshot()?.current.unwrap();
    println!(
        "PASS F6 focuses the current node, Left walks the trunk and Enter restores the parent."
    );

    ui.pages.selected_page().native.grab_focus();
    pump(100);
    key("ctrl+l")?;
    wait("Ctrl+L from the focused web view", || {
        chrome_focused(&ui, "#palette-input")
    })?;
    let field = chrome(
        &ui,
        "(() => { const i = document.getElementById('palette-input'); return [i.value, i.selectionStart, i.selectionEnd]; })()",
    )?;
    assert_eq!(field[0], fixture.url("/c"));
    assert_eq!(field[1], 0);
    assert_eq!(field[2], fixture.url("/c").chars().count());
    key("Escape")?;
    wait("palette closed", || !visible(&ui, "#palette"))?;
    wait("page focus", || ui.pages.selected_page().native.has_focus())?;
    println!("PASS Ctrl+L opens the palette on the current address from the focused web view; Escape returns to the page.");

    let (x, y) = element_point(&ui, "q")?;
    click_at(x, y)?;
    type_text("alpha")?;
    key("ctrl+k")?;
    wait("palette over a focused input", || {
        chrome_focused(&ui, "#palette-input")
    })?;
    type_text("zzz")?;
    key("Escape")?;
    wait("page focus", || ui.pages.selected_page().native.has_focus())?;
    type_text("omega")?;
    assert_eq!(
        eval(&ui, "JSON.stringify(document.getElementById('q').value)")?,
        "alphaomega"
    );
    println!("PASS Ctrl+K then Escape gives keyboard focus back to the page input that had it.");

    eval(
        &ui,
        "window.clicks = 0; addEventListener('click', () => clicks++); JSON.stringify(true)",
    )?;
    let clicks = |ui: &Ui| {
        eval(ui, "JSON.stringify(window.clicks)")
            .unwrap()
            .as_i64()
            .unwrap()
    };
    open_strip(&ui)?;
    let (x, y) = page_point(&ui, 900, 42 + 120 + 40)?;
    click_at(x, y)?;
    assert_eq!(
        clicks(&ui),
        1,
        "a click below the open strip reaches the page"
    );
    open_strip(&ui)?;
    hover_chrome(&ui, &node(c))?;
    wait("hover card", || visible(&ui, "#card"))?;
    pump(150);
    hover_chrome(&ui, "#card")?;
    pump(500);
    assert!(
        visible(&ui, "#card") && strip_height(&ui) == 120,
        "the card holds the strip open"
    );
    let (card_x, card_y, card_w, card_h) = rect(&ui, "#card")?;
    let beside = screen_point(
        &ui,
        &ui.chrome.native(),
        (card_x + card_w + 180.) as i32,
        (card_y + card_h / 2.) as i32,
    )?;
    move_to(beside.0, beside.1)?;
    wait("card and strip closed", || {
        !visible(&ui, "#card") && strip_height(&ui) == 52 && cover(&ui) == Some(94)
    })?;
    click_at(beside.0, beside.1)?;
    assert_eq!(
        clicks(&ui),
        2,
        "beside the card, the page gets clicks once the card has closed"
    );
    let (x, y) = page_point(&ui, 900, 42 + 120 + 300)?;
    click_at(x, y)?;
    assert_eq!(
        clicks(&ui),
        3,
        "a click under the card's band reaches the page"
    );
    open_strip(&ui)?;
    let (away_x, away_y) = page_point(&ui, 900, 600)?;
    move_to(away_x, away_y)?;
    pump(430);
    assert_eq!(
        cover(&ui),
        Some(42 + 120 + 24),
        "the closing strip keeps its open band"
    );
    let (x, y) = page_point(&ui, 900, 140)?;
    click_at(x, y)?;
    assert_eq!(
        clicks(&ui),
        3,
        "the closing band swallows a click during the close"
    );
    move_to(away_x, away_y)?;
    pump(900);
    assert_eq!(strip_height(&ui), 52);
    click_at(x, y)?;
    assert_eq!(
        clicks(&ui),
        4,
        "the same point reaches the page once the strip is closed"
    );
    println!("PASS page clicks pass below the open strip and under the card; beside the card and in the closing band they wait for the chrome to close, then reach the page.");

    save_window(&ui, "/tmp/canopy-chrome-webkit.png")?;

    ui.pages.selected_page().reload()?;
    wait("reload start", || {
        ui.pages.selected_page().native.is_loading()
    })?;
    loaded(&ui, "/c", 4)?;
    assert_eq!(ui.snapshot()?.current, Some(c));
    println!("PASS reloading adds no node.");

    ui.pages.selected_page().navigate(
        web_url(&fixture.url("/redirect"))?,
        NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap()),
    )?;
    loaded(&ui, "/final", 5)?;
    assert_eq!(ui.snapshot()?.current_visit().unwrap().url.path(), "/final");
    println!("PASS an HTTP 302 is recorded once, under its final URL.");

    let before_failure = ui.snapshot()?;
    let unused_port = TcpListener::bind("127.0.0.1:0")?;
    let port = unused_port.local_addr()?.port();
    drop(unused_port);
    let failed_url = format!("http://127.0.0.1:{port}/unreachable");
    ui.pages.selected_page().navigate(
        web_url(&failed_url)?,
        NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap()),
    )?;
    wait("connection failure", || {
        toast_text(&ui).is_some_and(|text| text.contains("Could not load"))
            && !ui.pages.selected_page().native.is_loading()
    })?;
    pump(150);
    assert_eq!(ui.snapshot()?, before_failure);
    open_memory_menu(&ui)?;
    assert!(text(&ui, "#popover").contains("Could not load"));
    key("Escape")?;
    wait("Memory menu closed", || !visible(&ui, "#popover"))?;
    println!("PASS a network failure shows a toast, stays listed in the Memory menu, and changes nothing.");

    ui.pages.selected_page().restore(a)?;
    loaded(&ui, "/a", 5)?;
    trusted_click(&ui, "blank")?;
    wait("target=_blank queued", || {
        ui.snapshot().unwrap().visits.len() == 6
    })?;
    let pending = ui
        .snapshot()?
        .visits
        .into_iter()
        .find(|visit| visit.state == VisitState::Queued)
        .context("queued target=_blank")?;
    assert_eq!(pending.parent, Some(a));
    assert_eq!(ui.snapshot()?.current, Some(a));
    open_strip(&ui)?;
    hover_chrome(&ui, &node(pending.id))?;
    wait("card of the queued page", || {
        visible(&ui, "#card [data-act=branch]")
    })?;
    pump(150);
    click_chrome(&ui, "#card [data-act=branch]")?;
    loaded(&ui, "/d", 6)?;
    assert_eq!(ui.snapshot()?.current, Some(pending.id));
    wait("palette anchored after the commit", || {
        visible(&ui, "#palette") && chrome_focused(&ui, "#palette-input")
    })?;
    assert!(text(&ui, "#palette-context").contains("Link opened in another window"));
    key("Escape")?;
    println!("PASS a target=_blank link waits in the tree; New branch here activates it without a duplicate, then anchors the palette on it.");

    ui.pages.selected_page().navigate(
        web_url(&fixture.url("/b"))?,
        NavigationTarget::ChildOf(ui.snapshot()?.current.unwrap()),
    )?;
    ui.pages
        .selected_page()
        .navigate(web_url(&fixture.url("/c"))?, NavigationTarget::Root)?;
    loaded(&ui, "/c", 7)?;
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    println!("PASS close requests coalesce; the last new tree stays a root.");

    pump(300);
    click_chrome(&ui, &format!("#tab-row [data-id='{a}']"))?;
    loaded(&ui, "/d", 7)?;
    assert_eq!(ui.snapshot()?.current, Some(pending.id));
    println!("PASS a tree tab restores its tree's last page.");

    let tab_title = format!("#tab-row [data-id='{a}'] .tab-title");
    let automatic = text(&ui, &tab_title);
    let name_tree = |keys: &dyn Fn() -> Result<()>| -> Result<()> {
        let (x, y) = chrome_point(&ui, &tab_title)?;
        move_to(x, y)?;
        xdo(&["click", "3"])?;
        click_chrome(&ui, "[data-tab-action=rename]")?;
        wait("tree name field", || chrome_focused(&ui, "#tree-name"))?;
        keys()?;
        key("Return")?;
        wait("tree name field closed", || !visible(&ui, "#popover"))
    };
    name_tree(&|| type_text("Reading list"))?;
    wait("tree renamed", || text(&ui, &tab_title) == "Reading list")?;
    assert_eq!(ui.snapshot()?.current, Some(pending.id));
    name_tree(&|| key("BackSpace"))?;
    wait("automatic tree name", || text(&ui, &tab_title) == automatic)?;
    println!("PASS a right click on a tab names its tree; an empty name is automatic again.");

    for n in 1..=8 {
        ui.pages.selected_page().navigate(
            web_url(&fixture.url(&format!("/topic-{n}")))?,
            NavigationTarget::Root,
        )?;
        loaded(&ui, &format!("/topic-{n}"), 7 + n)?;
    }
    pump(400);
    let roots: Vec<_> = ui
        .snapshot()?
        .visits
        .into_iter()
        .filter(|v| v.parent.is_none())
        .collect();
    let hidden: Vec<String> = roots
        .iter()
        .filter(|root| !visible(&ui, &format!("#tab-row [data-id='{}']", root.id)))
        .map(|root| root.label().to_owned())
        .collect();
    assert!(!hidden.is_empty(), "ten trees overflow a 1240 px tab row");
    assert!(visible(
        &ui,
        &format!("#tab-row [data-id='{}']", roots.last().unwrap().id)
    ));
    click_chrome(&ui, "#tab-overflow")?;
    wait("overflow list", || visible(&ui, "#popover [data-id]"))?;
    let listed: Vec<String> = chrome(
        &ui,
        "[...document.querySelectorAll('#popover [data-id] .grow')].map((e) => e.textContent)",
    )?
    .as_array()
    .unwrap()
    .iter()
    .map(|v| v.as_str().unwrap().to_owned())
    .collect();
    assert_eq!(listed, hidden);
    click_chrome(&ui, "#popover [data-id]")?;
    let hidden_root = roots
        .iter()
        .find(|root| root.label() == hidden[0])
        .unwrap()
        .id;
    wait("hidden tree selected", || {
        ui.snapshot().unwrap().current == Some(hidden_root)
            && !ui.pages.selected_page().native.is_loading()
    })?;
    println!(
        "PASS the overflow arrow lists exactly the hidden trees, in creation order, and opens one."
    );

    key("ctrl+e")?;
    wait("forest open", || visible(&ui, "#forest"))?;
    pump(200);
    let forest_node = |id| format!("#forest-nodes [data-id='{id}']");
    let root_node = forest_node(hidden_root);
    assert!(
        rect(&ui, &root_node)?.2 > 140.,
        "the first forest view fits after allocation"
    );
    let before_zoom = rect(&ui, &root_node)?.2;
    let (x, y) = chrome_point(&ui, "#forest-canvas")?;
    move_to(x, y)?;
    xdo(&["keydown", "ctrl", "click", "5", "keyup", "ctrl"])?;
    pump(80);
    assert!(rect(&ui, &root_node)?.2 < before_zoom);
    click_chrome(&ui, "#forest-fit")?;
    pump(300);
    let before_pan = rect(&ui, &root_node)?;
    move_to(x, y)?;
    xdo(&["click", "5"])?;
    pump(80);
    assert!(rect(&ui, &root_node)?.1 < before_pan.1);
    xdo(&[
        "mousedown",
        "1",
        "mousemove_relative",
        "--sync",
        "40",
        "20",
        "mouseup",
        "1",
    ])?;
    pump(80);
    assert!(rect(&ui, &root_node)?.0 > before_pan.0);
    click_chrome(&ui, "#forest-fit")?;
    pump(300);
    let other = ui
        .snapshot()?
        .visits
        .into_iter()
        .find(|v| v.parent.is_none() && v.id != hidden_root)
        .unwrap();
    click_chrome(&ui, &forest_node(other.id))?;
    wait(
        "forest node restores another tree and closes overlay",
        || {
            !visible(&ui, "#forest")
                && ui.snapshot().unwrap().current == Some(other.id)
                && !ui.pages.selected_page().native.is_loading()
        },
    )?;
    let first = other.id;
    key("ctrl+e")?;
    wait("forest open", || visible(&ui, "#forest"))?;
    key("Escape")?;
    wait("forest closed", || !visible(&ui, "#forest"))?;
    key("ctrl+t")?;
    wait("new-tree page", || visible(&ui, "#start"))?;
    assert!(!visible(&ui, "#palette"));
    key("Escape")?;
    wait("new-tree page closed", || !visible(&ui, "#start"))?;
    assert_eq!(ui.snapshot()?.current, Some(first));
    println!("PASS Ctrl+E opens the forest, node selection and Escape close it, and Ctrl+T opens a reversible new-tree page.");

    let count = ui.snapshot()?.visits.len();
    key("ctrl+t")?;
    key("ctrl+k")?;
    wait("palette suggestions", || {
        visible(&ui, "#palette [data-kind=shortcut]") && visible(&ui, "#palette [data-kind=path]")
    })?;
    key("Escape")?;
    wait("palette closed", || !visible(&ui, "#palette"))?;
    click_chrome(&ui, "[data-start=shortcut][data-id='0']")?;
    wait("shortcut plants a root", || {
        ui.snapshot().unwrap().visits.len() == count + 1
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert!(ui.snapshot()?.current_visit().unwrap().parent.is_none());
    key("ctrl+t")?;
    wait("new-tree page", || {
        visible(&ui, "[data-start=path][data-id='0']")
    })?;
    click_chrome(&ui, "[data-start=path][data-id='0']")?;
    wait("usual path loads two pages", || {
        ui.snapshot().unwrap().visits.len() == count + 3
            && !ui.pages.selected_page().native.is_loading()
    })?;
    let snapshot = ui.snapshot()?;
    let second = snapshot.current_visit().unwrap();
    let root = snapshot
        .visits
        .iter()
        .find(|v| Some(v.id) == second.parent)
        .context("usual-path root")?;
    assert!(root.parent.is_none());
    assert!(root.opened_at.is_some());
    assert!(second.opened_at.is_some());
    println!("PASS a shortcut plants a root and a usual path really loads a root and its child.");

    {
        let mut other = canopy::browser::Browser::open(&directory.path().join("forest.sqlite3"))?;
        let page = |path: &str, title: &str| canopy::browser::ImportedPage {
            url: web_url(&fixture.url(path)).unwrap(),
            title: title.into(),
            visits: 4,
            typed: 1,
            last_visit: Some(1),
        };
        other.import_pages(
            "chrome:Default",
            &[page("/imported-history", "Quokka field notes")],
            &[canopy::browser::ImportedBookmark {
                url: web_url(&fixture.url("/imported-bookmark"))?,
                title: "Quokka atlas".into(),
                folder: "Bookmarks bar".into(),
                visits: 0,
            }],
        )?;
    }
    let count = ui.snapshot()?.visits.len();
    key("ctrl+k")?;
    wait("palette focus", || chrome_focused(&ui, "#palette-input"))?;
    type_text("quokka")?;
    wait("imported results", || {
        visible(&ui, "#palette [data-kind=bookmark]")
            && visible(&ui, "#palette [data-kind=imported]")
    })?;
    assert!(text(&ui, "#palette-results").contains("From Chrome"));
    assert_eq!(
        ui.snapshot()?.visits.len(),
        count,
        "imported pages plant nothing by themselves"
    );
    click_chrome(&ui, "#palette [data-kind=imported]")?;
    loaded(&ui, "/imported-history", count + 1)?;
    println!("PASS imported Chrome history and bookmarks show in the palette, and opening one loads it as a visit.");

    let shots = std::env::var("CANOPY_PHASE3_SHOTS").ok();
    for theme in ["light", "dark"] {
        open_settings(&ui)?;
        click_chrome(&ui, &format!("#theme-mode [data-mode={theme}]"))?;
        wait("theme applied", || {
            chrome(&ui, "canopy.state().dark").is_ok_and(|dark| dark == (theme == "dark"))
        })?;
        pump(300);
        let pixels = ui
            .window
            .window()
            .unwrap()
            .pixbuf(4, 4, 1, 1)
            .context("chrome background pixel")?;
        let bytes = pixels.read_pixel_bytes();
        if theme == "dark" {
            assert!(bytes[0] < 50, "dark chrome pixel {:?}", &bytes[..3]);
        } else {
            assert!(bytes[0] > 230, "light chrome pixel {:?}", &bytes[..3]);
        }
        if let Some(dir) = &shots {
            save_window(&ui, &format!("{dir}/theme-menu-{theme}.png"))?;
        }
        key("Escape")?;
        wait("Settings closed", || !visible(&ui, "#popover"))?;
        key("ctrl+e")?;
        pump(250);
        if let Some(dir) = &shots {
            save_window(&ui, &format!("{dir}/forest-titles-{theme}.png"))?;
        }
        click_chrome(&ui, "#forest-titles")?;
        pump(150);
        if let Some(dir) = &shots {
            save_window(&ui, &format!("{dir}/forest-icons-{theme}.png"))?;
        }
        click_chrome(&ui, "#forest-titles")?;
        key("Escape")?;
        key("ctrl+t")?;
        if let Some(dir) = &shots {
            save_window(&ui, &format!("{dir}/start-{theme}.png"))?;
        }
        key("Escape")?;
    }
    println!(
        "PASS the theme buttons in Settings switch the chrome background between light and dark."
    );

    let expected: Snapshot = ui.snapshot()?;
    ui.window.close();
    drop(ui);
    pump(100);
    let reopened = Ui::new(directory.path())?;
    reopened.window.show_all();
    assert_eq!(reopened.snapshot()?, expected);
    reopened.resume(&[])?;
    wait("restart reload", || {
        !reopened.pages.selected_page().native.is_loading()
            && reopened
                .pages
                .selected_page()
                .native
                .uri()
                .is_some_and(|uri| uri.starts_with("http"))
    })?;
    assert_eq!(reopened.snapshot()?, expected);
    reopened.window.close();
    println!("PASS reopening SQLite reloads the URL with titles and branches unchanged.");
    Ok(())
}

fn loaded(ui: &Ui, path: &str, count: usize) -> Result<()> {
    wait(&format!("{path} loaded with {count} visits"), || {
        let snapshot = ui.snapshot().unwrap();
        snapshot.visits.len() == count
            && snapshot
                .current_visit()
                .is_some_and(|visit| visit.url.path() == path && !visit.title.is_empty())
            && !ui.pages.selected_page().native.is_loading()
    })
    .with_context(|| {
        format!(
            "observed {:?}, native URI {:?}",
            ui.snapshot(),
            ui.pages.selected_page().native.uri()
        )
    })
}

fn link(ui: &Ui, id: &str) -> Result<()> {
    eval(
        ui,
        &format!("document.getElementById('{id}').click(); JSON.stringify(true)"),
    )?;
    Ok(())
}

fn trusted_click(ui: &Ui, id: &str) -> Result<()> {
    let (x, y) = element_point(ui, id)?;
    focus_window()?;
    click_at(x, y)
}

struct Fixture {
    address: std::net::SocketAddr,
    stopped: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start(favicon: Vec<u8>) -> Result<Self> {
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
                if path == "/favicon.ico" {
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", favicon.len());
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&favicon);
                    continue;
                }
                let response = if path == "/redirect" {
                    "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                } else {
                    let topic;
                    let title = match path {
                        "/a" => "Research starting point",
                        "/b" => "Trail B",
                        "/c" => "Trail C",
                        "/d" => "Link opened in another window",
                        "/final" => "Destination after a redirect",
                        _ if path.starts_with("/topic-") => {
                            topic = format!("A tree about topic {}", &path[7..]);
                            &topic
                        }
                        _ => "Fixture",
                    };
                    let body = format!("<!doctype html><html lang='en'><meta charset='utf-8'><link rel='icon' href='/favicon.ico'><title>{title}</title><style>body{{font:18px system-ui;max-width:720px;margin:64px auto;color:#294331}}a{{display:inline-block;margin:8px 12px 8px 0;color:#356a48}}h1{{font-weight:550}}p{{line-height:1.7;color:#667568}}</style><h1>{title}</h1><p>A research page to explore several trails and find the way back.</p><input id='q' aria-label='Notes'><a id='b' href='/b'>Explore trail B</a><a id='c' href='/c'>Explore trail C</a><a id='blank' href='/d' target='_blank'>Open another trail</a><a id='same' href='/a'>Revisit this URL</a></html>");
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
