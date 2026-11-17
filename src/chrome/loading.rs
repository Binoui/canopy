use super::HtmlChrome;
use crate::pages::{LoadProgress, LoadStatus, LoadingReadout};
use serde_json::json;

impl HtmlChrome {
    pub fn loading(&self, value: LoadingReadout) {
        let mut status = match value.status {
            LoadStatus::Idle => json!({ "state": "idle" }),
            LoadStatus::Busy(progress) => json!({
                "state": "busy",
                "progress": match progress {
                    LoadProgress::Waiting => None,
                    LoadProgress::Measured(value) => Some(value.percent()),
                },
            }),
        };
        status["page"] = json!(value.page.map(|id| id.to_string()));
        self.inner.call("loading", &[status]);
    }
}
