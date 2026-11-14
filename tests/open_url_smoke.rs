use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, ensure, Result};
use rusqlite::{Connection, OpenFlags};

const CANOPY: &str = env!("CARGO_BIN_EXE_canopy");

fn main() -> Result<()> {
    let server = Server::start()?;
    let one = tempfile::tempdir()?;
    let two = tempfile::tempdir()?;

    let help = Command::new(CANOPY)
        .args(["--data-dir", &one.path().display().to_string(), "--help"])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()?;
    ensure!(help.status.success(), "--help without a display");
    ensure!(String::from_utf8(help.stdout)?.contains("ADDRESS"));
    ensure!(
        !one.path().join("instance.sock").exists(),
        "--help never claims the data directory"
    );
    println!("PASS --help needs no display and claims nothing.");

    let mut first = Canopy::start(one.path(), &[])?;
    first.wait_listening()?;
    let status = launch(one.path(), &[&server.url("/one")])?;
    ensure!(status.success(), "a second launch exits 0, got {status}");
    wait("the first window shows /one as a new tree", || {
        forest(one.path()).is_ok_and(|f| f.roots == [server.url("/one")] && f.shows("/one"))
    })?;
    first.ensure_running()?;
    println!("PASS a second launch on the same data directory opens its URL in the running window and exits 0.");

    let status = launch(one.path(), &[])?;
    ensure!(
        status.success(),
        "a second launch without URL exits 0, got {status}"
    );
    let status = launch(one.path(), &[&server.url("/a"), &server.url("/b")])?;
    ensure!(
        status.success(),
        "a launch with two URLs exits 0, got {status}"
    );
    let expected = [server.url("/one"), server.url("/a"), server.url("/b")];
    wait("two URLs become two trees and the last one shows", || {
        forest(one.path()).is_ok_and(|f| f.roots == expected && f.shows("/b"))
    })?;
    first.ensure_running()?;
    println!("PASS a launch without URL only reaches the window; several URLs become several trees and the last shows.");

    let refused = Command::new(CANOPY)
        .args([
            "--data-dir",
            &one.path().display().to_string(),
            "file:///etc/passwd",
        ])
        .output()?;
    ensure!(!refused.status.success(), "an unsupported address fails");
    ensure!(String::from_utf8(refused.stderr)?.contains("file:///etc/passwd"));
    ensure!(forest(one.path())?.roots == expected, "nothing opened");
    println!("PASS an unsupported address fails with a message and opens nothing.");

    let import = Command::new(CANOPY)
        .args([
            "--data-dir",
            &one.path().display().to_string(),
            "--import-chrome",
        ])
        .arg(one.path().join("no-such-profile"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()?;
    ensure!(
        !import.status.success(),
        "an import runs itself instead of being forwarded"
    );
    first.ensure_running()?;
    ensure!(forest(one.path())?.roots == expected);
    println!(
        "PASS --import-chrome runs without a display while Canopy is open and is never forwarded."
    );

    let mut second = Canopy::start(two.path(), &[&server.url("/two")])?;
    wait("another data directory starts its own window", || {
        forest(two.path()).is_ok_and(|f| f.roots == [server.url("/two")] && f.shows("/two"))
    })?;
    second.ensure_running()?;
    first.ensure_running()?;
    ensure!(
        forest(one.path())?.roots == expected,
        "the first instance never saw /two"
    );
    println!("PASS another data directory is an independent instance.");

    first.crash()?;
    let reloads_before = server.requests("/b");
    let mut first = Canopy::start(one.path(), &[&server.url("/three")])?;
    let mut restored = expected.to_vec();
    restored.push(server.url("/three"));
    wait(
        "a fresh start opens its URL as a new tree over the restored forest",
        || forest(one.path()).is_ok_and(|f| f.roots == restored && f.shows("/three")),
    )?;
    thread::sleep(Duration::from_millis(500));
    ensure!(
        server.requests("/b") == reloads_before,
        "the URL replaced the reload of the last page"
    );
    first.ensure_running()?;
    println!("PASS after a crash, a start with a URL restores the forest and opens the URL instead of reloading the last page.");

    second.stop();
    first.stop();
    Ok(())
}

struct Canopy {
    child: Child,
    data_dir: std::path::PathBuf,
}

impl Canopy {
    fn start(data_dir: &Path, urls: &[&str]) -> Result<Self> {
        let child = Command::new(CANOPY)
            .arg("--data-dir")
            .arg(data_dir)
            .args(urls)
            .stdout(Stdio::null())
            .spawn()?;
        Ok(Self {
            child,
            data_dir: data_dir.to_owned(),
        })
    }

    fn wait_listening(&self) -> Result<()> {
        let socket = self.data_dir.join("instance.sock");
        wait("the instance socket", || socket.exists())
    }

    fn ensure_running(&mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("Canopy on {} exited with {status}", self.data_dir.display());
        }
        Ok(())
    }

    fn crash(mut self) -> Result<()> {
        self.child.kill()?;
        self.child.wait()?;
        Ok(())
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Canopy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn launch(data_dir: &Path, urls: &[&str]) -> Result<ExitStatus> {
    let mut child = Command::new(CANOPY)
        .arg("--data-dir")
        .arg(data_dir)
        .args(urls)
        .stdout(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            bail!("a second launch on {} did not exit", data_dir.display());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

struct Forest {
    roots: Vec<String>,
    current: Option<String>,
}

impl Forest {
    fn shows(&self, path: &str) -> bool {
        self.current
            .as_deref()
            .is_some_and(|url| url.ends_with(path))
    }
}

/// The trees and the current page, read the way the tree strip reads them.
fn forest(data_dir: &Path) -> Result<Forest> {
    let connection = Connection::open_with_flags(
        data_dir.join("forest.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let roots = connection
        .prepare(
            "SELECT v.url FROM tree_state t JOIN visits v ON v.id=t.root_id
             ORDER BY v.created_at, v.rowid",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let current = connection.query_row(
        "SELECT v.url FROM session s LEFT JOIN visits v ON v.id=s.current_id",
        [],
        |row| row.get(0),
    )?;
    Ok(Forest { roots, current })
}

fn wait(label: &str, condition: impl Fn() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        if Instant::now() > deadline {
            bail!("Timed out waiting for {label}");
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

struct Server {
    address: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (log, stop) = (requests.clone(), stopped.clone());
        let thread = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buffer = [0; 8192];
                let size = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..size]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = format!("<!doctype html><title>Page {path}</title><h1>Page {path}</h1>");
                let _ = stream.write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes(),
                );
                log.lock().unwrap().push(path);
            }
        });
        Ok(Self {
            address,
            requests,
            stopped,
            thread: Some(thread),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn requests(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == path)
            .count()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
