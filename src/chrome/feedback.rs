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

/// A separate, tightly allocated view keeps a lasting error off the page's video surface.
pub(super) struct ErrorView {
    pub view: wry::WebView,
    container: gtk::Fixed,
    pub ready: std::cell::Cell<bool>,
    pub size: std::cell::Cell<(i32, i32)>,
    pub buttons: std::cell::RefCell<Vec<[i32; 4]>>,
}

impl ErrorView {
    pub fn new(container: &gtk::Fixed, inner: std::rc::Weak<super::Inner>) -> anyhow::Result<Self> {
        use gtk::prelude::*;
        use wry::{WebViewBuilderExtUnix, WebViewExtUnix};
        let view = wry::WebViewBuilder::new()
            .with_transparent(true)
            .with_visible(false)
            .with_focused(false)
            .with_bounds(super::rect(0., 0., 1., 1.))
            .with_custom_protocol("canopy".into(), |_, request| super::serve(&request))
            .with_ipc_handler(move |request| {
                let inner = inner.clone();
                let body = request.into_body();
                glib::idle_add_local_once(move || {
                    if let Some(inner) = inner.upgrade() {
                        inner.message(&body);
                    }
                });
            })
            .with_url("canopy://localhost/error.html")
            .build_gtk(container)?;
        crate::pages::stack::Stack::feedback(container, view.webview().upcast_ref());
        Ok(Self {
            view,
            container: container.clone(),
            ready: std::cell::Cell::new(false),
            size: std::cell::Cell::new((0, 0)),
            buttons: std::cell::RefCell::default(),
        })
    }

    pub fn bounds(&self, window: (f64, f64), showing: bool) {
        use gtk::prelude::*;
        use wry::WebViewExtUnix;
        let native = self.view.webview();
        let (width, height) = self.size.get();
        if !showing || width == 0 || height == 0 {
            native.hide();
            return;
        }
        // Leave room for the shadow inside this view and for the page scrollbar outside it.
        let (width, height) = (width + 32, height + 32);
        let x = ((window.0 - width as f64) / 2.).round() as i32;
        let y = (window.1 - height as f64 - 2.).round() as i32;
        native.show();
        let allocation = gtk::Allocation::new(x, y, width, height);
        if native.allocation() != allocation {
            self.container.move_(&native, x, y);
            native.set_size_request(width, height);
            let _ =
                self.view
                    .set_bounds(super::rect(x as f64, y as f64, width as f64, height as f64));
            native.size_allocate(&allocation);
        }
        if let Some(window) = native.window() {
            let region = gtk::cairo::Region::create();
            for r in self.buttons.borrow().iter() {
                let _ =
                    region.union_rectangle(&gtk::cairo::RectangleInt::new(r[0], r[1], r[2], r[3]));
            }
            window.input_shape_combine_region(&region, 0, 0);
            window.raise();
        }
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
