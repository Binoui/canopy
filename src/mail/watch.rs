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
use super::{
    marks::{Key, Marks},
    MailAsk, Unmarked, CONFIRM, LINGER, SETTLE, UNCONFIRMED,
};
use crate::pages::{ErrandEnd, ErrandTicket, Fetched, Pages};

pub struct MailWatch {
    me: Weak<Self>,
    pages: Rc<Pages>,
    origin: Option<Origin>,
    mail: RefCell<Mail>,
    marks: RefCell<Marks<ErrandTicket>>,
    linger: Cell<Duration>,
    settle: Cell<Duration>,
    confirm: Cell<Duration>,
    unconfirmed: Cell<Duration>,
    phase: RefCell<Phase>,
    stopped: Cell<bool>,
    failures: Cell<u32>,
    ended: Cell<Option<Instant>>,
    page: RefCell<Option<(Url, String)>>,
    gap: Cell<Duration>,
    /// A trigger that arrived while a check ran. It applies once that check ends.
    pending: Cell<Option<Duration>>,
    changed: Box<dyn Fn(Vec<String>)>,
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
        began: Instant,
    },
}

impl MailWatch {
    pub fn start(
        pages: Rc<Pages>,
        origin: Option<Origin>,
        changed: impl Fn(Vec<String>) + 'static,
    ) -> Rc<Self> {
        let watch = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            pages,
            origin,
            mail: RefCell::default(),
            marks: RefCell::default(),
            linger: Cell::new(LINGER),
            settle: Cell::new(SETTLE),
            confirm: Cell::new(CONFIRM),
            unconfirmed: Cell::new(UNCONFIRMED),
            phase: RefCell::new(Phase::Idle),
            stopped: Cell::new(false),
            failures: Cell::new(0),
            ended: Cell::new(None),
            page: RefCell::default(),
            gap: Cell::new(GAP),
            pending: Cell::new(None),
            changed: Box::new(changed),
        });
        if watch.origin.is_some() {
            watch.wait(FIRST);
        }
        watch
    }

    pub fn mail(&self) -> Mail {
        self.marks.borrow().shown(&self.mail.borrow())
    }

    pub fn ask(&self, ask: MailAsk) {
        match ask {
            MailAsk::Check => self.check_soon(),
            MailAsk::MarkRead { address, id } => self.mark_read(&address, &id),
        }
    }

    fn mark_read(&self, address: &str, id: &str) {
        if self.stopped.get() {
            return;
        }
        let changed =
            self.marks
                .borrow_mut()
                .mark(&self.mail.borrow(), address, id, |key, link| {
                    let weak = self.me.clone();
                    self.pages.run_errand(link, self.linger.get(), move |end| {
                        if let Some(watch) = weak.upgrade() {
                            watch.page_left(key, end);
                        }
                    })
                });
        if changed {
            (self.changed)(vec![]);
        }
    }

    fn page_left(&self, key: Key, end: ErrandEnd) {
        if self.stopped.get() {
            return;
        }
        let result = match end {
            ErrandEnd::Done => Ok(()),
            ErrandEnd::Strayed => Err(Unmarked::SignIn),
            ErrandEnd::Failed => Err(Unmarked::NoAnswer),
        };
        let changed = self
            .marks
            .borrow_mut()
            .page_left(&key, result, Instant::now());
        match result {
            Ok(()) => self.pull_forward(self.confirm.get()),
            Err(Unmarked::SignIn) => self.pull_forward(Duration::ZERO),
            Err(_) => (),
        }
        if changed {
            (self.changed)(vec![]);
        }
    }

    #[cfg(feature = "gui-smoke")]
    pub fn set_mark_pace(
        &self,
        linger: Duration,
        settle: Duration,
        confirm: Duration,
        unconfirmed: Duration,
    ) {
        self.linger.set(linger);
        self.settle.set(settle);
        self.confirm.set(confirm);
        self.unconfirmed.set(unconfirmed);
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
        self.marks.replace(Marks::default());
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
            Phase::Checking { .. } => {
                let wanted = self.pending.get().map_or(wanted, |p| p.min(wanted));
                self.pending.set(Some(wanted));
                return;
            }
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
            began: Instant::now(),
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
        let began = match &*self.phase.borrow() {
            Phase::Checking { began, .. } => *began,
            _ => return,
        };
        let good = matches!(check, Check::Read(_));
        let failures = if matches!(check, Check::Failed(_)) {
            self.failures.get().saturating_add(1)
        } else {
            0
        };
        self.failures.set(failures);
        self.ended.set(Some(Instant::now()));
        let mut delay = delay_after(&check, failures);
        if let Some(wanted) = self.pending.take() {
            delay = delay.min(trigger_delay(wanted, Some(Duration::ZERO), self.gap.get()));
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let (mail, signed_out) = self.mail.borrow().after(check, now_ms);
        *self.mail.borrow_mut() = mail;
        let now = Instant::now();
        if good {
            if let Some(wanted) =
                self.marks
                    .borrow_mut()
                    .checked(&self.mail.borrow(), began, self.settle.get(), now)
            {
                delay = delay.min(trigger_delay(wanted, Some(Duration::ZERO), self.gap.get()));
            }
        }
        self.marks.borrow_mut().expire(now, self.unconfirmed.get());
        self.wait(delay);
        (self.changed)(signed_out);
    }
}

impl Drop for MailWatch {
    fn drop(&mut self) {
        self.stop();
    }
}
