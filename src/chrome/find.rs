//! The find bar in the header: what Rust sends it, and what it sends back.

use serde_json::{json, Value};

use super::{ChromeIntent, HtmlChrome};
use crate::pages::{FindAction, FindClose, FindMatches, FindReadout, FindUpdate};

impl HtmlChrome {
    pub fn find(&self, update: FindUpdate) {
        let inner = &self.inner;
        match update {
            FindUpdate::Focused(readout) => {
                inner.find.set(Some(readout.id));
                inner.call("find", &[value(&readout), json!("focus")]);
                let _ = inner.view.focus();
            }
            FindUpdate::Shown(readout) => inner.call("find", &[value(&readout), json!("show")]),
            FindUpdate::Closed(FindClose::User) => {
                inner.find.set(None);
                inner.call("find", &[Value::Null, json!("user")]);
                if let Some(host) = inner.host() {
                    host.focus_page();
                }
            }
            FindUpdate::Closed(FindClose::ContextChanged) => {
                inner.find.set(None);
                inner.call("find", &[Value::Null, json!("context")]);
            }
        }
    }
}

fn value(readout: &FindReadout) -> Value {
    let matches = match readout.matches {
        FindMatches::EmptyQuery => json!({ "state": "empty" }),
        FindMatches::Pending => json!({ "state": "pending" }),
        FindMatches::NoMatches => json!({ "state": "none" }),
        FindMatches::Position(at) => {
            json!({ "state": "position", "index": at.index(), "total": at.total() })
        }
    };
    json!({ "id": readout.id.get(), "query": readout.query, "matches": matches })
}

pub(super) fn intent(message: &Value) -> Option<ChromeIntent> {
    let action = match message["action"].as_str()? {
        // A NUL cannot reach WebKit's C strings.
        "query" => FindAction::Query(
            message["text"]
                .as_str()
                .filter(|text| !text.contains('\0'))?
                .to_owned(),
        ),
        "next" => FindAction::Next,
        "previous" => FindAction::Previous,
        "close" => FindAction::Close,
        _ => return None,
    };
    Some(ChromeIntent::Find {
        session: message["session"].as_u64()?.into(),
        action,
    })
}
