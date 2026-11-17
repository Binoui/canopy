mod common;

use std::{
    cell::RefCell,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::Result;
use canopy::{
    chrome::{ChromeHost, ChromeIntent},
    pages::Fetched,
    ui::Ui,
};
use common::*;
use gtk::prelude::*;
use url::Url;

#[derive(Clone, Copy, Default, PartialEq)]
enum Mode {
    #[default]
    Normal,
    Drop,
    Garbage,
    Three,
    Long,
    Gap,
    Trim,
    Empty,
    Slow,
}

#[derive(Default)]
struct Seen {
    mode: Mode,
    slow: bool,
    requests: Vec<(String, String, String)>,
}

struct Fixture {
    origin: String,
    seen: Arc<Mutex<Seen>>,
}

impl Fixture {
    fn serve() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let seen = Arc::new(Mutex::new(Seen::default()));
        let state = seen.clone();
        let server_origin = origin.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let seen = state.clone();
                let origin = server_origin.clone();
                thread::spawn(move || {
                    let mut stream = stream;
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let (mut cookie, mut agent) = (String::new(), String::new());
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some((key, value)) = line.split_once(':') {
                            match key.to_ascii_lowercase().as_str() {
                                "cookie" => cookie = value.trim().to_owned(),
                                "user-agent" => agent = value.trim().to_owned(),
                                _ => (),
                            }
                        }
                    }
                    drop(reader);
                    let signed_in = cookie.contains("SID=fixture");
                    seen.lock()
                        .unwrap()
                        .requests
                        .push((path.clone(), cookie, agent));
                    if path == "/stall" {
                        thread::sleep(std::time::Duration::from_secs(35));
                        return;
                    }
                    let (mode, slow) = {
                        let seen = seen.lock().unwrap();
                        (seen.mode, seen.slow)
                    };
                    let (status, headers, body) = if path.ends_with("/feed/atom") {
                        if mode == Mode::Drop {
                            return;
                        }
                        if mode == Mode::Slow || slow {
                            thread::sleep(Duration::from_millis(1500));
                        }
                        let n: usize = path.split('/').nth(3).and_then(|s| s.parse().ok()).unwrap();
                        let account = account_at(mode, n);
                        if let Some(account) = account.filter(|_| signed_in) {
                            if mode == Mode::Garbage {
                                ("200 OK", "", "not an Atom feed".to_owned())
                            } else {
                                ("200 OK", "", atom(&origin, n, account, mode))
                            }
                        } else if signed_in {
                            (
                                "302 Found",
                                "Location: /mail/u/0/feed/atom\r\n",
                                String::new(),
                            )
                        } else {
                            ("401 Unauthorized", "", String::new())
                        }
                    } else if path == "/redirect" {
                        ("302 Found", "Location: /target\r\n", String::new())
                    } else if path == "/signin" {
                        (
                            "200 OK",
                            "Set-Cookie: SID=fixture; Path=/; HttpOnly\r\n",
                            "<title>Signed in fixture</title>".to_owned(),
                        )
                    } else if path == "/signout" {
                        (
                            "200 OK",
                            "Set-Cookie: SID=; Path=/; Max-Age=0; HttpOnly\r\n",
                            "<title>Signed out fixture</title>".to_owned(),
                        )
                    } else if let Some(address) = path
                        .strip_prefix("/mail/u/")
                        .filter(|rest| rest.contains('@'))
                    {
                        let n = (0..10)
                            .find(|&n| account_at(mode, n) == Some(address))
                            .unwrap_or(0);
                        (
                            "302 Found",
                            &*format!("Location: /mail/u/{n}/\r\n").leak(),
                            String::new(),
                        )
                    } else if path.starts_with("/mail/u/") {
                        (
                            "200 OK",
                            "",
                            "<title>Conversation fixture</title>".to_owned(),
                        )
                    } else {
                        if path == "/slow" {
                            thread::sleep(Duration::from_millis(1500));
                        }
                        ("200 OK", "", format!("<title>Fetch fixture {path}</title>"))
                    };
                    let _ = write!(stream, "HTTP/1.1 {status}\r\n{headers}Content-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        Ok(Self { origin, seen })
    }

    fn set(&self, mode: Mode) {
        self.seen.lock().unwrap().mode = mode;
    }

    fn set_slow(&self, slow: bool) {
        self.seen.lock().unwrap().slow = slow;
    }

    fn feed_hits(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.0.ends_with("/feed/atom"))
            .count()
    }

    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("{}{path}", self.origin)).unwrap()
    }
}

fn fetch(ui: &Ui, url: &Url) -> Result<Fetched> {
    let result = Rc::new(RefCell::new(None));
    let received = result.clone();
    ui.pages.fetch(url, None, move |answer| {
        *received.borrow_mut() = Some(answer);
    });
    assert!(
        result.borrow().is_none(),
        "fetch always answers after returning"
    );
    wait("fetch answer", || result.borrow().is_some())?;
    let answer = result.borrow_mut().take().unwrap();
    answer
}

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let fixture = Fixture::serve()?;
    std::env::set_var("CANOPY_GMAIL_ORIGIN", &fixture.origin);
    let ui = Ui::new(directory.path())?;
    ui.mail.set_gap(Duration::ZERO);
    ui.window.show_all();
    ready(&ui)?;
    wait("first check with no cookie", || ui.mail.mail().read_at > 0)?;
    assert!(
        ui.mail.mail().accounts.is_empty(),
        "no account without a cookie"
    );
    assert!(
        !visible(&ui, "#mail-pin"),
        "no button without a Gmail account"
    );
    ui.open(&[fixture.url("/signin")])?;
    wait("sign-in page loaded", || {
        ui.snapshot()
            .ok()
            .is_some_and(|s| s.visits.iter().any(|v| v.title == "Signed in fixture"))
    })?;
    let answer = fetch(&ui, &fixture.url("/echo"))?;
    assert_eq!(answer.status, 200);
    {
        let seen = fixture.seen.lock().unwrap();
        let page = seen.requests.iter().find(|r| r.0 == "/signin").unwrap();
        let request = seen.requests.iter().find(|r| r.0 == "/echo").unwrap();
        assert!(
            request.1.contains("SID=fixture"),
            "fetch carries the page-set HttpOnly session cookie"
        );
        assert!(!page.2.is_empty(), "page supplied a User-Agent");
        assert_eq!(request.2, page.2, "fetch and page send the same User-Agent");
    }
    let redirect = fetch(&ui, &fixture.url("/redirect"))?;
    assert_eq!(redirect.status, 302);
    assert_eq!(redirect.location.as_deref(), Some("/target"));
    assert!(
        !fixture
            .seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r.0 == "/target"),
        "fetch does not follow redirects"
    );
    let cancel = gio::Cancellable::new();
    let answers = Rc::new(RefCell::new(Vec::new()));
    let received = answers.clone();
    ui.pages
        .fetch(&fixture.url("/stall"), Some(&cancel), move |answer| {
            received.borrow_mut().push(answer.is_err())
        });
    wait("stalled fetch reached server", || {
        fixture
            .seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r.0 == "/stall")
    })?;
    gio::prelude::CancellableExt::cancel(&cancel);
    wait("cancelled fetch answers", || !answers.borrow().is_empty())?;
    pump(100);
    assert_eq!(
        *answers.borrow(),
        [true],
        "cancellation answers exactly once"
    );
    scenario(&ui, &fixture, directory.path())?;
    fixture.set(Mode::Garbage);
    let fresh = tempfile::tempdir()?;
    let fresh_ui = Ui::new(fresh.path())?;
    fresh_ui.window.show_all();
    ready(&fresh_ui)?;
    fresh_ui.open(&[fixture.url("/signin")])?;
    wait("first-ever feed failure", || {
        fresh_ui.mail.mail().trouble.is_some()
    })?;
    assert!(
        fresh_ui.mail.mail().accounts.is_empty(),
        "failed first check discovers no account"
    );
    assert!(
        !visible(&fresh_ui, "#mail-pin"),
        "first-check failure stays hidden"
    );
    fresh_ui.window.close();
    println!("PASS mail first failure: a profile with no successfully read account keeps the button hidden.");
    println!("PASS mail fetch: HttpOnly cookies and page User-Agent match, redirects remain unfollowed, cancellation answers once.");
    Ok(())
}

const SENDER: &str = "Unique mail sender 928374";
const SNIPPET: &str = "Unique mail snippet 837461";

fn account_at(mode: Mode, n: usize) -> Option<&'static str> {
    if mode == Mode::Long {
        return [
            "a.very.long.account.label@personal.example.test",
            "a.very.long.account.label@work.example.test",
            "another.very.long.account.label@example.test",
        ]
        .get(n)
        .copied();
    }
    match n {
        0 => Some("ana@example.test"),
        1 if mode != Mode::Gap => Some("bo@example.test"),
        2 if matches!(mode, Mode::Three | Mode::Gap) => Some("cy@example.test"),
        _ => None,
    }
}

fn atom(origin: &str, n: usize, address: &str, mode: Mode) -> String {
    let ids: &[&str] = if n != 0 || mode == Mode::Empty {
        &[]
    } else if mode == Mode::Trim {
        &["222"]
    } else {
        &["111", "222"]
    };
    let count = if n == 0 { 3 } else { 0 };
    let rows = ids.iter().map(|id| format!("<entry><id>tag:gmail.google.com,2004:{id}</id><title>Tom &amp; Jerry&#39;s lunch</title><summary>{SNIPPET}</summary><author><name>{SENDER}</name></author><issued>2026-11-17T09:12:33Z</issued><link href=\"{origin}/mail/u/{n}?message_id={id}\"/></entry>")).collect::<String>();
    format!("<feed xmlns=\"http://purl.org/atom/ns#\"><title>Inbox for {address}</title><fullcount>{count}</fullcount>{rows}</feed>")
}

fn mail(ui: &Ui) -> serde_json::Value {
    chrome(ui, "canopy.state().mail").unwrap()
}

fn recheck(ui: &Ui) -> Result<()> {
    let previous = ui.mail.mail().read_at;
    ui.mail.check_soon();
    wait("complete good mail check", || {
        ui.mail.mail().read_at != previous && ui.mail.mail().trouble.is_none()
    })?;
    wait("mail frame catches up", || {
        let frame = mail(ui);
        let data = ui.mail.mail();
        let accounts = frame["accounts"].as_array().unwrap();
        accounts.len() == data.accounts.len()
            && accounts.iter().zip(&data.accounts).all(|(drawn, account)| {
                drawn["account"] == account.address
                    && drawn["rows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r["id"].as_str().unwrap())
                        .eq(account.messages.iter().map(|m| m.id.as_str()))
            })
    })
}

fn scenario(ui: &Ui, fixture: &Fixture, data: &std::path::Path) -> Result<()> {
    wait("page sign-in shows three unread", || {
        mail(ui)["pin"]["count"] == "3"
    })?;
    assert_eq!(
        mail(ui)["accounts"].as_array().unwrap().len(),
        2,
        "the first signed-out unknown slot ends the walk"
    );
    click_chrome(ui, "#mail-pin")?;
    wait("mail list opens", || {
        chrome(ui, "canopy.state().popover").unwrap() == "mail"
    })?;
    assert_eq!(
        text(ui, ".mail-row .mail-subject"),
        "Tom & Jerry's lunch",
        "XML entities are decoded"
    );
    assert_eq!(text(ui, ".mail-more"), "1 more in the inbox");
    assert_eq!(
        chrome(ui, "document.querySelector('.mail-row').dataset.account")?,
        "ana@example.test",
        "actions carry an address"
    );
    assert!(visible(ui, ".mail-tabs[role=tablist]"));
    assert_eq!(
        text(ui, ".mail-tab[aria-selected=true] .mail-tab-label"),
        "All"
    );
    click_chrome(ui, ".mail-tab[data-key=\"bo@example.test\"]")?;
    assert_eq!(text(ui, ".mail-inbox .grow"), "bo@example.test");
    assert!(
        !visible(ui, ".mail-row"),
        "the empty account hides other accounts' messages"
    );
    assert!(!visible(ui, ".mail-more"));
    assert_eq!(text(ui, "#mail-pin .count"), "3");
    click_chrome(ui, "#mail-pin")?;
    click_chrome(ui, "#mail-pin")?;
    recheck(ui)?;
    assert_eq!(text(ui, ".mail-inbox .grow"), "bo@example.test");
    assert_eq!(
        text(ui, ".mail-tab[aria-selected=true] .mail-tab-label"),
        "bo"
    );
    click_chrome(ui, ".mail-tab[data-key=\"ana@example.test\"]")?;
    recheck(ui)?;
    assert_eq!(
        chrome(ui, "document.activeElement.getAttribute('aria-selected')")?,
        "true"
    );
    assert_eq!(
        chrome(ui, "document.activeElement.dataset.key")?,
        "ana@example.test"
    );
    mail_tabs_shot(ui, "account")?;
    key("Home")?;
    assert_eq!(
        text(ui, ".mail-tab[aria-selected=true] .mail-tab-label"),
        "All"
    );
    assert!(visible(ui, ".mail-row"));
    mail_tabs_shot(ui, "all")?;
    key("End")?;
    assert_eq!(
        text(ui, ".mail-tab[aria-selected=true] .mail-tab-label"),
        "bo"
    );
    key("Right")?;
    assert_eq!(
        text(ui, ".mail-tab[aria-selected=true] .mail-tab-label"),
        "All"
    );
    key("Left")?;
    assert_eq!(
        text(ui, ".mail-tab[aria-selected=true] .mail-tab-label"),
        "bo"
    );
    key("Home")?;
    fixture.set(Mode::Long);
    recheck(ui)?;
    for theme in ["light", "dark"] {
        chrome(ui, &format!("intent('theme', {{mode:'{theme}'}}), true"))?;
        pump(100);
        assert_eq!(chrome(ui, "(() => { const p = document.querySelector('#popover'); const tabs = [...p.querySelectorAll('.mail-tab')]; return tabs.length === 4 && p.offsetWidth >= 360 && p.offsetWidth <= 440 && p.scrollWidth <= p.clientWidth && tabs.every(t => t.getBoundingClientRect().top === tabs[0].getBoundingClientRect().top && t.scrollWidth <= t.clientWidth); })()")?, true, "three long account tabs fit on one line without scrolling");
    }
    mail_tabs_shot(ui, "long-accounts")?;
    fixture.set(Mode::Normal);
    recheck(ui)?;
    println!("PASS mail tabs: account filtering, total badge, session selection, refresh focus and keyboard navigation.");
    let trees = ui
        .snapshot()?
        .visits
        .iter()
        .filter(|v| v.parent.is_none())
        .count();
    click_chrome(ui, ".mail-row")?;
    wait("conversation opens as a new tree", || {
        ui.snapshot().ok().is_some_and(|s| {
            s.visits.iter().filter(|v| v.parent.is_none()).count() == trees + 1
                && s.current
                    .and_then(|id| s.visits.iter().find(|v| v.id == id))
                    .is_some_and(|v| {
                        v.title == "Conversation fixture" && v.url.query() == Some("message_id=111")
                    })
        })
    })?;
    assert!(
        chrome(ui, "canopy.state().popover")?.is_null(),
        "row closes the list"
    );
    println!("PASS mail list: grouped accounts, decoded text, overflow line, address-keyed row opens a new tree.");

    for (mode, notice) in [
        (Mode::Drop, "Could not reach Gmail"),
        (Mode::Garbage, "Gmail's feed changed"),
    ] {
        fixture.set(mode);
        click_chrome(ui, "#mail-pin")?;
        wait("failed check keeps rows and explains stale data", || {
            mail(ui)["notice"]
                .as_str()
                .is_some_and(|s| s.starts_with(notice))
        })?;
        assert_eq!(mail(ui)["pin"]["stale"], true);
        assert_eq!(mail(ui)["accounts"][0]["rows"].as_array().unwrap().len(), 2);
        click_chrome(ui, "#mail-pin")?;
    }
    fixture.set(Mode::Normal);
    recheck(ui)?;
    wait("good check clears stale notice", || {
        mail(ui)["notice"].is_null()
    })?;
    println!("PASS mail failures: network and changed feeds preserve the last list, and a good check clears the notice.");

    ui.dispatch(ChromeIntent::OpenPath {
        first: fixture.url("/slow"),
        second: fixture.url("/continued"),
    });
    assert!(
        ui.pages.has_pending_continuation(),
        "path has a pending continuation"
    );
    click_chrome(ui, "#mail-pin")?;
    assert!(
        ui.pages.has_pending_continuation(),
        "opening the list preserves the pending continuation"
    );
    wait("pending path reaches its second page", || {
        ui.snapshot()
            .ok()
            .is_some_and(|s| s.visits.iter().any(|v| v.url.path() == "/continued"))
    })?;
    println!("PASS mail continuation: opening the list preserves pending navigation.");

    chrome(
        ui,
        "document.querySelector('.mail-row[data-id=\"222\"]').focus(), true",
    )?;
    fixture.set(Mode::Trim);
    recheck(ui)?;
    wait("focus follows the same message after rows shift", || {
        chrome(ui, "document.activeElement.dataset.id").unwrap() == "222"
    })?;
    fixture.set(Mode::Empty);
    recheck(ui)?;
    wait("removed focused row falls back to popover", || {
        chrome(ui, "document.activeElement.id").unwrap() == "popover"
    })?;
    chrome(
        ui,
        "document.querySelectorAll('.mail-inbox')[1].focus(), true",
    )?;
    fixture.set(Mode::Three);
    recheck(ui)?;
    wait("new account above previous maximum appears", || {
        mail(ui)["accounts"].as_array().unwrap().len() == 3
    })?;
    assert_eq!(
        chrome(ui, "document.activeElement.dataset.account")?,
        "bo@example.test",
        "account-header focus follows the address"
    );
    click_chrome(ui, ".mail-tab[data-key=\"bo@example.test\"]")?;
    chrome(ui, "document.querySelector('.mail-inbox').focus(), true")?;
    fixture.set(Mode::Gap);
    recheck(ui)?;
    wait("gap walk keeps the account above the hole", || {
        mail(ui)["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["account"].as_str().unwrap())
            .collect::<Vec<_>>()
            == ["ana@example.test", "cy@example.test"]
    })?;
    assert_eq!(
        chrome(ui, "document.activeElement.id")?,
        "popover",
        "removed header falls back to popover"
    );
    assert_eq!(
        text(ui, ".mail-tab[aria-selected=true] .mail-tab-label"),
        "All"
    );
    assert!(visible(
        ui,
        ".mail-inbox[data-account=\"ana@example.test\"]"
    ));
    assert!(visible(ui, ".mail-inbox[data-account=\"cy@example.test\"]"));
    let before = ui.snapshot()?.visits.len();
    chrome(ui, "window.ipc.postMessage(JSON.stringify({t:'intent',kind:'open-inbox',account:'bo@example.test'})), true")?;
    pump(200);
    assert_eq!(
        ui.snapshot()?.visits.len(),
        before,
        "departed address cannot open a reused account slot"
    );
    click_chrome(ui, ".mail-inbox[data-account=\"cy@example.test\"]")?;
    wait("inbox action uses the retained account's slot", || {
        ui.snapshot().ok().is_some_and(|s| {
            s.current
                .and_then(|id| s.visits.iter().find(|v| v.id == id))
                .is_some_and(|v| v.url.path() == "/mail/u/2/")
        })
    })?;
    println!("PASS mail discovery and focus: new accounts and gaps work, focus follows identities, inbox actions resolve addresses.");

    fixture.set_slow(true);
    let hits = fixture.feed_hits();
    ui.mail.check_soon();
    wait("a slow check is in flight", || fixture.feed_hits() > hits)?;
    ui.open(&[fixture.url("/signout")])?;
    wait(
        "a sign-out during a check gets its own check, which hides mail",
        || {
            ui.mail.mail().accounts.is_empty()
                && toast_text(ui).as_deref() == Some("Signed out of Gmail as ana@example.test.")
        },
    )?;
    fixture.set_slow(false);
    assert!(!visible(ui, "#mail-pin"), "last sign-out hides the pin");
    assert!(
        !mentions(data, &[SENDER, SNIPPET])?,
        "no feed text anywhere under the data folder"
    );
    println!("PASS mail sign-out: pin hides, toast names addresses, no feed text is stored under the data folder.");

    fixture.set(Mode::Garbage);
    ui.open(&[fixture.url("/signin")])?;
    ui.mail.check_soon();
    wait("failed check without accounts", || {
        ui.mail.mail().trouble.is_some()
    })?;
    assert!(
        !visible(ui, "#mail-pin"),
        "failure without accounts stays hidden"
    );
    fixture.set(Mode::Slow);
    let hits = fixture.feed_hits();
    ui.mail.check_soon();
    wait("watch has an in-flight request", || {
        fixture.feed_hits() > hits
    })?;
    let mail_before = ui.mail.mail();
    ui.window.close();
    ui.mail.check_soon();
    pump(1800);
    assert_eq!(
        ui.mail.mail(),
        mail_before,
        "destroyed window ignores late answers"
    );
    assert_eq!(
        fixture.feed_hits(),
        hits + 1,
        "destroyed window stops the walk and future checks"
    );
    println!(
        "PASS mail stop: window destruction cancels an in-flight walk and ignores late answers."
    );
    Ok(())
}

fn mentions(path: &std::path::Path, needles: &[&str]) -> Result<bool> {
    for entry in std::fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            if mentions(&path, needles)? {
                return Ok(true);
            }
        } else if let Ok(bytes) = std::fs::read(&path) {
            if needles.iter().any(|needle| {
                bytes
                    .windows(needle.len())
                    .any(|part| part == needle.as_bytes())
            }) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn mail_tabs_shot(ui: &Ui, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CANOPY_MAIL_TABS_SHOTS") {
        chrome(ui, "intent('theme', {mode:'dark'}), true")?;
        pump(450);
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory)?;
        anyhow::ensure!(
            std::process::Command::new("import")
                .args(["-window", "root"])
                .arg(directory.join(format!("{name}-dark.png")))
                .status()?
                .success(),
            "mail tabs screenshot"
        );
    }
    Ok(())
}
