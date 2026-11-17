mod common;

use std::{
    cell::RefCell,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::Result;
use canopy::{pages::Fetched, ui::Ui};
use common::*;
use gtk::prelude::*;
use url::Url;

#[derive(Default)]
struct Seen {
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
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let seen = state.clone();
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
                    seen.lock()
                        .unwrap()
                        .requests
                        .push((path.clone(), cookie, agent));
                    if path == "/stall" {
                        thread::sleep(std::time::Duration::from_secs(35));
                        return;
                    }
                    let (status, headers, body) = if path == "/redirect" {
                        ("302 Found", "Location: /target\r\n", "")
                    } else if path == "/signin" {
                        (
                            "200 OK",
                            "Set-Cookie: SID=fixture; Path=/; HttpOnly\r\n",
                            "<title>Signed in fixture</title>",
                        )
                    } else {
                        ("200 OK", "", "<title>Fetch fixture</title>")
                    };
                    let _ = write!(stream, "HTTP/1.1 {status}\r\n{headers}Content-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        Ok(Self { origin, seen })
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
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
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
    ui.window.close();
    println!("PASS mail fetch: HttpOnly cookies and page User-Agent match, redirects remain unfollowed, cancellation answers once.");
    Ok(())
}
