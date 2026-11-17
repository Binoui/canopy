use serde_json::{json, Value};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Errors {
    recent: VecDeque<(String, i64)>,
    pending: usize,
    unread: bool,
}

impl Errors {
    pub fn push(&mut self, text: &str) {
        self.recent.push_front((
            text.to_owned(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
        ));
        self.recent.truncate(10);
        self.pending = self.pending.saturating_add(1);
        self.unread = true;
    }

    pub fn act(&mut self, action: &str) {
        match action {
            "dismiss" => self.pending = 0,
            "read" => self.unread = false,
            "clear" => {
                self.recent.clear();
                self.pending = 0;
                self.unread = false;
            }
            _ => {}
        }
    }

    pub fn showing(&self) -> bool {
        self.pending > 0
    }

    pub fn value(&self) -> Value {
        json!({
            "unread": self.unread,
            "toast": if self.showing() { self.recent.front().map(|(text, _)| json!({"text": text, "more": self.pending - 1})) } else { None },
            "recent": self.recent.iter().map(|(text, at)| json!({"text": text, "when": super::view::ago(*at)})).collect::<Vec<_>>()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_keep_newest_ten_and_stay_until_dismissed() {
        let mut errors = Errors::default();
        for n in 0..12 {
            errors.push(&format!("Error {n}"));
        }
        let value = errors.value();
        assert_eq!(value["recent"].as_array().unwrap().len(), 10);
        assert_eq!(value["recent"][0]["text"], "Error 11");
        assert_eq!(value["toast"]["more"], 11);
        assert_eq!(value["unread"], true);
        errors.act("read");
        assert_eq!(errors.value()["unread"], false);
        assert!(!errors.value()["toast"].is_null());
        errors.act("dismiss");
        assert!(errors.value()["toast"].is_null());
        assert_eq!(errors.value()["recent"].as_array().unwrap().len(), 10);
        errors.push("Another error");
        assert_eq!(errors.value()["toast"]["more"], 0);
        assert_eq!(errors.value()["unread"], true);
        errors.act("clear");
        assert_eq!(errors.value()["recent"], json!([]));
        assert_eq!(errors.value()["unread"], false);
    }
}
