//! Signing in on the live page: noticing its sign-in fields, filling the login the user
//! picked, and hearing the login the user sends.

use std::rc::Rc;

use anyhow::{anyhow, Result};
use javascriptcore::ValueExt;
use serde_json::Value;
use url::Url;
use webkit2gtk::{
    UserContentInjectedFrames, UserContentManager, UserContentManagerExt, UserScript,
    UserScriptInjectionTime,
};
use zeroize::Zeroizing;

use super::{LivePage, NativeState, WORLD};
use crate::{
    address::WebOrigin,
    logins::Login,
    pages::{LoginEvent, PageEvent, Pages},
    secret::Secret,
};

const HELPER: &str = include_str!("../login_fields.js");
/// Script message the helper posts when sign-in fields appear or go away.
const CHANGED: &str = "canopyLoginFields";
/// Script message carrying the user name and password the user sends to the page.
const SENT: &str = "canopyLoginSent";

/// Adds the helper to the page's isolated world. Only the top frame gets it: a login is
/// never offered to, or filled into, an embedded frame.
pub(super) fn setup(
    manager: &UserContentManager,
    state: &Rc<NativeState>,
) -> [glib::SignalHandlerId; 2] {
    manager.add_script(&UserScript::for_world(
        HELPER,
        UserContentInjectedFrames::TopFrame,
        UserScriptInjectionTime::Start,
        WORLD,
        &[],
        &[],
    ));
    manager.register_script_message_handler_in_world(CHANGED, WORLD);
    manager.register_script_message_handler_in_world(SENT, WORLD);
    let weak = Rc::downgrade(state);
    let changed = manager.connect_script_message_received(Some(CHANGED), move |_, _| {
        if let Some(state) = Pages::enter_native(&weak) {
            state.look_for_login_fields();
        }
    });
    let weak = Rc::downgrade(state);
    let sent = manager.connect_script_message_received(Some(SENT), move |_, message| {
        if let (Some(state), Some(value)) = (Pages::enter_native(&weak), message.js_value()) {
            state.login_sent(&Zeroizing::new(value.to_str().to_string()));
        }
    });
    [changed, sent]
}

impl NativeState {
    /// Asks the document on screen what sign-in fields it shows, and reports the answer if
    /// that document is still the one on screen.
    pub(super) fn look_for_login_fields(self: &Rc<Self>) {
        let Some(ticket) = self.ticket() else { return };
        self.evaluate(
            "JSON.stringify(globalThis.__canopyLogins?.state() ?? null)",
            move |state, value| {
                if !state.valid(&ticket) {
                    return;
                }
                if let Some(event) = value.and_then(|value| fields(&value, &ticket.page_url)) {
                    state.emit(PageEvent::Login(event));
                }
            },
        );
    }

    /// The helper heard the user send a login. It counts for the committed document's
    /// origin only: a load that has started but not committed leaves that document in place.
    fn login_sent(&self, message: &str) {
        let origin = self
            .navigation
            .borrow()
            .current
            .as_ref()
            .and_then(|document| WebOrigin::of(&document.page_url));
        if let Some(login) = origin.and_then(|origin| sent(message, origin)) {
            self.emit(PageEvent::Login(LoginEvent::Sent(login)));
        }
    }
}

impl LivePage {
    /// Types a saved login into the page's sign-in fields. The page must still be on the
    /// origin the login was saved for, which the helper checks again as it fills.
    pub(in crate::pages) fn fill_login(
        &self,
        origin: &WebOrigin,
        username: &str,
        password: &Secret,
        done: impl FnOnce(Result<()>) + 'static,
    ) {
        let on_origin = self
            .state
            .ticket()
            .is_some_and(|ticket| WebOrigin::of(&ticket.page_url).as_ref() == Some(origin));
        if !on_origin {
            return done(Err(moved()));
        }
        let quoted = |text: &str| {
            Zeroizing::new(serde_json::to_string(text).expect("a string serializes as JSON"))
        };
        let script = Zeroizing::new(format!(
            "JSON.stringify(globalThis.__canopyLogins?.fill({},{},{}) ?? null)",
            *quoted(origin.as_str()),
            *quoted(username),
            *quoted(password.expose()),
        ));
        self.state.evaluate(&script, move |_, value| {
            done(match value.as_ref().and_then(Value::as_str) {
                Some("filled" | "username") => Ok(()),
                Some("origin") => Err(moved()),
                _ => Err(anyhow!("This page has no sign-in fields to fill.")),
            });
        });
    }
}

fn moved() -> anyhow::Error {
    anyhow!("The page changed before the login could be filled.")
}

/// The helper's answer about a document, when that document is of the page's own origin.
fn fields(value: &Value, page: &Url) -> Option<LoginEvent> {
    let document = Url::parse(value["url"].as_str()?).ok()?;
    (document.origin() == page.origin()).then(|| LoginEvent::Fields {
        page: page.clone(),
        username: value["username"] == true,
        password: value["password"] == true,
        asks_password: value["asks"] == true,
    })
}

/// The login in the helper's message, when it was sent from a document of `origin`.
fn sent(message: &str, origin: WebOrigin) -> Option<Login> {
    const LIMIT: usize = 4096;
    let mut value: Value = serde_json::from_str(message).ok()?;
    let login = (|| {
        let password = value["password"].as_str()?;
        let username = value["username"].as_str()?;
        let fits = !password.is_empty() && password.len() <= LIMIT && username.len() <= LIMIT;
        (fits && value["origin"].as_str() == Some(origin.as_str())).then(|| Login {
            username: username.to_owned(),
            password: Secret::new(password.to_owned()),
            last_used: now_ms(),
            origin,
        })
    })();
    if let Some(Value::String(password)) = value.get_mut("password") {
        zeroize::Zeroize::zeroize(password);
    }
    login
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_count_only_for_the_pages_own_origin() {
        let page = Url::parse("https://example.test/account").unwrap();
        let answer =
            |url: &str| json!({ "url": url, "username": false, "password": true, "asks": true });
        let Some(LoginEvent::Fields {
            page: reported,
            username: false,
            password: true,
            asks_password: true,
        }) = fields(&answer("https://example.test/sign-in"), &page)
        else {
            panic!("a document of the page's origin reports its fields");
        };
        assert_eq!(reported, page);
        assert!(fields(&answer("https://other.test/"), &page).is_none());
        assert!(fields(&answer("http://example.test/"), &page).is_none());
        assert!(fields(&Value::Null, &page).is_none());
    }

    #[test]
    fn a_sent_login_counts_only_for_the_committed_origin() {
        let origin = || WebOrigin::of(&Url::parse("https://example.test/").unwrap()).unwrap();
        let message = |origin: &str, username: &str, password: &str| {
            json!({ "origin": origin, "username": username, "password": password }).to_string()
        };
        let login = sent(&message("https://example.test", "alice", "fake"), origin()).unwrap();
        assert_eq!(login.origin, origin());
        assert_eq!(login.username, "alice");
        assert!(login.password.expose() == "fake");
        assert!(login.last_used > 0);
        assert!(sent(&message("https://other.test", "alice", "fake"), origin()).is_none());
        assert!(sent(&message("https://example.test", "alice", ""), origin()).is_none());
        let long = "x".repeat(4097);
        assert!(sent(&message("https://example.test", "alice", &long), origin()).is_none());
        assert!(sent("not json", origin()).is_none());
    }
}
