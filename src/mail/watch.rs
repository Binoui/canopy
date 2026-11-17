//! Main-loop checks and the only writer of the mail list.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gio::prelude::CancellableExt;
use url::Url;

use super::{
    delay_after, feed, trigger_delay, AccountNumber, Check, Mail, Origin, Step, Walk, FIRST, GAP,
    PAGE_DELAY,
};
use crate::pages::{Fetched, Pages};

pub struct MailWatch {
    me: Weak<Self>,
    pages: Rc<Pages>,
    origin: Option<Origin>,
    mail: RefCell<Mail>,
    phase: RefCell<Phase>,
    stopped: Cell<bool>,
    failures: Cell<u32>,
    ended: Cell<Option<Instant>>,
    page: RefCell<Option<(Url, String)>>,
    gap: Cell<Duration>,
    checked: Box<dyn Fn(Vec<String>)>,
}

enum Phase {
    Idle,
    Waiting {
        timer: glib::SourceId,
        due: Instant,
    },
    Checking {
        walk: Walk,
        cancel: gio::Cancellable,
    },
}

impl MailWatch {
    pub fn start(
        pages: Rc<Pages>,
        origin: Option<Origin>,
        checked: impl Fn(Vec<String>) + 'static,
    ) -> Rc<Self> {
        let watch = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            pages,
            origin,
            mail: RefCell::default(),
            phase: RefCell::new(Phase::Idle),
            stopped: Cell::new(false),
            failures: Cell::new(0),
            ended: Cell::new(None),
            page: RefCell::default(),
            gap: Cell::new(GAP),
            checked: Box::new(checked),
        });
        if watch.origin.is_some() {
            watch.wait(FIRST);
        }
        watch
    }

    pub fn mail(&self) -> Mail {
        self.mail.borrow().clone()
    }

    pub fn check_soon(&self) {
        self.pull_forward(Duration::ZERO);
    }

    pub fn notice_page(&self, url: &Url, title: &str) {
        let Some(origin) = &self.origin else { return };
        if !origin.concerns(url) {
            self.page.borrow_mut().take();
            return;
        }
        let seen = (url.clone(), title.to_owned());
        if self.page.borrow().as_ref() == Some(&seen) {
            return;
        }
        *self.page.borrow_mut() = Some(seen);
        self.pull_forward(PAGE_DELAY);
    }

    /// Permanently stops timers and in-flight requests. Late answers are ignored.
    pub fn stop(&self) {
        self.stopped.set(true);
        match self.phase.replace(Phase::Idle) {
            Phase::Waiting { timer, .. } => timer.remove(),
            Phase::Checking { cancel, .. } => cancel.cancel(),
            Phase::Idle => (),
        }
    }

    #[cfg(feature = "gui-smoke")]
    pub fn set_gap(&self, gap: Duration) {
        self.gap.set(gap);
    }

    fn pull_forward(&self, wanted: Duration) {
        if self.stopped.get() || self.origin.is_none() {
            return;
        }
        let delay = trigger_delay(
            wanted,
            self.ended.get().map(|t| t.elapsed()),
            self.gap.get(),
        );
        match &*self.phase.borrow() {
            Phase::Checking { .. } => return,
            Phase::Waiting { due, .. } if *due <= Instant::now() + delay => return,
            _ => (),
        }
        if let Phase::Waiting { timer, .. } = self.phase.replace(Phase::Idle) {
            timer.remove();
        }
        self.wait(delay);
    }

    fn wait(&self, delay: Duration) {
        let weak = self.me.clone();
        let timer = glib::timeout_add_local_once(delay, move || {
            if let Some(watch) = weak.upgrade() {
                watch.begin();
            }
        });
        *self.phase.borrow_mut() = Phase::Waiting {
            timer,
            due: Instant::now() + delay,
        };
    }

    fn begin(&self) {
        if self.stopped.get() {
            return;
        }
        let known_max = self.mail.borrow().accounts.iter().map(|a| a.number).max();
        let (walk, first) = Walk::start(known_max);
        *self.phase.borrow_mut() = Phase::Checking {
            walk,
            cancel: gio::Cancellable::new(),
        };
        self.fetch(first);
    }

    fn fetch(&self, number: AccountNumber) {
        let Some(origin) = &self.origin else { return };
        let cancel = match &*self.phase.borrow() {
            Phase::Checking { cancel, .. } => cancel.clone(),
            _ => return,
        };
        let weak = self.me.clone();
        self.pages
            .fetch(&origin.feed(number), Some(&cancel), move |fetched| {
                if let Some(watch) = weak.upgrade() {
                    watch.answered(number, fetched);
                }
            });
    }

    fn answered(&self, number: AccountNumber, fetched: anyhow::Result<Fetched>) {
        if self.stopped.get() {
            return;
        }
        let Some(origin) = &self.origin else { return };
        let answer = fetched.as_ref().ok().map(|f| feed::Answer {
            status: f.status,
            location: f.location.as_deref(),
            body: &f.body,
        });
        let outcome = feed::read(answer.as_ref(), origin, number);
        let step = match &mut *self.phase.borrow_mut() {
            Phase::Checking { walk, .. } => walk.take(outcome),
            _ => return,
        };
        match step {
            Step::Fetch(next) => self.fetch(next),
            Step::Done(check) => self.finish(check),
        }
    }

    fn finish(&self, check: Check) {
        let failures = if matches!(check, Check::Failed(_)) {
            self.failures.get().saturating_add(1)
        } else {
            0
        };
        self.failures.set(failures);
        self.ended.set(Some(Instant::now()));
        let delay = delay_after(&check, failures);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let (mail, signed_out) = self.mail.borrow().after(check, now_ms);
        *self.mail.borrow_mut() = mail;
        self.wait(delay);
        (self.checked)(signed_out);
    }
}

impl Drop for MailWatch {
    fn drop(&mut self) {
        self.stop();
    }
}
