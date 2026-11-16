//! The scripted run: drives the prototype through each question we need answered on a Mac and
//! writes what it saw to `report.md`, `report.json` and a few screenshots.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::{eval, App, STRIP};
use crate::native;

struct Report {
    rows: Vec<(String, &'static str, String)>,
    failures: u32,
}

impl Report {
    fn check(&mut self, name: &str, passed: bool, detail: impl Into<String>) {
        if !passed {
            self.failures += 1;
        }
        let detail = detail.into();
        println!("[{}] {name}: {detail}", if passed { "pass" } else { "FAIL" });
        self.rows.push((name.to_string(), if passed { "pass" } else { "FAIL" }, detail));
    }

    fn info(&mut self, name: &str, detail: impl Into<String>) {
        let detail = detail.into();
        println!("[info] {name}: {detail}");
        self.rows.push((name.to_string(), "info", detail));
    }

    fn section(&mut self, title: &str) {
        println!("\n== {title}");
        self.rows.push((title.to_string(), "section", String::new()));
    }

    fn write(&self, out: &Path) {
        let mut markdown = String::from("# Canopy Mac prototype: scripted run\n");
        for (name, kind, detail) in &self.rows {
            match *kind {
                "section" => markdown.push_str(&format!("\n## {name}\n\n")),
                _ => markdown.push_str(&format!("- **{kind}** {name}: {detail}\n")),
            }
        }
        let rows: Vec<Value> = self
            .rows
            .iter()
            .filter(|(_, kind, _)| *kind != "section")
            .map(|(name, kind, detail)| json!({ "name": name, "result": kind, "detail": detail }))
            .collect();
        let _ = std::fs::write(out.join("report.md"), markdown);
        let _ = std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&rows).unwrap());
    }
}

fn settle(seconds: f64) {
    native::wait(Duration::from_secs_f64(seconds), || false);
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}

/// Waits until a slot's page has finished `loads` loads in total and shows `title`.
fn loaded(app: &App, slot: usize, loads: u32, title: &str, seconds: u64) -> bool {
    native::wait(Duration::from_secs(seconds), || {
        let page = app.page(slot);
        page.log.finished.get() >= loads && !native::is_loading(&page.view) && native::title(&page.view) == title
    })
}

fn counters(app: &App, slot: usize) -> Value {
    eval(&app.page(slot).view, "window.__proto")
}

fn chrome_clicks(app: &App) -> f64 {
    number(&eval(&app.chrome, "window.__proto.clicks"))
}

fn shot(app: &App, out: &Path, name: &str, report: &mut Report) {
    settle(0.4);
    let saved = native::screenshot(&app.ns_window, &out.join(name));
    report.info(&format!("screenshot {name}"), if saved { "saved" } else { "refused by the system" });
}

pub fn run(app: &mut App, out: &Path) -> u32 {
    let _ = std::fs::create_dir_all(out);
    let mut report = Report { rows: Vec::new(), failures: 0 };
    app.window.set_focus();
    settle(0.5);
    let (width, height) = app.size();
    report.info("window", format!("{width:.0} x {height:.0} points, scale {}", app.window.scale_factor()));

    stack(app, out, &mut report);
    background(app, out, &mut report);
    history(app, out, &mut report);
    memory(app, &mut report);
    video(app, out, &mut report);

    report.write(out);
    println!("\n{} failure(s)", report.failures);
    report.failures
}

/// Chrome over pages: drawing order, transparency, and which view gets a click.
fn stack(app: &mut App, out: &Path, report: &mut Report) {
    report.section("Chrome over a page");
    let first = app.site.url("/p/1");
    app.open(0, Some(&first), true);
    app.select(0);
    app.layout();
    report.check("a page loads in a child view", loaded(app, 0, 1, "Page 1", 15), native::title(&app.page(0).view));
    let chrome_ready = native::wait(Duration::from_secs(10), || eval(&app.chrome, "document.readyState") == json!("complete"));
    report.check("the chrome page loads", chrome_ready, "");
    app.publish();
    shot(app, out, "1-strip-over-page.png", report);

    let (width, height) = app.size();
    let views = [&app.chrome, &app.page(0).view];
    let in_strip = native::hit(&app.ns_window, 60.0, STRIP / 2.0, &views);
    let in_page = native::hit(&app.ns_window, width / 2.0, height / 2.0, &views);
    report.check("a point in the strip belongs to the chrome", in_strip == Some(0), format!("{in_strip:?}"));
    report.check("a point below the strip belongs to the page", in_page == Some(1), format!("{in_page:?}"));

    native::click(&app.ns_window, width / 2.0, height / 2.0);
    settle(0.3);
    let page_clicks = number(&counters(app, 0)["clicks"]);
    report.check("a click on the page reaches the page", page_clicks == 1.0, format!("page saw {page_clicks} click(s)"));
    native::click(&app.ns_window, 60.0, STRIP / 2.0);
    settle(0.3);
    let strip_clicks = chrome_clicks(app);
    report.check("a click on the strip reaches the chrome", strip_clicks == 1.0, format!("chrome saw {strip_clicks} click(s)"));

    app.expanded = true;
    app.note = "Scripted run: overlay open.".into();
    app.layout();
    app.publish();
    shot(app, out, "2-overlay-over-page.png", report);
    let views = [&app.chrome, &app.page(0).view];
    let covered = native::hit(&app.ns_window, 80.0, height - 80.0, &views);
    report.check("with the overlay open, an empty area belongs to the chrome", covered == Some(0), format!("{covered:?}"));
    native::click(&app.ns_window, 80.0, height - 80.0);
    settle(0.3);
    let page_after = number(&counters(app, 0)["clicks"]);
    let chrome_after = chrome_clicks(app);
    report.check(
        "a click on the overlay's empty area stays in the chrome",
        chrome_after == strip_clicks + 1.0 && page_after == page_clicks,
        format!("chrome {chrome_after}, page {page_after}"),
    );

    app.expanded = false;
    app.layout();
    app.publish();
    settle(0.3);
    let views = [&app.chrome, &app.page(0).view];
    let released = native::hit(&app.ns_window, 80.0, height - 80.0, &views);
    report.check("with the overlay closed, the same area belongs to the page again", released == Some(1), format!("{released:?}"));
}

/// A page that loads without ever being shown, then switching between live pages.
fn background(app: &mut App, out: &Path, report: &mut Report) {
    report.section("Background loading and switching");
    let url = app.site.url("/p/10");
    let begun = Instant::now();
    app.open(1, Some(&url), false);
    let done = loaded(app, 1, 1, "Page 10", 15);
    report.check("a page that was never shown loads fully", done, format!("{} ms", begun.elapsed().as_millis()));
    let views = [&app.chrome, &app.page(0).view, &app.page(1).view];
    let strip = native::hit(&app.ns_window, 60.0, STRIP / 2.0, &views);
    report.check("the chrome is still on top after a new page view", strip == Some(0), format!("{strip:?}"));

    let before = counters(app, 1);
    settle(2.0);
    let after = counters(app, 1);
    report.info(
        "a never-shown page over 2 s",
        format!(
            "{} timer ticks of 20 expected, {} animation frames",
            number(&after["ticks"]) - number(&before["ticks"]),
            number(&after["frames"]) - number(&before["frames"])
        ),
    );

    let frames = number(&counters(app, 1)["frames"]);
    let begun = Instant::now();
    app.select(1);
    let drew = native::wait(Duration::from_secs(3), || number(&counters(app, 1)["frames"]) > frames);
    report.check("a hidden page draws once shown", drew, format!("first new frame within {} ms", begun.elapsed().as_millis()));
    app.publish();
    shot(app, out, "3-switched-to-second-page.png", report);

    let before = counters(app, 0);
    settle(2.0);
    let after = counters(app, 0);
    report.info(
        "a page hidden after being shown, over 2 s",
        format!(
            "{} timer ticks of 20 expected, {} animation frames",
            number(&after["ticks"]) - number(&before["ticks"]),
            number(&after["frames"]) - number(&before["frames"])
        ),
    );

    for round in 0..10 {
        app.select(round % 2);
        settle(0.1);
    }
    app.select(0);
    settle(0.3);
    let (width, height) = app.size();
    let views = [&app.chrome, &app.page(0).view, &app.page(1).view];
    let shown = native::hit(&app.ns_window, width / 2.0, height / 2.0, &views);
    report.check("after ten quick switches the selected page is the one on screen", shown == Some(1), format!("{shown:?}"));
}

/// Save a page's history, destroy the page, rebuild it, and see what came back.
fn history(app: &mut App, out: &Path, report: &mut Report) {
    report.section("Save and restore a page's history");
    app.select(0);
    for (step, title) in [(2, "Page 2"), (3, "Page 3")] {
        eval(&app.page(0).view, "document.getElementById('next').click(); 0");
        if !loaded(app, 0, step, title, 15) {
            report.check("following links", false, format!("never reached {title}"));
            return;
        }
    }
    eval(&app.page(0).view, "document.getElementById('note').value = 'hello canopy'; scrollTo(0, 1200); 0");
    settle(0.6);
    let before = native::history(&app.page(0).view);
    report.check("the back list counts two pages", before.back == 2 && before.can_go_back, format!("back {}, forward {}", before.back, before.forward));

    let fetched = app.site.hits("/p/3");
    let process = app.drop_page(0, true);
    match &app.saved[0] {
        Some(saved) => report.check(
            "the page hands over its saved history",
            true,
            match saved.bytes {
                Some(bytes) => format!("{} of {bytes} bytes, so it can also be written to disk", saved.class),
                None => format!("an object of class {}, not plain bytes", saved.class),
            },
        ),
        None => {
            report.check("the page hands over its saved history", false, "nothing returned");
            return;
        }
    }
    settle(1.5);
    if let Some(pid) = process {
        report.check("closing a page ends its process", !native::process_alive(pid), format!("process {pid}"));
    }

    app.revive(0, true);
    let back = loaded(app, 0, 1, "Page 3", 15);
    report.check("the rebuilt page returns to the same address", back, app.page(0).view.url().unwrap_or_default());
    settle(0.6);
    let after = native::history(&app.page(0).view);
    report.check("the back list survives", after.back == 2, format!("back {}, forward {}", after.back, after.forward));
    let scroll = number(&eval(&app.page(0).view, "scrollY"));
    report.check("the scroll position survives", (scroll - 1200.0).abs() < 60.0, format!("{scroll} of 1200"));
    let note = eval(&app.page(0).view, "document.getElementById('note').value");
    report.info("text typed in a field", format!("came back as {note}"));
    report.info(
        "the restore fetched the page from the site again",
        if app.site.hits("/p/3") > fetched { "yes" } else { "no" },
    );
    app.publish();
    shot(app, out, "4-rebuilt-page.png", report);

    native::go_back(&app.page(0).view);
    let returned = native::wait(Duration::from_secs(10), || native::title(&app.page(0).view) == "Page 2" && !native::is_loading(&app.page(0).view));
    let list = native::history(&app.page(0).view);
    report.check("Back works on the rebuilt page", returned && list.back == 1 && list.forward == 1, format!("back {}, forward {}", list.back, list.forward));

    // Same again, but the page is rebuilt hidden, as when Canopy revives a page in the background.
    settle(0.5);
    eval(&app.page(0).view, "scrollTo(0, 2400); 0");
    settle(0.6);
    app.drop_page(0, true);
    app.select(1);
    app.revive(0, false);
    let hidden = loaded(app, 0, 1, "Page 2", 15);
    settle(0.6);
    let hidden_scroll = number(&eval(&app.page(0).view, "scrollY"));
    report.check("a page rebuilt while hidden loads", hidden, format!("scroll while hidden {hidden_scroll} of 2400"));
    app.select(0);
    settle(0.6);
    let shown_scroll = number(&eval(&app.page(0).view, "scrollY"));
    report.check("its scroll position is right once shown", (shown_scroll - 2400.0).abs() < 60.0, format!("{shown_scroll} of 2400"));

    // What wry does on its own when a view is dropped, without the explicit close.
    let url = app.site.url("/p/20");
    app.open(2, Some(&url), false);
    loaded(app, 2, 1, "Page 20", 15);
    let process = app.drop_page(2, false);
    app.saved[2] = None;
    settle(2.0);
    if let Some(pid) = process {
        report.info(
            "dropping a wry view without closing it",
            if native::process_alive(pid) { "its process stays alive: wry never frees the page" } else { "its process ends" },
        );
    }
}

/// Memory per live page, on real sites.
fn memory(app: &mut App, report: &mut Report) {
    report.section("Memory");
    let sites = [
        (1, "https://en.wikipedia.org/wiki/Tree_(graph_theory)", "a Wikipedia article"),
        (2, "https://www.youtube.com/watch?v=aqz-KE-bpKQ", "a YouTube video page"),
    ];
    for (slot, url, _) in sites {
        if app.pages[slot].is_some() {
            app.drop_page(slot, true);
            app.saved[slot] = None;
        }
        app.open(slot, Some(url), slot == 2);
    }
    app.select(2);
    native::wait(Duration::from_secs(30), || {
        sites.iter().all(|(slot, _, _)| app.page(*slot).log.finished.get() >= 1)
    });
    settle(8.0);

    let mut processes = Vec::new();
    let mut describe = |app: &App, slot: usize, what: &str, report: &mut Report| {
        let page = app.page(slot);
        let finished = page.log.finished.get() >= 1;
        let process = native::page_process(&page.view);
        let memory = process.and_then(native::footprint_mib);
        processes.extend(process);
        report.info(
            what,
            match (finished, process, memory) {
                (false, _, _) => "did not finish loading (no network?)".to_string(),
                (true, Some(pid), Some(mib)) => format!("{mib:.0} MiB in process {pid}"),
                (true, _, _) => "memory not readable".to_string(),
            },
        );
    };
    describe(app, 0, "the local test page", report);
    describe(app, 1, "a Wikipedia article, loaded hidden", report);
    describe(app, 2, "a YouTube video page, on screen", report);
    processes.sort();
    processes.dedup();
    report.info("page processes for three live pages", format!("{} distinct: {processes:?}", processes.len()));

    let own = native::footprint_mib(std::process::id() as i32).unwrap_or(f64::NAN);
    report.info("the prototype's own process", format!("{own:.0} MiB"));
    for (name, pid) in native::helper_processes() {
        if let Some(mib) = native::footprint_mib(pid) {
            report.info(&format!("shared helper {name}"), format!("{mib:.0} MiB in process {pid}"));
        }
    }
}

/// Video and sound, on screen and hidden.
fn video(app: &mut App, out: &Path, report: &mut Report) {
    report.section("Video");
    app.drop_page(2, true);
    app.saved[2] = None;
    let url = app.site.url("/video");
    app.open(2, Some(&url), true);
    app.select(2);
    if !loaded(app, 2, 1, "Video", 15) {
        report.check("the video page loads", false, "");
        return;
    }
    let time = |app: &App| number(&eval(&app.page(2).view, "document.getElementById('v').currentTime"));
    let playing = native::wait(Duration::from_secs(8), || time(app) > 0.5);
    let size = eval(&app.page(2).view, "[v.videoWidth, v.videoHeight, v.paused, v.muted]");
    report.check("an H.264 video with sound plays without a click", playing, format!("at {:.1} s, [width, height, paused, muted] = {size}", time(app)));
    report.info("the page reports it is making sound", format!("{:?}", native::playing_audio(&app.page(2).view)));
    app.publish();
    shot(app, out, "5-video.png", report);

    app.select(0);
    settle(0.5);
    let frames_before = number(&eval(&app.page(2).view, "v.getVideoPlaybackQuality().totalVideoFrames"));
    let paused_at = number(&eval(&app.page(2).view, "v.currentTime + (v.loop ? 0 : 0)"));
    settle(2.0);
    let frames_after = number(&eval(&app.page(2).view, "v.getVideoPlaybackQuality().totalVideoFrames"));
    let paused = eval(&app.page(2).view, "v.paused");
    report.info(
        "the same video once its page is hidden, over 2 s",
        format!(
            "paused = {paused}, {} more video frames decoded, sound reported = {:?}, position was {paused_at:.1} s",
            frames_after - frames_before,
            native::playing_audio(&app.page(2).view)
        ),
    );
}
