//! Decides when a login the user sent to a page is worth offering to save.
//!
//! A sent login is only a candidate: the password may be wrong, or the click that sent it
//! may have been on another button. It is offered once the page stops asking for a
//! password, and dropped when a new document asks again or nothing happens for a while.

use std::time::{Duration, Instant};

use crate::logins::Login;

/// How long a sent login waits for the page to stop asking for a password.
const PATIENCE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct SaveWatch {
    candidate: Option<Candidate>,
}

struct Candidate {
    login: Login,
    sent: Instant,
    new_document: bool,
}

impl SaveWatch {
    pub(super) fn sent(&mut self, login: Login, now: Instant) {
        self.candidate = Some(Candidate {
            login,
            sent: now,
            new_document: false,
        });
    }

    pub(super) fn new_document(&mut self) {
        if let Some(candidate) = &mut self.candidate {
            candidate.new_document = true;
        }
    }

    /// The page reported whether it shows a password field. Returns the login to offer.
    pub(super) fn fields(&mut self, asks_password: bool, now: Instant) -> Option<Login> {
        let candidate = self.candidate.take()?;
        if now.duration_since(candidate.sent) > PATIENCE {
            return None;
        }
        if !asks_password {
            return Some(candidate.login);
        }
        // The same document still showing its form says nothing yet. A new document that
        // asks for a password is the sign-in page again: the login was refused.
        if !candidate.new_document {
            self.candidate = Some(candidate);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{address::WebOrigin, secret::Secret};

    fn login() -> Login {
        Login {
            origin: WebOrigin::of(&url::Url::parse("https://example.test").unwrap()).unwrap(),
            username: "alice".into(),
            password: Secret::new("fake".into()),
            last_used: 1,
        }
    }

    #[test]
    fn a_sent_login_is_offered_once_the_page_stops_asking() {
        let start = Instant::now();
        let mut watch = SaveWatch::default();
        assert!(watch.fields(false, start).is_none(), "nothing was sent");

        // The form goes away without a navigation.
        watch.sent(login(), start);
        assert!(watch.fields(true, start).is_none());
        assert!(watch.fields(false, start).is_some());
        assert!(watch.fields(false, start).is_none(), "offered once");

        // The form's page is replaced by one that asks for nothing.
        watch.sent(login(), start);
        watch.new_document();
        assert!(watch.fields(false, start).is_some());
    }

    #[test]
    fn a_refused_or_stale_login_is_dropped() {
        let start = Instant::now();
        let mut watch = SaveWatch::default();

        // The sign-in page comes back.
        watch.sent(login(), start);
        watch.new_document();
        assert!(watch.fields(true, start).is_none());
        assert!(watch.fields(false, start).is_none());

        // Nothing happened for too long.
        watch.sent(login(), start);
        assert!(watch
            .fields(false, start + PATIENCE + Duration::from_secs(1))
            .is_none());
        watch.sent(login(), start);
        assert!(watch.fields(false, start + PATIENCE).is_some());
    }
}
