//! One Canopy per data directory. The process holding `instance.lock` owns the profile and
//! listens on `instance.sock`; any later launch on that directory hands its request over
//! and exits. The kernel drops the lock when its holder dies, so a crash never blocks a start.

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
    path::Path,
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
    let socket_path = data_dir.join("instance.sock");
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
                let listener = UnixListener::bind(&socket_path)
                    .with_context(|| format!("Could not listen at {}.", socket_path.display()))?;
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
    if reply != ACCEPTED {
        bail!("Canopy is open on this data directory but refused the request.");
    }
    Ok(())
}

impl Primary {
    /// Hands every waiting launch's request to `open`, then tells that launch it can exit.
    pub fn answer(&self, mut open: impl FnMut(Request)) -> Result<()> {
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
            let request = Request::decode(&bytes)
                .map_err(|error| anyhow!("Ignored a launch request. {error}"))?;
            open(request);
            stream.write_all(ACCEPTED)?;
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

    #[test]
    fn a_second_launch_hands_its_request_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let first = primary(launch(dir.path(), &Request::default()).unwrap());
        let sent = request(&["a", "b"]);
        let path = dir.path().to_owned();
        let second = {
            let sent = sent.clone();
            thread::spawn(move || matches!(launch(&path, &sent), Ok(Launch::Forwarded)))
        };
        let mut received = Vec::new();
        while !second.is_finished() {
            first.answer(|request| received.push(request)).unwrap();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            second.join().unwrap(),
            "the second launch exits as forwarded"
        );
        assert_eq!(received, [sent]);
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
        next.answer(|_| panic!("nothing was forwarded")).unwrap();
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
    }
}
