use super::*;
use crate::pages::{PageScaleAction, PageScaleReadout};

impl HtmlChrome {
    pub fn page_scale(&self, value: PageScaleReadout) {
        let value = match value {
            PageScaleReadout::NoPage => Value::Null,
            PageScaleReadout::Page { visit, level } => {
                json!({"page": visit.to_string(), "percent": level.percent()})
            }
        };
        self.inner.call("pageScale", &[value]);
    }
}

pub(super) fn intent(message: &Value) -> Option<ChromeIntent> {
    let action = match message["action"].as_str()? {
        "increase" => PageScaleAction::Increase,
        "decrease" => PageScaleAction::Decrease,
        "reset" => PageScaleAction::Reset,
        _ => return None,
    };
    Some(ChromeIntent::PageScale(action))
}
