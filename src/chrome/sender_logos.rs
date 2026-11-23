use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::{Duration, SystemTime},
};

use gtk::gdk_pixbuf::{prelude::*, Pixbuf, PixbufLoader};

use crate::{
    mail::Origin,
    pages::{Fetched, Pages},
};

use super::favicon::normalized_png;

const LOGO_AGE: Duration = Duration::from_secs(30 * 86400);
const MISSING_AGE: Duration = Duration::from_secs(7 * 86400);
const RETRY: Duration = Duration::from_secs(10 * 60);
const IN_FLIGHT: usize = 4;
const CHANGE_WINDOW: Duration = Duration::from_millis(250);

type ChangeHandler = Box<dyn Fn(&[String])>;

#[derive(Clone)]
struct Logo {
    png: Rc<[u8]>,
    fetched_at: SystemTime,
}

enum State {
    Unknown,
    Fetching(Option<Logo>),
    Logo(Logo),
    Missing {
        until: SystemTime,
        old: Option<Logo>,
    },
    Failed {
        until: SystemTime,
        old: Option<Logo>,
    },
}

impl State {
    fn logo(&self) -> Option<&Logo> {
        match self {
            Self::Unknown => None,
            Self::Logo(logo) => Some(logo),
            Self::Fetching(old) | Self::Missing { old, .. } | Self::Failed { old, .. } => {
                old.as_ref()
            }
        }
    }

    fn lookup(&mut self, now: SystemTime) -> (Option<Rc<[u8]>>, bool) {
        let refresh = match self {
            Self::Unknown => true,
            Self::Fetching(_) => false,
            Self::Logo(logo) => now >= logo.fetched_at + LOGO_AGE,
            Self::Missing { until, .. } | Self::Failed { until, .. } => now >= *until,
        };
        let logo = self.logo();
        let png = logo.map(|logo| logo.png.clone());
        if refresh {
            *self = Self::Fetching(logo.cloned());
        }
        (png, refresh)
    }

    fn complete(old: Option<Logo>, answer: Answer, now: SystemTime) -> Self {
        match answer {
            Answer::Logo(png) => Self::Logo(Logo {
                png,
                fetched_at: now,
            }),
            Answer::NotFound | Answer::NoImage => Self::Missing {
                until: now + MISSING_AGE,
                old,
            },
            Answer::Failed => Self::Failed {
                until: now + RETRY,
                old,
            },
        }
    }
}

enum Answer {
    Logo(Rc<[u8]>),
    NotFound,
    NoImage,
    Failed,
}

impl Answer {
    fn from_fetch(answer: anyhow::Result<Fetched>) -> Self {
        match answer {
            Ok(answer) if answer.status == 200 => decode(&answer.body)
                .as_ref()
                .and_then(normalized_png)
                .map_or(Self::NoImage, Self::Logo),
            Ok(answer) if answer.status == 404 => Self::NotFound,
            _ => Self::Failed,
        }
    }

    fn fallback(&self, site: &str) -> Option<String> {
        matches!(self, Self::NotFound).then(|| format!("www.{site}"))
    }
}

fn valid_site(site: &str) -> bool {
    fn alphanumeric(byte: u8) -> bool {
        byte.is_ascii_lowercase() || byte.is_ascii_digit()
    }
    site.len() <= 253
        && site.split('.').all(|label| {
            let bytes = label.as_bytes();
            bytes.len() <= 63
                && bytes.first().copied().is_some_and(alphanumeric)
                && bytes.last().copied().is_some_and(alphanumeric)
                && bytes.iter().all(|&byte| byte == b'-' || alphanumeric(byte))
        })
}

fn decode(bytes: &[u8]) -> Option<Pixbuf> {
    let loader = PixbufLoader::new();
    loader.connect_size_prepared(|loader, width, height| {
        if width.max(height) > 1024 {
            loader.set_size(32, 32);
        }
    });
    let written = loader.write(bytes).is_ok();
    let closed = loader.close().is_ok();
    (written && closed).then(|| loader.pixbuf()).flatten()
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<String>,
    active: usize,
}

impl Queue {
    fn start_next(&mut self) -> Option<String> {
        if self.active == IN_FLIGHT {
            return None;
        }
        let site = self.waiting.pop_front()?;
        self.active += 1;
        Some(site)
    }
}

pub(super) struct SenderLogos {
    origin: Origin,
    pages: Weak<Pages>,
    disk: Disk,
    sites: RefCell<HashMap<String, State>>,
    queue: RefCell<Queue>,
    changes: RefCell<Vec<String>>,
    on_change: RefCell<Option<ChangeHandler>>,
}

impl SenderLogos {
    pub(super) fn new(origin: Origin, pages: &Rc<Pages>, data: &Path) -> Rc<Self> {
        Rc::new(Self {
            origin,
            pages: Rc::downgrade(pages),
            disk: Disk(data.join("sender-logos")),
            sites: RefCell::default(),
            queue: RefCell::default(),
            changes: RefCell::default(),
            on_change: RefCell::default(),
        })
    }

    pub(super) fn connect_changed(&self, callback: impl Fn(&[String]) + 'static) {
        self.on_change.replace(Some(Box::new(callback)));
    }

    pub(super) fn png(self: &Rc<Self>, site: &str) -> Option<Rc<[u8]>> {
        if !valid_site(site) {
            return None;
        }
        let (png, refresh) = self
            .sites
            .borrow_mut()
            .entry(site.to_owned())
            .or_insert_with(|| self.disk.load(site))
            .lookup(SystemTime::now());
        if refresh {
            self.queue.borrow_mut().waiting.push_back(site.to_owned());
            self.start_waiting();
        }
        png
    }

    fn start_waiting(self: &Rc<Self>) {
        let Some(pages) = self.pages.upgrade() else {
            return;
        };
        while let Some(site) = self.queue.borrow_mut().start_next() {
            let weak = Rc::downgrade(self);
            let pages = pages.clone();
            let origin = self.origin.clone();
            glib::MainContext::default().spawn_local(async move {
                let mut answer = fetch(&pages, &origin, &site).await;
                if let Some(host) = answer.fallback(&site) {
                    answer = fetch(&pages, &origin, &host).await;
                }
                if let Some(store) = weak.upgrade() {
                    store.complete(site, answer);
                }
            });
        }
    }

    fn complete(self: &Rc<Self>, site: String, answer: Answer) {
        let changed = matches!(&answer, Answer::Logo(_));
        let mut sites = self.sites.borrow_mut();
        let old = sites.get(&site).and_then(State::logo).cloned();
        let state = State::complete(old, answer, SystemTime::now());
        let _ = self.disk.save(&site, &state);
        sites.insert(site.clone(), state);
        drop(sites);
        if changed {
            self.notify(site);
        }
        self.queue.borrow_mut().active -= 1;
        self.start_waiting();
    }

    fn notify(self: &Rc<Self>, site: String) {
        let mut changes = self.changes.borrow_mut();
        changes.push(site);
        if changes.len() > 1 {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(CHANGE_WINDOW, move || {
            if let Some(store) = weak.upgrade() {
                let sites = store.changes.take();
                if let Some(callback) = store.on_change.borrow().as_ref() {
                    callback(&sites);
                }
            }
        });
    }
}

async fn fetch(pages: &Pages, origin: &Origin, host: &str) -> Answer {
    Answer::from_fetch(
        pages
            .fetch_without_cookies(&origin.sender_logo_url(host), 256 * 1024)
            .await,
    )
}

struct Disk(PathBuf);

impl Disk {
    fn load(&self, site: &str) -> State {
        let directory = self.0.join(site);
        let logo =
            read_entry(&directory.join("logo.png"), 32 * 1024).and_then(|(fetched_at, bytes)| {
                let pixbuf = decode(&bytes)?;
                (bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                    && (pixbuf.width(), pixbuf.height()) == (32, 32))
                    .then(|| Logo {
                        png: bytes.into(),
                        fetched_at,
                    })
            });
        let missing = read_entry(&directory.join("missing"), 0)
            .map(|(at, _)| at)
            .filter(|at| logo.as_ref().is_none_or(|logo| *at > logo.fetched_at));
        if let Some(at) = missing {
            State::Missing {
                until: at + MISSING_AGE,
                old: logo,
            }
        } else {
            logo.map_or(State::Unknown, State::Logo)
        }
    }

    fn save(&self, site: &str, state: &State) -> io::Result<()> {
        let (file, bytes): (_, &[u8]) = match state {
            State::Logo(logo) => ("logo.png", &logo.png),
            State::Missing { .. } => ("missing", &[]),
            _ => return Ok(()),
        };
        let directory = self.0.join(site);
        fs::create_dir_all(&directory)?;
        crate::atomic_file::write(&directory.join(file), bytes)?;
        if matches!(state, State::Logo(_)) {
            let _ = fs::remove_file(directory.join("missing"));
        }
        Ok(())
    }
}

fn read_entry(path: &Path, limit: u64) -> Option<(SystemTime, Vec<u8>)> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some((metadata.modified().ok()?, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: i32, height: i32) -> Vec<u8> {
        let pixbuf = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, width, height).unwrap();
        pixbuf.fill(0x2c_75_c4_ff);
        pixbuf.save_to_bufferv("png", &[]).unwrap()
    }

    fn answer(status: u16, body: Vec<u8>) -> Answer {
        Answer::from_fetch(Ok(Fetched {
            status,
            location: None,
            body,
        }))
    }

    #[test]
    fn fresh_logo_is_served_and_expired_logo_is_served_during_one_refresh() {
        let now = SystemTime::UNIX_EPOCH;
        let png = Rc::<[u8]>::from([1]);
        let mut state = State::complete(None, Answer::Logo(png.clone()), now);
        assert_eq!(
            state.lookup(now + LOGO_AGE - Duration::from_secs(1)),
            (Some(png.clone()), false)
        );
        assert_eq!(state.lookup(now + LOGO_AGE), (Some(png.clone()), true));
        assert_eq!(state.lookup(now + LOGO_AGE), (Some(png), false));
    }

    #[test]
    fn a_network_failure_keeps_the_previous_logo_and_retries_after_ten_minutes() {
        let now = SystemTime::UNIX_EPOCH;
        for old in [
            None,
            Some(Logo {
                png: Rc::from([1]),
                fetched_at: now,
            }),
        ] {
            let png = old.as_ref().map(|logo| logo.png.clone());
            let failure = Answer::from_fetch(Err(anyhow::anyhow!("network failure")));
            let mut state = State::complete(old, failure, now);
            assert_eq!(
                state.lookup(now + RETRY - Duration::from_secs(1)),
                (png.clone(), false)
            );
            assert_eq!(state.lookup(now + RETRY), (png, true));
        }
    }

    #[test]
    fn two_404s_leave_a_letter_for_seven_days() {
        let now = SystemTime::UNIX_EPOCH;
        let site = "example.test";
        let first = answer(404, png(16, 16));
        assert_eq!(first.fallback(site).as_deref(), Some("www.example.test"));
        let mut state = State::complete(None, answer(404, png(16, 16)), now);
        assert_eq!(
            state.lookup(now + MISSING_AGE - Duration::from_secs(1)),
            (None, false)
        );
        assert_eq!(state.lookup(now + MISSING_AGE), (None, true));
        assert_eq!(state.lookup(now + MISSING_AGE), (None, false));
    }

    #[test]
    fn a_small_image_becomes_a_32_px_png_and_an_invalid_body_is_missing() {
        let good = answer(200, png(16, 12));
        let Answer::Logo(bytes) = good else {
            panic!("expected a PNG");
        };
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let scaled = decode(&bytes).unwrap();
        assert_eq!((scaled.width(), scaled.height()), (32, 32));
        let invalid = answer(200, b"not an image".to_vec());
        assert!(matches!(invalid, Answer::NoImage));
        assert!(invalid.fallback("example.test").is_none());
    }

    #[test]
    fn only_four_sites_fetch_at_once_and_the_queue_keeps_order() {
        let mut queue = Queue {
            waiting: (0..6).map(|n| format!("site-{n}.test")).collect(),
            ..Queue::default()
        };
        for n in 0..4 {
            assert_eq!(queue.start_next(), Some(format!("site-{n}.test")));
        }
        assert_eq!(queue.start_next(), None);
        queue.active -= 1;
        assert_eq!(queue.start_next().as_deref(), Some("site-4.test"));
        assert_eq!(queue.start_next(), None);
    }

    #[test]
    fn only_plain_lower_case_domains_can_be_cache_keys() {
        let label = "a".repeat(63);
        let longest = format!("{label}.{label}.{label}.{}", "b".repeat(61));
        assert_eq!(longest.len(), 253);
        for site in ["shop-7.example", "a.co.uk", "xn--caf-dma.fr", &longest] {
            assert!(valid_site(site), "{site}");
        }
        for site in [
            "",
            ".",
            "..",
            "a..test",
            "-a.test",
            "a-.test",
            ".example.test",
            "example.test.",
            "Example.test",
            "café.test",
            "a_b.test",
            "a@b.test",
            "../a.test",
            "http://a.test",
            "a.test:80",
            "a.test?q=1",
            "a.test#x",
            "a.test\n",
            &format!("{longest}b"),
            &format!("{label}a.test"),
        ] {
            assert!(!valid_site(site), "{site:?}");
        }
    }

    #[test]
    fn disk_restores_logos_and_misses_and_ignores_invalid_files() -> anyhow::Result<()> {
        let data = tempfile::tempdir()?;
        let disk = Disk(data.path().join("sender-logos"));
        let now = SystemTime::now();
        let site = "example.test";
        let bytes = Rc::<[u8]>::from(png(32, 32));
        let state = State::complete(None, Answer::Logo(bytes.clone()), now);
        disk.save(site, &state)?;
        assert_eq!(disk.load(site).lookup(now), (Some(bytes.clone()), false));

        let state = State::complete(None, Answer::NotFound, now);
        disk.save("absent.test", &state)?;
        assert_eq!(disk.load("absent.test").lookup(now), (None, false));
        fs::write(disk.0.join("absent.test/missing"), b"bad marker")?;
        assert!(matches!(disk.load("absent.test"), State::Unknown));
        fs::write(disk.0.join(site).join("logo.png"), b"bad PNG")?;
        assert!(matches!(disk.load(site), State::Unknown));
        Ok(())
    }
}
