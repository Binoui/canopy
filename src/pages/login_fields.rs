//! Sign-in fields on the live page: noticing them, and filling the login the user picked.

use std::rc::Rc;

use anyhow::{anyhow, Result};
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
    pages::{PageEvent, Pages},
    secret::Secret,
};

const HELPER: &str = include_str!("../login_fields.js");
/// Script message the helper posts when sign-in fields appear or go away.
const CHANGED: &str = "canopyLoginFields";

/// Adds the helper to the page's isolated world. Only the top frame gets it: a login is
/// never offered to, or filled into, an embedded frame.
pub(super) fn setup(
    manager: &UserContentManager,
    state: &Rc<NativeState>,
) -> glib::SignalHandlerId {
    manager.add_script(&UserScript::for_world(
        HELPER,
        UserContentInjectedFrames::TopFrame,
        UserScriptInjectionTime::Start,
        WORLD,
        &[],
        &[],
    ));
    manager.register_script_message_handler_in_world(CHANGED, WORLD);
    let weak = Rc::downgrade(state);
    manager.connect_script_message_received(Some(CHANGED), move |_, _| {
        if let Some(state) = Pages::enter_native(&weak) {
            state.look_for_login_fields();
        }
    })
}

impl NativeState {
    /// Asks the document on screen whether it shows sign-in fields, and reports the answer
    /// if that document is still the one on screen.
    pub(super) fn look_for_login_fields(self: &Rc<Self>) {
        let Some(ticket) = self.ticket() else { return };
        self.evaluate(
            "JSON.stringify(globalThis.__canopyLogins?.state() ?? null)",
            move |state, value| {
                if state.valid(&ticket) {
                    let page = value.and_then(|value| shown(&value, &ticket.page_url));
                    state.emit(PageEvent::LoginFields(page));
                }
            },
        );
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

/// The page address to look saved logins up for, when the helper's answer shows sign-in
/// fields in a document of the page's own origin.
fn shown(value: &Value, page: &Url) -> Option<Url> {
    let document = Url::parse(value["url"].as_str()?).ok()?;
    let fields = value["username"] == true || value["password"] == true;
    (fields && document.origin() == page.origin()).then(|| page.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_count_only_for_the_pages_own_origin() {
        let page = Url::parse("https://example.test/account").unwrap();
        let answer = |url: &str, username: bool, password: bool| json!({ "url": url, "username": username, "password": password });
        assert_eq!(
            shown(&answer("https://example.test/sign-in", false, true), &page),
            Some(page.clone())
        );
        assert_eq!(
            shown(&answer("https://example.test/", true, false), &page),
            Some(page.clone())
        );
        assert_eq!(
            shown(&answer("https://example.test/", false, false), &page),
            None
        );
        assert_eq!(
            shown(&answer("https://other.test/", true, true), &page),
            None
        );
        assert_eq!(
            shown(&answer("http://example.test/", true, true), &page),
            None
        );
        assert_eq!(shown(&Value::Null, &page), None);
    }
}
