//! Memory-only unread Gmail, read with the login the pages already hold.

mod feed;
#[cfg(feature = "desktop")]
mod watch;
#[cfg(feature = "desktop")]
pub use watch::MailWatch;

use std::time::Duration;
use url::Url;

const SIGN_IN_HOST: &str = "accounts.google.com";
const MAX_ACCOUNTS: u64 = 10;
const FIRST: Duration = Duration::from_secs(3);
const EVERY: Duration = Duration::from_secs(180);
const RETRY: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(900);
const GAP: Duration = Duration::from_secs(20);
const PAGE_DELAY: Duration = Duration::from_secs(2);

/// Gmail's bounded account slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AccountNumber(u8);

impl AccountNumber {
    pub fn new(n: u64) -> Option<Self> {
        (n < MAX_ACCOUNTS).then_some(Self(n as u8))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

/// Gmail's opaque conversation id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageId(String);

impl MessageId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Gmail's web origin, or a local fixture's origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Origin(Url);

impl Origin {
    pub fn gmail() -> Self {
        Self(Url::parse("https://mail.google.com").unwrap())
    }

    pub fn configured() -> Option<Self> {
        match std::env::var("CANOPY_GMAIL_ORIGIN") {
            Ok(value) => Self::parse(&value).or_else(|| {
                eprintln!("Invalid CANOPY_GMAIL_ORIGIN; mail checks disabled.");
                None
            }),
            Err(_) if cfg!(feature = "gui-smoke") => None,
            Err(_) => Some(Self::gmail()),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        let url = Url::parse(text).ok()?;
        (matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none())
        .then_some(Self(url))
    }

    fn feed(&self, number: AccountNumber) -> Url {
        self.0
            .join(&format!("/mail/u/{}/feed/atom", number.get()))
            .unwrap()
    }

    /// Gmail resolves an address in place of the account number to that account's current
    /// number, so the link survives renumbering (measured 2026-11-17).
    fn inbox(&self, address: &str) -> Url {
        self.0.join(&format!("/mail/u/{address}")).unwrap()
    }

    fn serves(&self, url: &Url) -> bool {
        self.0.origin() == url.origin()
    }

    fn concerns(&self, page: &Url) -> bool {
        self.serves(page) || page.host_str() == Some(SIGN_IN_HOST)
    }
}

/// The last complete check. Only the watch mutates it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mail {
    pub accounts: Vec<Account>,
    pub read_at: i64,
    pub trouble: Option<Trouble>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub number: AccountNumber,
    pub address: String,
    pub unread: u32,
    pub messages: Vec<Message>,
    pub inbox: Url,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub from: String,
    pub subject: String,
    pub snippet: String,
    pub issued: Option<i64>,
    pub link: Url,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trouble {
    Unreachable,
    Unreadable,
}

impl Mail {
    pub fn unread(&self) -> u32 {
        self.accounts
            .iter()
            .fold(0u32, |n, a| n.saturating_add(a.unread))
    }

    pub fn link_for(&self, address: &str, id: &str) -> Option<&Url> {
        let account = self.accounts.iter().find(|a| a.address == address)?;
        Some(
            account
                .messages
                .iter()
                .find(|m| m.id.as_str() == id)
                .map_or(&account.inbox, |m| &m.link),
        )
    }

    pub fn inbox(&self, address: &str) -> Option<&Url> {
        self.accounts
            .iter()
            .find(|a| a.address == address)
            .map(|a| &a.inbox)
    }

    fn after(&self, check: Check, now_ms: i64) -> (Mail, Vec<String>) {
        match check {
            Check::Read(accounts) => {
                let gone = self
                    .accounts
                    .iter()
                    .filter(|old| !accounts.iter().any(|a| a.address == old.address))
                    .map(|a| a.address.clone())
                    .collect();
                (
                    Self {
                        accounts,
                        read_at: now_ms,
                        trouble: None,
                    },
                    gone,
                )
            }
            Check::Failed(trouble) => (
                Self {
                    trouble: Some(trouble),
                    ..self.clone()
                },
                vec![],
            ),
        }
    }
}

#[derive(Debug)]
enum Check {
    Read(Vec<Account>),
    Failed(Trouble),
}

struct Walk {
    read: Vec<Account>,
    number: AccountNumber,
    known_max: Option<AccountNumber>,
}

#[derive(Debug)]
enum Step {
    Fetch(AccountNumber),
    Done(Check),
}

impl Walk {
    fn start(known_max: Option<AccountNumber>) -> (Self, AccountNumber) {
        let number = AccountNumber(0);
        (
            Self {
                read: vec![],
                number,
                known_max,
            },
            number,
        )
    }

    fn take(&mut self, outcome: feed::Outcome) -> Step {
        match outcome {
            feed::Outcome::Inbox(account) => self.read.push(account),
            feed::Outcome::SignedOut if Some(self.number) > self.known_max => {
                return Step::Done(Check::Read(std::mem::take(&mut self.read)))
            }
            feed::Outcome::SignedOut => (),
            feed::Outcome::Trouble(t) => return Step::Done(Check::Failed(t)),
        }
        match AccountNumber::new(u64::from(self.number.get()) + 1) {
            Some(next) => {
                self.number = next;
                Step::Fetch(next)
            }
            None => Step::Done(Check::Read(std::mem::take(&mut self.read))),
        }
    }
}

fn delay_after(check: &Check, failures: u32) -> Duration {
    match check {
        Check::Read(_) => EVERY,
        Check::Failed(_) => RETRY
            .saturating_mul(1u32 << failures.saturating_sub(1).min(5))
            .min(RETRY_MAX),
    }
}

fn trigger_delay(wanted: Duration, since_last: Option<Duration>, gap: Duration) -> Duration {
    wanted.max(since_last.map_or(Duration::ZERO, |since| gap.saturating_sub(since)))
}

#[cfg(test)]
pub(crate) fn fixture() -> Mail {
    let body = br#"<feed><title>Inbox for ana@example.test</title><fullcount>3</fullcount><entry><id>tag:111</id><title>A subject</title><summary>A snippet</summary><author><name>A sender</name></author><issued>2026-11-17T09:12:33Z</issued></entry></feed>"#;
    let feed::Outcome::Inbox(account) = feed::read(
        Some(&feed::Answer {
            status: 200,
            location: None,
            body,
        }),
        &Origin::gmail(),
        AccountNumber(0),
    ) else {
        panic!("fixture must parse")
    };
    Mail {
        accounts: vec![account],
        read_at: 1_794_906_753_000,
        trouble: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use feed::Outcome;

    fn account(n: u8) -> Account {
        let address = format!("a{n}@example.test");
        Account {
            number: AccountNumber(n),
            inbox: Origin::gmail().inbox(&address),
            address,
            unread: 3,
            messages: vec![],
        }
    }

    fn read_accounts(step: Step) -> Vec<Account> {
        let Step::Done(Check::Read(accounts)) = step else {
            panic!("expected a complete good check, got {step:?}")
        };
        accounts
    }

    #[test]
    fn walks_accounts_and_stops_at_the_first_unknown_signed_out_slot() {
        let (mut walk, first) = Walk::start(None);
        assert_eq!(first, AccountNumber(0));
        assert!(matches!(
            walk.take(Outcome::Inbox(account(0))),
            Step::Fetch(AccountNumber(1))
        ));
        assert!(matches!(
            walk.take(Outcome::Inbox(account(1))),
            Step::Fetch(AccountNumber(2))
        ));
        assert_eq!(read_accounts(walk.take(Outcome::SignedOut)).len(), 2);
        assert!(read_accounts(Walk::start(None).0.take(Outcome::SignedOut)).is_empty());
    }

    #[test]
    fn walks_past_known_holes_and_discovers_new_accounts() {
        let (mut walk, _) = Walk::start(Some(AccountNumber(2)));
        walk.take(Outcome::Inbox(account(0)));
        assert!(matches!(
            walk.take(Outcome::SignedOut),
            Step::Fetch(AccountNumber(2))
        ));
        assert!(matches!(
            walk.take(Outcome::Inbox(account(2))),
            Step::Fetch(AccountNumber(3))
        ));
        assert!(matches!(
            walk.take(Outcome::Inbox(account(3))),
            Step::Fetch(AccountNumber(4))
        ));
        assert_eq!(
            read_accounts(walk.take(Outcome::SignedOut))
                .iter()
                .map(|a| a.number.get())
                .collect::<Vec<_>>(),
            [0, 2, 3]
        );
    }

    #[test]
    fn failure_discards_a_partial_walk_and_ten_accounts_bound_it() {
        let (mut walk, _) = Walk::start(None);
        walk.take(Outcome::Inbox(account(0)));
        assert!(matches!(
            walk.take(Outcome::Trouble(Trouble::Unreachable)),
            Step::Done(Check::Failed(Trouble::Unreachable))
        ));
        let (mut walk, _) = Walk::start(None);
        for n in 0..9 {
            assert!(matches!(
                walk.take(Outcome::Inbox(account(n))),
                Step::Fetch(_)
            ));
        }
        assert_eq!(
            read_accounts(walk.take(Outcome::Inbox(account(9)))).len(),
            10
        );
        assert!(AccountNumber::new(10).is_none());
        assert!(AccountNumber::new(256).is_none());
    }

    #[test]
    fn failures_keep_the_last_complete_list_and_success_names_missing_addresses() {
        let mail = Mail {
            accounts: vec![account(0), account(1)],
            read_at: 100,
            trouble: None,
        };
        let (failed, gone) = mail.after(Check::Failed(Trouble::Unreadable), 200);
        assert_eq!(failed.accounts, mail.accounts);
        assert_eq!(failed.read_at, 100);
        assert_eq!(failed.trouble, Some(Trouble::Unreadable));
        assert!(gone.is_empty());
        let mut renumbered = account(1);
        renumbered.number = AccountNumber(0);
        let (good, gone) = failed.after(Check::Read(vec![renumbered.clone()]), 300);
        assert_eq!(gone, ["a0@example.test"]);
        assert_eq!(good.trouble, None);
        let (again, gone) = good.after(Check::Read(vec![renumbered]), 300);
        assert_eq!(again, good);
        assert!(gone.is_empty());
        assert_eq!(
            good.link_for("a1@example.test", "gone"),
            good.inbox("a1@example.test")
        );
        assert!(
            good.link_for("a0@example.test", "gone").is_none(),
            "a reused slot never resolves a departed address"
        );
    }

    #[test]
    fn cadence_backs_off_and_triggers_respect_the_gap() {
        for (n, seconds) in [
            (1, 30),
            (2, 60),
            (3, 120),
            (4, 240),
            (5, 480),
            (6, 900),
            (u32::MAX, 900),
        ] {
            assert_eq!(
                delay_after(&Check::Failed(Trouble::Unreachable), n),
                Duration::from_secs(seconds)
            );
        }
        assert_eq!(delay_after(&Check::Read(vec![]), 9), EVERY);
        assert_eq!(trigger_delay(Duration::ZERO, None, GAP), Duration::ZERO);
        assert_eq!(
            trigger_delay(PAGE_DELAY, Some(Duration::from_secs(5)), GAP),
            Duration::from_secs(15)
        );
        assert_eq!(trigger_delay(PAGE_DELAY, Some(GAP), GAP), PAGE_DELAY);
        assert_eq!(FIRST, Duration::from_secs(3));
    }

    #[test]
    fn accepts_only_bare_web_origins() {
        for text in ["http://localhost:42", "https://mail.google.com/"] {
            assert!(Origin::parse(text).is_some());
        }
        for text in [
            "file:///tmp/x",
            "https://host/path",
            "https://host/?x",
            "https://host/#x",
            "https://user:pass@host/",
        ] {
            assert!(Origin::parse(text).is_none(), "{text}");
        }
        let origin = Origin::gmail();
        assert!(origin.concerns(&Url::parse("https://accounts.google.com/login").unwrap()));
        assert_eq!(origin.feed(AccountNumber(2)).path(), "/mail/u/2/feed/atom");
    }
}
