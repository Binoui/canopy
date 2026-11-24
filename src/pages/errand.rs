use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
    time::Duration,
};
use url::Url;
use webkit2gtk::{
    AuthenticationRequestExt, FileChooserRequestExt, PermissionRequestExt, PolicyDecisionExt,
    ResponsePolicyDecisionExt, SettingsExt, URIRequestExt, URIResponseExt, WebViewExt,
};

use super::context::PageContext;

thread_local! {
    static BUSY: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn budget_busy() -> bool {
    BUSY.with(|n| n.get() > 0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrandEnd {
    Done,
    Strayed,
    Failed,
}

#[must_use = "dropping the ticket cancels the errand"]
pub struct ErrandTicket {
    id: u64,
    errands: Weak<Errands>,
}

impl Drop for ErrandTicket {
    fn drop(&mut self) {
        if let Some(errands) = self.errands.upgrade() {
            errands.cancel(self.id);
        }
    }
}

pub(super) struct Errands {
    me: Weak<Self>,
    context: Rc<PageContext>,
    queue: RefCell<VecDeque<Queued>>,
    running: RefCell<Option<Running>>,
    pause: RefCell<Option<glib::SourceId>>,
    next_id: Cell<u64>,
    generation: Cell<u64>,
    load_limit: Cell<Duration>,
}

type Done = Box<dyn FnOnce(ErrandEnd)>;

struct Queued {
    id: u64,
    url: Url,
    linger: Duration,
    done: Done,
}

struct Running {
    id: u64,
    view: webkit2gtk::WebView,
    origin: url::Origin,
    linger: Duration,
    stage: Cell<Stage>,
    /// The page replaced its own load. That load's end is not the document's.
    superseded: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
    signals: Vec<glib::SignalHandlerId>,
    done: Option<Done>,
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Loading,
    Lingering,
    Ending,
}

impl Drop for Running {
    fn drop(&mut self) {
        for signal in self.signals.drain(..) {
            self.view.disconnect(signal);
        }
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        self.view.stop_loading();
        unsafe { self.view.destroy() };
    }
}

impl Errands {
    pub(super) fn new(context: Rc<PageContext>) -> Rc<Self> {
        Rc::new_cyclic(|me| Self {
            me: me.clone(),
            context,
            queue: RefCell::default(),
            running: RefCell::default(),
            pause: RefCell::default(),
            next_id: Cell::new(0),
            generation: Cell::new(0),
            load_limit: Cell::new(Duration::from_secs(30)),
        })
    }

    pub(super) fn run(&self, url: &Url, linger: Duration, done: Done) -> ErrandTicket {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.queue.borrow_mut().push_back(Queued {
            id,
            url: url.clone(),
            linger,
            done,
        });
        self.pump();
        ErrandTicket {
            id,
            errands: self.me.clone(),
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.running.borrow().is_some() || self.pause.borrow().is_some()
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation.get()
    }

    fn pump(&self) {
        if self.busy() {
            return;
        }
        let Some(queued) = self.queue.borrow_mut().pop_front() else {
            return;
        };
        let url = queued.url.clone();
        let running = self.open(queued);
        let view = running.view.clone();
        BUSY.with(|n| n.set(n.get() + 1));
        *self.running.borrow_mut() = Some(running);
        self.generation.set(self.generation.get() + 1);
        view.load_uri(url.as_str());
    }

    fn open(&self, queued: Queued) -> Running {
        let id = queued.id;
        let settings = webkit2gtk::Settings::new();
        settings.set_user_agent(None);
        settings.set_enable_webgl(false);
        settings.set_enable_webaudio(false);
        settings.set_enable_media(false);
        settings.set_enable_page_cache(false);
        settings.set_enable_developer_extras(false);
        let view = webkit2gtk::WebView::builder()
            .web_context(self.context.temporary_webkit().temporary_web_context())
            .user_content_manager(&webkit2gtk::UserContentManager::new())
            .settings(&settings)
            .is_controlled_by_automation(false)
            .build();
        view.set_is_muted(true);
        self.context.engine().refuse_downloads(view.upcast_ref());
        let mut signals = vec![
            view.connect_create(|_, _| None),
            view.connect_close(|_| {}),
            view.connect_script_dialog(|_, dialog| {
                if matches!(
                    dialog.dialog_type(),
                    webkit2gtk::ScriptDialogType::Confirm
                        | webkit2gtk::ScriptDialogType::BeforeUnloadConfirm
                ) {
                    dialog.confirm_set_confirmed(false);
                }
                true
            }),
            view.connect_permission_request(|_, request| {
                request.deny();
                true
            }),
            view.connect_show_notification(|_, _| true),
            view.connect_run_file_chooser(|_, request| {
                request.cancel();
                true
            }),
            view.connect_enter_fullscreen(|_| true),
            view.connect_print(|_, _| true),
        ];
        let weak = self.me.clone();
        signals.push(view.connect_decide_policy(move |_, decision, kind| {
            if kind == webkit2gtk::PolicyDecisionType::NewWindowAction {
                decision.ignore();
                return true;
            }
            let Some(response) = decision.downcast_ref::<webkit2gtk::ResponsePolicyDecision>()
            else {
                return false;
            };
            let Some(owner) = weak.upgrade() else {
                decision.ignore();
                return true;
            };
            let main = response.is_main_frame_main_resource();
            let strayed = main
                && !response
                    .request()
                    .and_then(|r| r.uri())
                    .is_some_and(|uri| owner.stays(id, &uri));
            let attachment = response
                .response()
                .and_then(|r| r.http_headers())
                .and_then(|h| h.content_disposition())
                .is_some_and(|(d, _)| d.eq_ignore_ascii_case("attachment"));
            if strayed || !response.is_mime_type_supported() || attachment {
                decision.ignore();
                if main {
                    owner.end(
                        id,
                        if strayed {
                            ErrandEnd::Strayed
                        } else {
                            ErrandEnd::Failed
                        },
                    );
                }
                return true;
            }
            false
        }));
        let weak = self.me.clone();
        signals.push(view.connect_load_changed(move |view, event| {
            let Some(owner) = weak.upgrade() else { return };
            if matches!(
                event,
                webkit2gtk::LoadEvent::Committed | webkit2gtk::LoadEvent::Finished
            ) && !view.uri().is_some_and(|uri| owner.stays(id, &uri))
            {
                owner.end(id, ErrandEnd::Strayed);
            } else if event == webkit2gtk::LoadEvent::Started {
                owner.superseded(id, false);
            } else if event == webkit2gtk::LoadEvent::Finished && !owner.superseded(id, false) {
                owner.loaded(id);
            }
        }));
        let weak = self.me.clone();
        signals.push(view.connect_load_failed(move |_, _, _, error| {
            if let Some(owner) = weak.upgrade() {
                // A page that navigates from script before its first load ends cancels that load.
                if error.matches(webkit2gtk::NetworkError::Cancelled) {
                    owner.superseded(id, true);
                } else {
                    owner.load_failed(id);
                }
            }
            true
        }));
        let weak = self.me.clone();
        signals.push(view.connect_load_failed_with_tls_errors(move |_, _, _, _| {
            if let Some(owner) = weak.upgrade() {
                owner.load_failed(id);
            }
            true
        }));
        let weak = self.me.clone();
        signals.push(view.connect_web_process_terminated(move |_, _| {
            if let Some(owner) = weak.upgrade() {
                owner.end(id, ErrandEnd::Failed);
            }
        }));
        let weak = self.me.clone();
        signals.push(view.connect_authenticate(move |_, request| {
            request.cancel();
            if let Some(owner) = weak.upgrade() {
                owner.end(id, ErrandEnd::Failed);
            }
            true
        }));
        let weak = self.me.clone();
        let timer = glib::timeout_add_local_once(self.load_limit.get(), move || {
            if let Some(owner) = weak.upgrade() {
                if let Some(running) = owner.running.borrow().as_ref() {
                    running.timer.borrow_mut().take();
                }
                owner.end(id, ErrandEnd::Failed);
            }
        });
        Running {
            id,
            view: view.clone(),
            origin: queued.url.origin(),
            linger: queued.linger,
            stage: Cell::new(Stage::Loading),
            superseded: Cell::new(false),
            timer: RefCell::new(Some(timer)),
            signals,
            done: Some(queued.done),
        }
    }

    fn stays(&self, id: u64, uri: &str) -> bool {
        self.running
            .borrow()
            .as_ref()
            .is_some_and(|r| r.id == id && stays(&r.origin, uri))
    }

    fn superseded(&self, id: u64, now: bool) -> bool {
        self.running
            .borrow()
            .as_ref()
            .filter(|r| r.id == id)
            .is_some_and(|r| r.superseded.replace(now))
    }

    fn load_failed(&self, id: u64) {
        let loading = self
            .running
            .borrow()
            .as_ref()
            .is_some_and(|r| r.id == id && r.stage.get() == Stage::Loading);
        if loading {
            self.end(id, ErrandEnd::Failed);
        }
    }

    fn loaded(&self, id: u64) {
        let running = self.running.borrow();
        let Some(r) = running
            .as_ref()
            .filter(|r| r.id == id && r.stage.get() == Stage::Loading)
        else {
            return;
        };
        r.stage.set(Stage::Lingering);
        if let Some(timer) = r.timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = self.me.clone();
        let timer = glib::timeout_add_local_once(r.linger, move || {
            if let Some(owner) = weak.upgrade() {
                if let Some(r) = owner.running.borrow().as_ref() {
                    r.timer.borrow_mut().take();
                }
                owner.end(id, ErrandEnd::Done);
            }
        });
        *r.timer.borrow_mut() = Some(timer);
    }

    fn end(&self, id: u64, end: ErrandEnd) {
        let view = {
            let running = self.running.borrow();
            let Some(r) = running.as_ref().filter(|r| r.id == id) else {
                return;
            };
            if r.stage.replace(Stage::Ending) == Stage::Ending {
                return;
            }
            r.view.clone()
        };
        view.stop_loading();
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(owner) = weak.upgrade() {
                owner.retire(id, Some(end));
            }
        });
    }

    fn retire(&self, id: u64, end: Option<ErrandEnd>) {
        let mut running = self.running.borrow_mut();
        if running.as_ref().is_none_or(|r| r.id != id) {
            return;
        }
        let mut r = running.take().unwrap();
        drop(running);
        let done = r.done.take();
        drop(r);
        self.generation.set(self.generation.get() + 1);
        let weak = self.me.clone();
        *self.pause.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_secs(1),
            move || {
                if let Some(owner) = weak.upgrade() {
                    owner.pause.borrow_mut().take();
                    BUSY.with(|n| n.set(n.get() - 1));
                    owner.pump();
                }
            },
        ));
        if let (Some(end), Some(done)) = (end, done) {
            done(end);
        }
    }

    fn cancel(&self, id: u64) {
        self.queue.borrow_mut().retain(|q| q.id != id);
        // Tickets can drop during a WebKit callback, so destruction belongs on idle too.
        if let Some(r) = self.running.borrow_mut().as_mut().filter(|r| r.id == id) {
            r.done.take();
        }
        self.end(id, ErrandEnd::Failed);
    }

    #[cfg(feature = "gui-smoke")]
    pub(super) fn set_load_limit(&self, limit: Duration) {
        self.load_limit.set(limit);
    }

    #[cfg(feature = "gui-smoke")]
    pub(super) fn open_pages(&self) -> usize {
        usize::from(self.running.borrow().is_some())
    }
}

impl Drop for Errands {
    fn drop(&mut self) {
        if self.running.get_mut().is_some() || self.pause.get_mut().is_some() {
            BUSY.with(|n| n.set(n.get() - 1));
        }
        if let Some(timer) = self.pause.get_mut().take() {
            timer.remove();
        }
    }
}

fn stays(origin: &url::Origin, uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|u| u.origin().is_tuple() && &u.origin() == origin)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_same_web_origin_stays() {
        let origin = Url::parse("https://example.test:443/a").unwrap().origin();
        assert!(stays(&origin, "https://example.test/b"));
        for uri in [
            "http://example.test",
            "https://other.test",
            "https://example.test:444",
            "about:blank",
            "data:text/html,x",
            "file:///x",
            "broken",
        ] {
            assert!(!stays(&origin, uri), "{uri}");
        }
    }
}
