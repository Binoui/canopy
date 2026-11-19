//! One Canopy per data directory. The process holding `instance.lock` owns the profile and
//! listens on `instance.sock`; any later launch on that directory hands its request over
//! and exits. The kernel drops the lock when its holder dies, so a crash never blocks a start.
//! The same socket answers questions about the running window, such as `canopy session`.

use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::{
            fs::{OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use url::Url;

use crate::address::web_url;

const ACCEPTED: &[u8] = b"ok\n";
const MAX_REQUEST: u64 = 1 << 20;

/// What a launch asks of the Canopy that owns its data directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Request {
    pub urls: Vec<Url>,
    /// The launching app's startup-notification or activation token, so focus can move.
    pub activation: Option<String>,
}

/// A question for the running Canopy, answered later through a [`Reply`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Question {
    /// The trees and pages on screen, and with `text` the shown page's visible text.
    Session { text: bool },
}

impl Question {
    fn encode(self) -> Vec<u8> {
        match self {
            Self::Session { text } => json!({ "query": "session", "text": text }),
        }
        .to_string()
        .into_bytes()
    }

    fn decode(value: &Value) -> Option<Result<Self>> {
        let query = value.get("query")?;
        Some(match query.as_str() {
            Some("session") => Ok(Self::Session {
                text: value["text"].as_bool().unwrap_or(false),
            }),
            _ => Err(anyhow!("Unknown question: {query}.")),
        })
    }
}

/// The way back to the process that asked a [`Question`]. Dropping it unsent closes the
/// connection, which the asker reports as no answer.
pub struct Reply(UnixStream);

impl Reply {
    /// Sends `answer`, or the error in its place, as one JSON object.
    pub fn send(mut self, answer: Result<Value>) {
        let body = match answer {
            Ok(value) => json!({ "answer": value }),
            Err(error) => json!({ "error": format!("{error:#}") }),
        };
        // The asker may have given up; nothing else is waiting on this connection.
        let _ = self.0.write_all(body.to_string().as_bytes());
    }
}

pub enum Launch {
    /// This process owns the data directory and must open the window.
    Primary(Primary),
    /// The running Canopy accepted the request.
    Forwarded,
}

pub struct Primary {
    listener: UnixListener,
    _lock: File,
}

impl Request {
    fn encode(&self) -> Vec<u8> {
        let urls: Vec<&str> = self.urls.iter().map(Url::as_str).collect();
        json!({ "urls": urls, "activation": self.activation })
            .to_string()
            .into_bytes()
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let value: Value = serde_json::from_slice(bytes)?;
        let urls = value["urls"]
            .as_array()
            .context("A launch request needs a URL list.")?
            .iter()
            .map(|url| web_url(url.as_str().context("A launch URL must be text.")?))
            .collect::<Result<_>>()?;
        let activation = value["activation"].as_str().map(str::to_owned);
        Ok(Self { urls, activation })
    }
}

/// Becomes the Canopy for `data_dir`, or hands `request` to the one already running there.
pub fn launch(data_dir: &Path, request: &Request) -> Result<Launch> {
    let lock_path = data_dir.join("instance.lock");
    // A socket address holds about 100 bytes. Naming the socket through a handle on its
    // directory keeps the address short whatever the data directory's path is.
    let directory =
        File::open(data_dir).with_context(|| format!("Could not open {}.", data_dir.display()))?;
    let socket_path = PathBuf::from(format!(
        "/proc/self/fd/{}/instance.sock",
        directory.as_raw_fd()
    ));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .with_context(|| format!("Could not open {}.", lock_path.display()))?;
    // The owner may still be starting, between taking the lock and listening, or may be
    // quitting, in which case this launch takes over.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match lock.try_lock() {
            Ok(()) => {
                match std::fs::remove_file(&socket_path) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => {
                        return Err(error).context("Could not remove the stale instance socket.")
                    }
                    _ => {}
                }
                let listener = UnixListener::bind(&socket_path).with_context(|| {
                    format!(
                        "Could not listen at {}.",
                        data_dir.join("instance.sock").display()
                    )
                })?;
                std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
                listener.set_nonblocking(true)?;
                return Ok(Launch::Primary(Primary {
                    listener,
                    _lock: lock,
                }));
            }
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => {
                return Err(error).context("Could not lock the data directory.")
            }
        }
        match UnixStream::connect(&socket_path) {
            Ok(stream) => return forward(stream, request).map(|()| Launch::Forwarded),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(error)
                    .context("Canopy is open on this data directory but did not answer.")
            }
        }
    }
}

fn forward(mut stream: UnixStream, request: &Request) -> Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(&request.encode())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reply = Vec::new();
    stream
        .read_to_end(&mut reply)
        .context("Canopy is open on this data directory but did not answer.")?;
    if reply == ACCEPTED {
        return Ok(());
    }
    if reply.is_empty() {
        bail!("Canopy is open on this data directory but did not answer.");
    }
    bail!(
        "Canopy is open on this data directory but could not open the addresses. {}",
        String::from_utf8_lossy(&reply).trim()
    );
}

/// Asks the Canopy running on `data_dir`. Never starts one.
pub fn ask(data_dir: &Path, question: Question) -> Result<Value> {
    let Ok(directory) = File::open(data_dir) else {
        bail!("Canopy is not running on {}.", data_dir.display())
    };
    let socket_path = PathBuf::from(format!(
        "/proc/self/fd/{}/instance.sock",
        directory.as_raw_fd()
    ));
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            bail!("Canopy is not running on {}.", data_dir.display())
        }
        Err(error) => return Err(error).context("Could not reach Canopy."),
    };
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(&question.encode())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reply = Vec::new();
    stream
        .read_to_end(&mut reply)
        .context("Canopy did not answer.")?;
    if reply.is_empty() {
        bail!("Canopy did not answer. A Canopy started before this version cannot; restart it.");
    }
    let mut reply: Value =
        serde_json::from_slice(&reply).context("Canopy answered something unreadable.")?;
    if let Some(error) = reply["error"].as_str() {
        bail!("{error}");
    }
    Ok(reply["answer"].take())
}

impl Primary {
    /// Hands every waiting launch's request to `open`, then tells that launch how it went,
    /// and every waiting question to `ask` with the way to answer it.
    pub fn answer(
        &self,
        mut open: impl FnMut(Request) -> Result<()>,
        mut ask: impl FnMut(Question, Reply),
    ) -> Result<()> {
        loop {
            let mut stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            let mut bytes = Vec::new();
            (&mut stream).take(MAX_REQUEST).read_to_end(&mut bytes)?;
            let question = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| Question::decode(&value));
            match question {
                Some(Ok(question)) => {
                    ask(question, Reply(stream));
                    continue;
                }
                Some(Err(error)) => {
                    Reply(stream).send(Err(error));
                    continue;
                }
                None => {}
            }
            let request = Request::decode(&bytes)
                .map_err(|error| anyhow!("Ignored a launch request. {error}"))?;
            match open(request) {
                Ok(()) => stream.write_all(ACCEPTED)?,
                Err(error) => stream.write_all(format!("{error:#}").as_bytes())?,
            }
        }
    }
}

impl AsRawFd for Primary {
    fn as_raw_fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(paths: &[&str]) -> Request {
        Request {
            urls: paths
                .iter()
                .map(|p| Url::parse(&format!("https://example.org/{p}")).unwrap())
                .collect(),
            activation: Some("canopy-test_TIME42".into()),
        }
    }

    fn primary(launch: Launch) -> Primary {
        match launch {
            Launch::Primary(primary) => primary,
            Launch::Forwarded => panic!("expected to own the data directory"),
        }
    }

    /// Launches on `path` from another thread while `first` answers with `open`.
    fn second_launch(
        first: &Primary,
        path: &Path,
        sent: &Request,
        mut open: impl FnMut(Request) -> Result<()>,
    ) -> Result<Launch> {
        let (path, sent) = (path.to_owned(), sent.clone());
        let second = thread::spawn(move || launch(&path, &sent));
        while !second.is_finished() {
            first
                .answer(&mut open, |_, _| panic!("no question"))
                .unwrap();
            thread::sleep(Duration::from_millis(5));
        }
        second.join().unwrap()
    }

    #[test]
    fn a_second_launch_hands_its_request_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let first = primary(launch(dir.path(), &Request::default()).unwrap());
        let sent = request(&["a", "b"]);
        let mut received = Vec::new();
        let second = second_launch(&first, dir.path(), &sent, |request| {
            received.push(request);
            Ok(())
        });
        assert!(
            matches!(second, Ok(Launch::Forwarded)),
            "the second launch exits as forwarded"
        );
        assert_eq!(received, [sent]);
    }

    #[test]
    fn a_launch_fails_with_the_reason_when_the_window_could_not_open_its_request() {
        let dir = tempfile::tempdir().unwrap();
        let first = primary(launch(dir.path(), &Request::default()).unwrap());
        let second = second_launch(&first, dir.path(), &request(&["a"]), |_| {
            bail!("database is locked")
        });
        let error = second.err().expect("the launch fails").to_string();
        assert!(error.contains("database is locked"), "{error}");
    }

    #[test]
    fn a_symlink_to_the_data_directory_reaches_the_same_instance() {
        let dir = tempfile::tempdir().unwrap();
        let alias = tempfile::tempdir().unwrap().keep().join("alias");
        std::os::unix::fs::symlink(dir.path(), &alias).unwrap();
        let first = primary(launch(dir.path(), &Request::default()).unwrap());
        let second = second_launch(&first, &alias, &request(&["a"]), |_| Ok(()));
        assert!(matches!(second, Ok(Launch::Forwarded)));
        std::fs::remove_dir_all(alias.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_data_directory_path_longer_than_a_socket_address_still_works() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("d".repeat(200));
        std::fs::create_dir(&dir).unwrap();
        let first = primary(launch(&dir, &Request::default()).unwrap());
        let second = second_launch(&first, &dir, &request(&["a"]), |_| Ok(()));
        assert!(matches!(second, Ok(Launch::Forwarded)));
    }

    #[test]
    fn separate_data_directories_are_separate_instances() {
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _first = primary(launch(one.path(), &Request::default()).unwrap());
        primary(launch(two.path(), &request(&["a"])).unwrap());
    }

    #[test]
    fn leftovers_of_a_crashed_instance_do_not_block_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        drop(primary(launch(dir.path(), &Request::default()).unwrap()));
        assert!(dir.path().join("instance.sock").exists());
        assert!(dir.path().join("instance.lock").exists());
        let next = primary(launch(dir.path(), &request(&["a"])).unwrap());
        next.answer(
            |_| panic!("nothing was forwarded"),
            |_, _| panic!("nothing was asked"),
        )
        .unwrap();
    }

    /// Asks `question` on `path` from another thread while `first` answers with `respond`.
    fn ask_from_another_process(
        first: &Primary,
        path: &Path,
        question: Question,
        mut respond: impl FnMut(Question, Reply),
    ) -> Result<Value> {
        let path = path.to_owned();
        let asker = thread::spawn(move || ask(&path, question));
        while !asker.is_finished() {
            first
                .answer(|_| panic!("nothing was opened"), &mut respond)
                .unwrap();
            thread::sleep(Duration::from_millis(5));
        }
        asker.join().unwrap()
    }

    #[test]
    fn a_question_reaches_the_running_canopy_and_its_answer_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let first = primary(launch(dir.path(), &Request::default()).unwrap());
        let mut asked = Vec::new();
        let answer = ask_from_another_process(
            &first,
            dir.path(),
            Question::Session { text: true },
            |question, reply| {
                asked.push(question);
                reply.send(Ok(json!({ "tabs": [] })));
            },
        )
        .unwrap();
        assert_eq!(asked, [Question::Session { text: true }]);
        assert_eq!(answer, json!({ "tabs": [] }));
        let error = ask_from_another_process(
            &first,
            dir.path(),
            Question::Session { text: false },
            |_, reply| reply.send(Err(anyhow!("no page is shown"))),
        )
        .unwrap_err();
        assert!(error.to_string().contains("no page is shown"), "{error}");
        let error = ask_from_another_process(
            &first,
            dir.path(),
            Question::Session { text: false },
            |_, reply| drop(reply),
        )
        .unwrap_err();
        assert!(error.to_string().contains("did not answer"), "{error}");
    }

    #[test]
    fn asking_never_starts_a_canopy() {
        let dir = tempfile::tempdir().unwrap();
        let error = ask(dir.path(), Question::Session { text: false }).unwrap_err();
        assert!(error.to_string().contains("not running"), "{error}");
        assert!(!dir.path().join("instance.lock").exists());
        drop(primary(launch(dir.path(), &Request::default()).unwrap()));
        let error = ask(dir.path(), Question::Session { text: false }).unwrap_err();
        assert!(error.to_string().contains("not running"), "{error}");
        let missing = dir.path().join("missing");
        assert!(ask(&missing, Question::Session { text: false }).is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn a_request_from_the_socket_is_checked_like_any_input() {
        let round_trip = request(&["a"]);
        assert_eq!(Request::decode(&round_trip.encode()).unwrap(), round_trip);
        for bytes in [
            &br#"{"urls":["file:///etc/passwd"]}"#[..],
            br#"{"urls":"https://example.org"}"#,
            b"not json",
        ] {
            assert!(Request::decode(bytes).is_err());
        }
        let question = Question::Session { text: true };
        let value: Value = serde_json::from_slice(&question.encode()).unwrap();
        assert_eq!(Question::decode(&value).unwrap().unwrap(), question);
        assert!(Question::decode(&json!({ "query": "cookies" }))
            .unwrap()
            .is_err());
        assert!(Question::decode(&json!({ "urls": [] })).is_none());
    }
}
