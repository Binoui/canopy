use std::time::{Duration, Instant};
use url::Url;

use super::{feed, Account, Mail, MessageId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Key {
    pub address: String,
    pub id: MessageId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unmarked {
    NoAnswer,
    SignIn,
    StillUnread,
    Unconfirmed,
}

pub(super) enum State<T> {
    Sending(T),
    Sent { left: Instant },
    Failed(Unmarked),
}

struct Entry<T> {
    key: Key,
    issued: Option<i64>,
    state: State<T>,
}

pub(super) struct Marks<T> {
    entries: Vec<Entry<T>>,
}

impl<T> Default for Marks<T> {
    fn default() -> Self {
        Self { entries: vec![] }
    }
}

impl<T> Marks<T> {
    pub fn mark(
        &mut self,
        checked: &Mail,
        address: &str,
        id: &str,
        send: impl FnOnce(Key, &Url) -> T,
    ) -> bool {
        let Some(account) = checked.accounts.iter().find(|a| a.address == address) else {
            return false;
        };
        let Some(message) = account.messages.iter().find(|m| m.id.as_str() == id) else {
            return false;
        };
        if message.link == account.inbox {
            return false;
        }
        let index = self
            .entries
            .iter()
            .position(|e| e.key.address == address && e.key.id.as_str() == id);
        if index.is_some_and(|i| {
            matches!(
                self.entries[i].state,
                State::Sending(_) | State::Sent { .. }
            )
        }) {
            return false;
        }
        let key = Key {
            address: address.to_owned(),
            id: message.id.clone(),
        };
        let ticket = send(key.clone(), &message.link);
        let entry = Entry {
            key,
            issued: message.issued,
            state: State::Sending(ticket),
        };
        if let Some(i) = index {
            self.entries[i] = entry;
        } else {
            self.entries.push(entry);
        }
        true
    }

    pub fn page_left(&mut self, key: &Key, result: Result<(), Unmarked>, now: Instant) -> bool {
        let Some(Entry { state, .. }) = self.entries.iter_mut().find(|e| &e.key == key) else {
            return false;
        };
        if !matches!(state, State::Sending(_)) {
            return false;
        }
        *state = match result {
            Ok(()) => State::Sent { left: now },
            Err(why) => State::Failed(why),
        };
        result.is_err()
    }

    pub fn checked(
        &mut self,
        check: &Mail,
        began: Instant,
        settle: Duration,
        now: Instant,
    ) -> Option<Duration> {
        self.entries.retain_mut(|entry| {
            let Some(account) = check
                .accounts
                .iter()
                .find(|a| a.address == entry.key.address)
            else {
                return false;
            };
            if !lists(account, &entry.key) {
                return may_have_slid_out(account, entry.issued);
            }
            if let State::Sent { left } = entry.state {
                if began >= left + settle {
                    entry.state = State::Failed(Unmarked::StillUnread);
                }
            }
            true
        });
        self.entries
            .iter()
            .filter_map(|entry| match entry.state {
                State::Sent { left }
                    if check
                        .accounts
                        .iter()
                        .any(|a| a.address == entry.key.address && lists(a, &entry.key)) =>
                {
                    Some((left + settle).saturating_duration_since(now))
                }
                _ => None,
            })
            .min()
    }

    pub fn expire(&mut self, now: Instant, unconfirmed: Duration) {
        for entry in &mut self.entries {
            if matches!(entry.state, State::Sent { left } if now.saturating_duration_since(left) >= unconfirmed)
            {
                entry.state = State::Failed(Unmarked::Unconfirmed);
            }
        }
    }

    pub fn shown(&self, checked: &Mail) -> Mail {
        let mut mail = checked.clone();
        for account in &mut mail.accounts {
            let before = account.messages.len();
            account.messages.retain_mut(|message| {
                match self
                    .entries
                    .iter()
                    .find(|e| e.key.address == account.address && e.key.id == message.id)
                    .map(|e| &e.state)
                {
                    Some(State::Sending(_) | State::Sent { .. }) => false,
                    state => {
                        message.unmarked = match state {
                            Some(State::Failed(why)) => Some(*why),
                            _ => None,
                        };
                        true
                    }
                }
            });
            account.unread = account
                .unread
                .saturating_sub((before - account.messages.len()) as u32);
        }
        mail
    }
}

fn lists(account: &Account, key: &Key) -> bool {
    account.messages.iter().any(|m| m.id == key.id)
}

/// The feed lists only a page of the newest unread mails. A mail missing from a full page that
/// holds fewer mails than the unread count may have been pushed out by newer mail while still
/// unread, unless it is newer than the oldest mail listed.
fn may_have_slid_out(account: &Account, issued: Option<i64>) -> bool {
    let listed = account.messages.len();
    let cut_off = listed >= feed::PAGE && account.unread as usize > listed;
    let oldest = account.messages.iter().map(|m| m.issued).min();
    cut_off && !matches!((issued, oldest), (Some(t), Some(Some(o))) if t > o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct Ticket(Rc<Cell<usize>>);
    impl Drop for Ticket {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn fixture() -> Mail {
        let mut mail = super::super::fixture();
        let row = &mut mail.accounts[0].messages[0];
        row.link = Url::parse("https://mail.google.com/mail/u/0?message_id=111").unwrap();
        let mut second = row.clone();
        second.id = MessageId("222".into());
        mail.accounts[0].messages.push(second);
        mail
    }

    fn mark(book: &mut Marks<Ticket>, mail: &Mail, dropped: &Rc<Cell<usize>>) -> Key {
        let mut key = None;
        assert!(book.mark(mail, "ana@example.test", "111", |k, _| {
            key = Some(k);
            Ticket(dropped.clone())
        }));
        key.unwrap()
    }

    #[test]
    fn derives_counts_once_and_refuses_stale_duplicate_and_inbox_targets() {
        let mail = fixture();
        let mut book = Marks::default();
        assert_eq!(book.shown(&mail), mail);
        let dropped = Rc::default();
        mark(&mut book, &mail, &dropped);
        let shown = book.shown(&mail);
        assert_eq!(shown.unread(), 2);
        assert_eq!(shown.accounts[0].unread, 2);
        assert_eq!(shown.accounts[0].messages.len(), 1);
        assert_eq!(
            shown.accounts[0].unread as usize - shown.accounts[0].messages.len(),
            1
        );
        for (address, id) in [
            ("gone", "111"),
            ("ana@example.test", "gone"),
            ("ana@example.test", "111"),
        ] {
            assert!(!book.mark(&mail, address, id, |_, _| panic!("must not send")));
        }
        let mut inbox = mail.clone();
        inbox.accounts[0].messages[1].link = inbox.accounts[0].inbox.clone();
        assert!(!book.mark(&inbox, "ana@example.test", "222", |_, _| panic!("inbox")));
    }

    #[test]
    fn settles_by_check_start_and_allows_retry_after_failures() {
        let mail = fixture();
        let dropped = Rc::default();
        let mut book = Marks::default();
        let key = mark(&mut book, &mail, &dropped);
        let now = Instant::now();
        assert!(!book.page_left(&key, Ok(()), now));
        assert_eq!(dropped.get(), 1);
        assert_eq!(
            book.checked(&mail, now, Duration::from_secs(15), now),
            Some(Duration::from_secs(15))
        );
        assert_eq!(book.shown(&mail).unread(), 2);
        assert_eq!(
            book.checked(
                &mail,
                now + Duration::from_secs(15),
                Duration::from_secs(15),
                now + Duration::from_secs(16)
            ),
            None
        );
        assert_eq!(
            book.shown(&mail).accounts[0].messages[0].unmarked,
            Some(Unmarked::StillUnread)
        );
        for why in [Unmarked::SignIn, Unmarked::NoAnswer] {
            mark(&mut book, &mail, &dropped);
            assert!(book.page_left(&key, Err(why), now));
            assert!(!book.page_left(&key, Ok(()), now));
            assert_eq!(
                book.shown(&mail).accounts[0].messages[0].unmarked,
                Some(why)
            );
            assert_eq!(book.shown(&mail).unread(), 3);
        }
    }

    #[test]
    fn absence_settles_every_state_and_cancels_sending() {
        for state in 0..3 {
            for gone_account in [false, true] {
                let mail = fixture();
                let dropped = Rc::default();
                let mut book = Marks::default();
                let key = mark(&mut book, &mail, &dropped);
                let now = Instant::now();
                match state {
                    1 => {
                        book.page_left(&key, Ok(()), now);
                    }
                    2 => {
                        book.page_left(&key, Err(Unmarked::NoAnswer), now);
                    }
                    _ => (),
                }
                let mut check = mail.clone();
                if gone_account {
                    check.accounts.clear();
                } else {
                    check.accounts[0].messages.remove(0);
                }
                book.checked(&check, now, Duration::ZERO, now);
                assert_eq!(dropped.get(), 1);
                assert_eq!(book.shown(&mail), mail);
                assert!(!book.page_left(&key, Err(Unmarked::SignIn), now));
            }
        }
    }

    #[test]
    fn failed_checks_expire_optimism_and_good_checks_confirm_first() {
        let mail = fixture();
        let dropped = Rc::default();
        let mut book = Marks::default();
        let key = mark(&mut book, &mail, &dropped);
        let now = Instant::now();
        book.page_left(&key, Ok(()), now);
        book.expire(now + Duration::from_secs(59), Duration::from_secs(60));
        assert_eq!(book.shown(&mail).unread(), 2);
        book.expire(now + Duration::from_secs(60), Duration::from_secs(60));
        assert_eq!(
            book.shown(&mail).accounts[0].messages[0].unmarked,
            Some(Unmarked::Unconfirmed)
        );
        mark(&mut book, &mail, &dropped);
        book.page_left(&key, Ok(()), now);
        let mut check = mail.clone();
        check.accounts[0].messages.remove(0);
        book.checked(
            &check,
            now,
            Duration::from_secs(15),
            now + Duration::from_secs(60),
        );
        book.expire(now + Duration::from_secs(60), Duration::from_secs(60));
        assert_eq!(book.shown(&mail), mail);
    }

    /// A full page: 20 listed of 25 unread, newest first, with 111 the oldest listed.
    fn full_page() -> Mail {
        let mut mail = fixture();
        let account = &mut mail.accounts[0];
        let first = account.messages[0].clone();
        let at = first.issued.unwrap();
        account.messages = (0..feed::PAGE as i64)
            .map(|i| {
                let mut m = first.clone();
                m.id = MessageId(format!("{}", 1000 + i));
                m.issued = Some(at - i * 1_000);
                m
            })
            .collect();
        account.messages[feed::PAGE - 1].id = MessageId("111".into());
        account.unread = 25;
        mail
    }

    #[test]
    fn a_mail_pushed_past_a_full_page_stays_marked() {
        let mail = full_page();
        let dropped = Rc::default();
        let mut book = Marks::default();
        let key = mark(&mut book, &mail, &dropped);
        let now = Instant::now();
        let settle = Duration::from_secs(15);
        // Newer mail arrives: 111 drops off the end while still unread.
        let mut slid = mail.clone();
        let mut newer = slid.accounts[0].messages[0].clone();
        newer.id = MessageId("999".into());
        newer.issued = newer.issued.map(|t| t + 1_000);
        slid.accounts[0].messages.insert(0, newer);
        slid.accounts[0].messages.pop();
        slid.accounts[0].unread = 26;
        assert_eq!(book.checked(&slid, now, settle, now), None);
        assert_eq!(dropped.get(), 0, "the hidden page keeps working");
        book.page_left(&key, Ok(()), now);
        assert_eq!(dropped.get(), 1);
        assert_eq!(
            book.checked(&slid, now, settle, now),
            None,
            "no confirming check for a mail the feed cannot show"
        );
        assert_eq!(book.shown(&mail).accounts[0].messages.len(), feed::PAGE - 1);
        book.checked(&mail, now + settle, settle, now + settle);
        let shown = book.shown(&mail);
        assert_eq!(
            shown.accounts[0].messages[feed::PAGE - 1].unmarked,
            Some(Unmarked::StillUnread),
            "back in the feed still unread: the row says the mark failed"
        );
    }

    #[test]
    fn a_missing_mail_newer_than_the_oldest_listed_was_read() {
        let mail = full_page();
        let dropped = Rc::default();
        let mut book = Marks::default();
        mark(&mut book, &mail, &dropped);
        // Gmail read 111, so the next older unread mail joins the page.
        let mut read = mail.clone();
        let mut older = read.accounts[0].messages.pop().unwrap();
        older.id = MessageId("2000".into());
        older.issued = older.issued.map(|t| t - 1_000);
        read.accounts[0].messages.push(older);
        read.accounts[0].unread = 24;
        let now = Instant::now();
        book.checked(&read, now, Duration::from_secs(15), now);
        assert_eq!(dropped.get(), 1);
        assert_eq!(book.shown(&mail), mail);
    }
}
