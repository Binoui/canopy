#[path = "common/http.rs"]
mod http;
use std::{
    collections::BTreeSet,
    io::Write,
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
use serde_json::{json, Value};
use webkit2gtk::WebViewExt;

mod common;

use common::*;

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    let config = directory.path().join("config");
    std::fs::create_dir_all(&config)?;
    std::fs::write(
        config.join("user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Saved files\"\n",
    )?;
    // Text files and folders open with a script that only notes what it was given.
    let data = home.join("share");
    let opened = home.join("opened");
    let opener = home.join("opener.sh");
    std::fs::create_dir_all(data.join("applications"))?;
    std::fs::write(
        &opener,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\n",
            opened.display()
        ),
    )?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::write(
        data.join("applications/canopy-opener.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Opener\nExec=\"{}\" %f\nMimeType=text/plain;inode/directory;\nNoDisplay=true\n",
            opener.display()
        ),
    )?;
    std::fs::write(
        config.join("mimeapps.list"),
        "[Default Applications]\ntext/plain=canopy-opener.desktop\ninode/directory=canopy-opener.desktop\n",
    )?;
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", &config);
    std::env::set_var("XDG_DATA_HOME", &data);
    let downloads = home.join("Saved files");
    start(directory.path())?;
    let fixture = Fixture::start()?;
    let ui = Ui::new(&directory.path().join("data"))?;
    ui.window.show_all();
    ready(&ui)?;
    ui.pages
        .selected_page()
        .navigate(web_url(&fixture.url("/page"))?, NavigationTarget::Root)?;
    loaded(&ui, "/page")?;
    let page = ui.snapshot()?;
    let unchanged = || -> Result<()> {
        let now = ui.snapshot()?;
        assert_eq!(now.current, page.current, "the cursor stays on the page");
        assert_eq!(now.visits, page.visits, "a download creates no visit");
        assert_eq!(
            ui.pages.selected_page().native.uri().as_deref(),
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

    let origin = page.current.unwrap();
    wait("one leaf on the page", || {
        leaves(&ui, origin) == json!([1, 0, null])
    })?;
    let recorded = ui.downloads()?;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].visit, origin);
    assert_eq!(recorded[0].path, downloads.join("report.txt"));
    assert_eq!(recorded[0].size, payload().len() as u64);
    assert_eq!(
        recorded[0].source.as_ref().map(|url| url.as_str()),
        Some(fixture.url("/report").as_str())
    );
    settle(&ui)?;
    let (_, leaf_y, _, leaf_height) = rect(&ui, &leaf_button(origin))?;
    let (_, node_y, _, _) = rect(&ui, &node(origin))?;
    assert!(
        leaf_y >= 42. && (leaf_y + leaf_height - node_y).abs() <= 2.,
        "the leaf stands on its page's top edge, inside the strip: {leaf_y} {leaf_height} {node_y}"
    );
    assert_eq!(
        chrome(
            &ui,
            &format!("document.querySelector({:?}).title", leaf_button(origin))
        )?,
        "Downloaded here: report.txt"
    );
    assert_eq!(chrome(&ui, &format!(
        "(() => {{ const el = document.querySelector({:?}); const r = el.getBoundingClientRect(); return [-11, 11].every(dx => [1, 23].every(dy => {{ const hit = document.elementFromPoint(r.x+r.width/2+dx, r.y+dy); return hit === el || el.contains(hit); }})); }})()", leaf_button(origin)
    ))?, true);
    println!("PASS leaf hit area covers 24 by 24 pixels without enlarging its visible mark.");
    shot(&ui, "one-leaf")?;
    println!("PASS a saved file is recorded on the page it was clicked from, with its path, size and address, and shows as a leaf on that page's node.");

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
    wait("three leaves", || {
        leaves(&ui, origin) == json!([3, 0, null])
    })?;
    shot(&ui, "three-leaves")?;
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
    wait("leaves collapsed to a count", || {
        leaves(&ui, origin) == json!([1, 0, "5"])
    })?;
    assert!(ui.downloads()?.iter().all(|file| file.visit == origin));
    println!("PASS up to three files show a leaf each, and more collapse to one leaf with their count; files saved from the context menu land on the page too.");

    let before = files(&downloads);
    page_click(&ui, "broken")?;
    wait("Downloading toast", || {
        toast_text(&ui).as_deref() == Some("Downloading broken.bin…")
    })?;
    fixture.release.store(true, Ordering::Relaxed);
    wait("failure toast", || {
        error_toast_text(&ui).as_deref()
            == Some(
                "Could not download broken.bin. The connection closed before the whole file arrived.",
            )
    })?;
    pump(300);
    assert_eq!(files(&downloads), before, "no partial file is left behind");
    unchanged()?;
    click_chrome(&ui, "#toast-close")?;
    click_chrome(&ui, "#settings-button")?;
    click_chrome(&ui, "#recent-errors")?;
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('#popover .menu-error')].map((e) => e.firstChild.textContent)"
        )?,
        serde_json::json!([
            "Could not download broken.bin. The connection closed before the whole file arrived."
        ]),
        "no download is reported as a page that failed to load"
    );
    key("Escape")?;
    wait("Recent errors closed", || !visible(&ui, "#popover"))?;
    assert_eq!(leaves(&ui, origin), json!([1, 0, "5"]));
    assert_eq!(ui.downloads()?.len(), 5);
    println!("PASS the user is told when a download starts and when it fails, a failed download leaves no partial file and no leaf, and the only error recorded is that failure.");

    open_files(&ui, origin)?;
    assert_eq!(
        text(&ui, "#popover .menu-heading"),
        "5 files downloaded from this page"
    );
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('#popover .file-row')].map((r) => [r.querySelector('.grow').innerText, r.querySelector('.menu-detail').innerText.split(' · ')[0], r.classList.contains('missing')])"
        )?,
        json!([
            ["report.txt", "200 kB", false],
            ["report (1).txt", "200 kB", false],
            ["data", "200 kB", false],
            ["report (2).txt", "200 kB", false],
            ["pic.png", format!("{} B", png().len()), false],
        ])
    );
    shot(&ui, "files-menu")?;
    let recorded = ui.downloads()?;
    click_chrome(&ui, &format!("#popover [data-file='{}']", recorded[0].id))?;
    let opened_lines = || -> Vec<String> {
        std::fs::read_to_string(&opened)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    wait("the file handed to its app", || opened_lines().len() == 1)?;
    assert_eq!(
        opened_lines(),
        [downloads.join("report.txt").display().to_string()]
    );
    assert!(!visible(&ui, "#popover"));
    open_files(&ui, origin)?;
    click_chrome(&ui, &format!("#popover [data-folder='{}']", recorded[4].id))?;
    wait("the folder handed to its app", || opened_lines().len() == 2)?;
    assert_eq!(opened_lines()[1], downloads.display().to_string());
    unchanged()?;
    println!("PASS the leaf lists the page's files with their size and age; a row opens the file with the desktop's app for it, and its folder button opens the download folder.");

    page_click(&ui, "slow")?;
    wait("Downloading slow.bin toast", || {
        toast_text(&ui).as_deref() == Some("Downloading slow.bin…")
    })?;

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

    let after = visit.id;
    assert_eq!(ui.downloads()?.len(), 5);
    fixture.finish.store(true, Ordering::Relaxed);
    wait("slow.bin saved", || {
        toast_text(&ui).as_deref() == Some("Saved slow.bin in Saved files.")
    })?;
    assert_eq!(std::fs::read(downloads.join("slow.bin"))?, payload());
    wait("six files on the first page", || {
        leaves(&ui, origin) == json!([1, 0, "6"])
    })?;
    assert_eq!(ui.downloads()?[5].visit, origin);
    assert_eq!(leaves(&ui, after), Value::Null);
    println!("PASS a download that ends after the user moved on still lands on the page it started from, now an ancestor in the strip.");

    // A link followed from another page than the one on screen, which redirects to a file.
    ui.pages.selected_page().navigate(
        web_url(&fixture.url("/moved"))?,
        NavigationTarget::ChildOf(origin),
    )?;
    wait("report (3).txt saved", || {
        toast_text(&ui).as_deref() == Some("Saved report (3).txt in Saved files.")
    })?;
    wait("seven files on the first page", || {
        leaves(&ui, origin) == json!([1, 0, "7"])
    })?;
    assert_eq!(ui.downloads()?[6].visit, origin);
    assert_eq!(ui.snapshot()?, next, "the file made no page");
    shot(&ui, "ancestor-count")?;
    println!("PASS a navigation that turns into a download puts its file on the page it started from, not on the page on screen.");

    page_click(&ui, "again")?;
    wait("report (4).txt saved", || {
        toast_text(&ui).as_deref() == Some("Saved report (4).txt in Saved files.")
    })?;
    wait("a leaf on the second page", || {
        leaves(&ui, after) == json!([1, 0, null])
    })?;
    let last = ui.downloads()?.pop().unwrap();
    assert_eq!(last.visit, after);
    assert_eq!(leaves(&ui, origin), json!([1, 0, "7"]));
    std::fs::remove_file(&last.path)?;
    open_files(&ui, after)?;
    assert_eq!(
        text(&ui, "#popover .menu-heading"),
        "Downloaded from this page"
    );
    click_chrome(&ui, &format!("#popover [data-file='{}']", last.id))?;
    wait("missing file toast", || {
        error_toast_text(&ui).as_deref()
            == Some("report (4).txt is no longer in Saved files. It was moved or deleted.")
    })?;
    wait("the leaf drawn as missing", || {
        leaves(&ui, after) == json!([1, 1, null])
    })?;
    assert_eq!(opened_lines().len(), 2, "nothing was opened");
    open_files(&ui, after)?;
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('#popover .file-row')].map((r) => [r.querySelector('.grow').innerText, r.querySelector('.menu-detail').innerText, r.classList.contains('missing')])"
        )?,
        json!([["report (4).txt", "Moved or deleted", true]])
    );
    shot(&ui, "missing")?;
    key("Escape")?;
    wait("files menu closed", || !visible(&ui, "#popover"))?;
    assert_eq!(
        ui.downloads()?.len(),
        8,
        "the record of a missing file stays"
    );
    println!("PASS a file deleted behind Canopy's back is said to be gone when opened, then drawn as a hollow leaf and listed as moved or deleted.");

    // The pointer leaves the strip before the forest changes under it.
    leave_strip(&ui)?;
    ui.pages.selected_page().restore(origin)?;
    loaded(&ui, "/page")?;
    wait("side branch leaves the resting strip", || {
        leaves(&ui, after) == Value::Null
    })?;
    assert_eq!(leaves(&ui, origin), json!([1, 0, "7"]));
    shot(&ui, "branch-rest")?;
    open_list(&ui)?;
    assert!(visible(&ui, &node(after)));
    assert!(visible(
        &ui,
        &format!("#strip-list [data-files='{after}'] .leaf.missing")
    ));
    click_chrome(&ui, &format!("#strip-list [data-files='{after}']"))?;
    wait("side branch files", || visible(&ui, "#popover [data-file]"))?;
    assert!(text(&ui, "#popover").contains("Moved or deleted"));
    shot(&ui, "branch-list")?;
    key("Escape")?;
    wait("side branch file trigger regains focus", || {
        chrome_focused(&ui, &format!("#strip-list [data-files='{after}']"))
    })?;
    key("Escape")?;
    leave_strip(&ui)?;
    println!("PASS at rest the trunk carries leaves; the attached list keeps side branch files reachable.");
    Ok(())
}

/// What the strip draws for the files of `visit`: its leaves, how many of them are drawn as
/// missing, and the count beside them when they are collapsed. Null without any leaf.
fn leaves(ui: &Ui, visit: impl std::fmt::Display) -> Value {
    chrome(
        ui,
        &format!(
            "(() => {{ const b = document.querySelector({:?}); return b && [b.querySelectorAll('.leaf').length, b.querySelectorAll('.leaf.missing').length, b.querySelector('b')?.innerText ?? null]; }})()",
            leaf_button(visit)
        ),
    )
    .unwrap_or_default()
}

/// Saves the window as `name`.png in the folder `CANOPY_DOWNLOAD_SHOTS` names, if it is set.
fn shot(ui: &Ui, name: &str) -> Result<()> {
    match std::env::var("CANOPY_DOWNLOAD_SHOTS") {
        Ok(folder) => save_window(ui, &format!("{folder}/{name}.png")),
        Err(_) => Ok(()),
    }
}

fn leaf_button(visit: impl std::fmt::Display) -> String {
    format!(".leaves[data-id='{visit}']")
}

fn open_files(ui: &Ui, visit: impl std::fmt::Display) -> Result<()> {
    click_chrome(ui, &leaf_button(visit))?;
    wait("files menu", || visible(ui, "#popover .file-row"))?;
    pump(100);
    Ok(())
}

fn loaded(ui: &Ui, path: &str) -> Result<()> {
    wait(&format!("{path} loaded"), || {
        ui.snapshot()
            .unwrap()
            .current_visit()
            .is_some_and(|v| v.url.path() == path && !v.title.is_empty())
            && !ui.pages.selected_page().native.is_loading()
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
    finish: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let finish = Arc::new(AtomicBool::new(false));
        let (stop, released, finished) = (stopped.clone(), release.clone(), finish.clone());
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                // One thread per connection: a held download must not block the next page.
                let (stop, released, finished) = (stop.clone(), released.clone(), finished.clone());
                thread::spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let Ok(bytes) = http::request(&mut stream) else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&bytes);
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
                    "/moved" => {
                        let _ = stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /report\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                    "/slow" => {
                        let body = payload();
                        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"slow.bin\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&body[..65_536]);
                        let _ = stream.flush();
                        while !finished.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(5));
                        }
                        let _ = stream.write_all(&body[65_536..]);
                    }
                    _ => respond(&mut stream, "text/html; charset=utf-8", page(path).as_bytes()),
                }
                });
            }
        });
        Ok(Self {
            address,
            stopped,
            release,
            finish,
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
            r#"<article id="notes" data-permalink="/page#notes"><p>The files for this release.</p><a id="report" href="/report">Download the report</a><a id="blob" href="/data">Raw data</a><a id="broken" href="/broken">A file that breaks halfway</a><a id="slow" href="/slow">A file that takes its time</a><a id="next" href="/next">Next chapter</a><img id="pic" src="/pic.png" width="64" height="64" alt="Cover"></article>"#,
        ),
        "/next" => (
            "Next chapter",
            r#"<main><p>The page after the downloads.</p><a id="again" href="/report">The report again</a></main>"#,
        ),
        _ => ("Fixture", "<main></main>"),
    };
    format!("<!doctype html><html lang='en'><meta charset='utf-8'><title>{title}</title><style>body{{font:18px system-ui;margin:34px}}a{{display:block;margin:16px 0;padding:8px 0}}</style><h1>{title}</h1>{content}</html>")
}
