//! WebKit's signals on a page view, translated into `ViewEvent`s.
//!
//! Every handler hands its event to the observer at once, inside WebKit's own emission, and
//! returns the observer's answer as WebKit's. Nothing is queued or reordered: one WebKit signal
//! is one event.

use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use gtk::prelude::*;

use glib::SignalHandlerId;
use javascriptcore::ValueExt;
use webkit2gtk::{
    AuthenticationRequest, BackForwardListExt, BackForwardListItem, FileChooserRequest,
    FileChooserRequestExt, FindControllerExt, NavigationPolicyDecision,
    NavigationPolicyDecisionExt, NetworkError, PermissionRequest, PermissionRequestExt,
    PolicyDecisionExt, PolicyError, ResponsePolicyDecision, ResponsePolicyDecisionExt,
    URIRequestExt, URIResponseExt, UserContentManager, UserContentManagerExt, WebView, WebViewExt,
    WindowPropertiesExt,
};

use super::view::{entry_id, entry_id_of_raw, history_item, WebKitView};
use crate::pages::engine::{
    Decide, EngineView, LoadEvent, LoadFailure, LoadFailureKind, NavigationAction, NavigationType,
    NewWindow, Observer, PolicyDecision, PolicyDecisionType, Request, Responder, Response,
    ResponseDetails, ScriptDialog, ScriptDialogType, ViewEvent,
};

pub(super) type ObserverCell = Rc<RefCell<Option<Observer>>>;

/// Hands `event` to the observer. Without one, the event is unhandled.
fn emit(observer: &ObserverCell, event: ViewEvent) -> bool {
    // The observer may replace or drop itself while it runs.
    let current = observer.borrow().clone();
    current.is_some_and(|observer| observer(event))
}

/// Connects every signal Canopy hears on `native`, in the order Canopy connected them before
/// the engine layer.
pub(super) fn connect(
    native: &WebView,
    manager: &UserContentManager,
    message_names: &[&'static str],
    observer: &ObserverCell,
    parent: Option<glib::WeakRef<gtk::Window>>,
) -> Vec<(glib::Object, SignalHandlerId)> {
    let view_object: glib::Object = native.clone().upcast();
    let mut signals = Vec::new();
    let mut on_view = |id: SignalHandlerId| signals.push((view_object.clone(), id));

    let cell = observer.clone();
    on_view(native.connect_close(move |_| {
        emit(&cell, ViewEvent::Close);
    }));
    let cell = observer.clone();
    on_view(native.connect_create(move |opener, action| {
        let request = Rc::new(WindowRequest {
            opener: opener.clone(),
            uri: action
                .request()
                .and_then(|request| request.uri())
                .map(Into::into),
            made: RefCell::new(None),
        });
        emit(&cell, ViewEvent::Create(Box::new(request.clone())));
        let made = request.made.borrow_mut().take();
        made.map(|view| view.native().clone().upcast())
    }));
    let cell = observer.clone();
    on_view(native.connect_ready_to_show(move |view| {
        let parent = parent.as_ref().and_then(|parent| parent.upgrade());
        emit(
            &cell,
            ViewEvent::ReadyToShow {
                size: requested_size(view, parent.as_ref()),
            },
        );
    }));
    let cell = observer.clone();
    on_view(native.connect_show_notification(move |_, _| emit(&cell, ViewEvent::ShowNotification)));
    let cell = observer.clone();
    on_view(native.connect_script_dialog(move |_, dialog| {
        emit(
            &cell,
            ViewEvent::ScriptDialog(Box::new(Dialog(dialog.clone()))),
        )
    }));
    let cell = observer.clone();
    on_view(native.connect_permission_request(move |_, request| {
        emit(
            &cell,
            ViewEvent::PermissionRequest(Box::new(Permission(request.clone()))),
        )
    }));
    let cell = observer.clone();
    on_view(native.connect_run_file_chooser(move |_, request| {
        emit(
            &cell,
            ViewEvent::FileChooser(Box::new(FileChooser(request.clone()))),
        )
    }));
    let cell = observer.clone();
    on_view(native.connect_enter_fullscreen(move |_| emit(&cell, ViewEvent::EnterFullscreen)));
    let cell = observer.clone();
    on_view(native.connect_leave_fullscreen(move |_| emit(&cell, ViewEvent::LeaveFullscreen)));
    let cell = observer.clone();
    on_view(native.connect_print(move |_, _| emit(&cell, ViewEvent::Print)));
    let cell = observer.clone();
    on_view(native.connect_web_process_terminated(move |_, _| {
        emit(&cell, ViewEvent::WebProcessTerminated);
    }));
    let cell = observer.clone();
    on_view(native.connect_authenticate(move |_, request| {
        emit(
            &cell,
            ViewEvent::Authenticate(Box::new(Authentication(request.clone()))),
        )
    }));
    let manager_object: glib::Object = manager.clone().upcast();
    let mut messages = Vec::new();
    for &name in message_names {
        let cell = observer.clone();
        let id = manager.connect_script_message_received(Some(name), move |_, message| {
            let body = message
                .js_value()
                .map(|value| zeroize::Zeroizing::new(value.to_str().to_string()));
            emit(
                &cell,
                ViewEvent::ScriptMessage {
                    name: name.to_owned(),
                    body,
                },
            );
        });
        messages.push((manager_object.clone(), id));
    }
    let cell = observer.clone();
    on_view(native.connect_estimated_load_progress_notify(move |_| {
        emit(&cell, ViewEvent::ProgressChanged);
    }));
    let mut find = Vec::new();
    if let Some(controller) = native.find_controller() {
        let object: glib::Object = controller.clone().upcast();
        let cell = observer.clone();
        let found = controller.connect_found_text(move |_, count| {
            emit(&cell, ViewEvent::FoundText(count));
        });
        let cell = observer.clone();
        let failed = controller.connect_failed_to_find_text(move |_| {
            emit(&cell, ViewEvent::FailedToFindText);
        });
        find.push((object.clone(), found));
        find.push((object, failed));
    }
    let cell = observer.clone();
    on_view(native.connect_load_changed(move |_, event| {
        let event = match event {
            webkit2gtk::LoadEvent::Started => LoadEvent::Started,
            webkit2gtk::LoadEvent::Redirected => LoadEvent::Redirected,
            webkit2gtk::LoadEvent::Committed => LoadEvent::Committed,
            webkit2gtk::LoadEvent::Finished => LoadEvent::Finished,
            _ => return,
        };
        emit(&cell, ViewEvent::Load(event));
    }));
    let cell = observer.clone();
    on_view(native.connect_load_failed(move |_, _, _uri, error| {
        let kind = if error.matches(NetworkError::Cancelled) {
            LoadFailureKind::Cancelled
        } else if error.matches(PolicyError::FrameLoadInterruptedByPolicyChange) {
            LoadFailureKind::InterruptedByPolicyChange
        } else {
            LoadFailureKind::Other
        };
        emit(
            &cell,
            ViewEvent::LoadFailed(LoadFailure {
                kind,
                message: error.message().to_owned(),
            }),
        )
    }));
    let cell = observer.clone();
    on_view(
        native.connect_load_failed_with_tls_errors(move |_, _uri, _, _| {
            emit(&cell, ViewEvent::LoadFailedWithTlsErrors)
        }),
    );
    let cell = observer.clone();
    on_view(native.connect_title_notify(move |_| {
        emit(&cell, ViewEvent::TitleChanged);
    }));
    let cell = observer.clone();
    on_view(native.connect_is_playing_audio_notify(move |_| {
        emit(&cell, ViewEvent::PlayingAudioChanged);
    }));
    let mut history = Vec::new();
    if let Some(list) = native.back_forward_list() {
        let cell = observer.clone();
        let id = list.connect_local("changed", false, move |values| {
            let list: webkit2gtk::BackForwardList = values[0].get().unwrap();
            let added: Option<BackForwardListItem> = values[1].get().unwrap();
            let current = list.current_item().as_ref().map(entry_id);
            let removed: glib::Pointer = values[2].get().unwrap_or(std::ptr::null_mut());
            let mut removed_ids = Vec::new();
            // WebKit keeps removed objects readable while our bindings retain them.
            unsafe {
                let mut node = removed as *mut glib::ffi::GList;
                while !node.is_null() {
                    removed_ids.push(entry_id_of_raw(
                        (*node).data as *mut webkit2gtk::ffi::WebKitBackForwardListItem,
                    ));
                    node = (*node).next;
                }
            }
            emit(
                &cell,
                ViewEvent::HistoryChanged {
                    added: added.as_ref().map(history_item),
                    removed: removed_ids,
                    current,
                },
            );
            None
        });
        history.push((list.upcast(), id));
    }
    let cell = observer.clone();
    on_view(native.connect_uri_notify(move |_| {
        emit(&cell, ViewEvent::UriChanged);
    }));
    let cell = observer.clone();
    on_view(native.connect_is_loading_notify(move |_| {
        emit(&cell, ViewEvent::LoadingChanged);
    }));
    let cell = observer.clone();
    on_view(native.connect_decide_policy(move |_, decision, kind| {
        let kind = match kind {
            webkit2gtk::PolicyDecisionType::NavigationAction => {
                PolicyDecisionType::NavigationAction
            }
            webkit2gtk::PolicyDecisionType::NewWindowAction => PolicyDecisionType::NewWindowAction,
            webkit2gtk::PolicyDecisionType::Response => PolicyDecisionType::Response,
            _ => return false,
        };
        emit(&cell, ViewEvent::Decide(policy_decision(decision), kind))
    }));
    let cell = observer.clone();
    on_view(
        native.connect_resource_load_started(move |native, resource, request| {
            if native.main_resource().as_ref() != Some(resource) {
                return;
            }
            if !emit(&cell, ViewEvent::MainResourceStarted(request_of(request))) {
                return;
            }
            let cell = cell.clone();
            let native = native.downgrade();
            // The typed binding treats the absent redirect response as non-null.
            resource.connect_local("sent-request", false, move |values| {
                // A panic here would cross the signal boundary and abort the browser.
                let (Some(Ok(resource)), Some(Ok(request))) = (
                    values.first().map(|v| v.get::<webkit2gtk::WebResource>()),
                    values.get(1).map(|v| v.get::<webkit2gtk::URIRequest>()),
                ) else {
                    return None;
                };
                let native = native.upgrade()?;
                if native.main_resource().as_ref() == Some(&resource) {
                    emit(&cell, ViewEvent::MainRequestSent(request_of(&request)));
                }
                None
            });
        }),
    );
    let cell = observer.clone();
    on_view(native.connect_favicon_notify(move |page| {
        emit(
            &cell,
            ViewEvent::FaviconChanged {
                uri: page.uri().map(Into::into),
                favicon: page.favicon(),
            },
        );
    }));
    signals.extend(messages);
    signals.extend(find);
    signals.extend(history);
    signals
}

/// The size the page asked for a window it opened. WebKit reports every window feature as
/// given, and gives a `window.open` without a size the opener's window size.
fn requested_size(view: &WebView, parent: Option<&gtk::Window>) -> Option<(i32, i32)> {
    let geometry = view.window_properties()?.geometry();
    let size = (geometry.width(), geometry.height());
    (size.0 > 0 && size.1 > 0 && parent.is_none_or(|parent| parent.size() != size)).then_some(size)
}

struct WindowRequest {
    opener: WebView,
    uri: Option<String>,
    /// The view `open` made, handed to WebKit once the observer returns.
    made: RefCell<Option<Rc<WebKitView>>>,
}

impl NewWindow for Rc<WindowRequest> {
    fn uri(&self) -> Option<String> {
        self.uri.clone()
    }

    fn open(&self) -> Result<Rc<dyn EngineView>> {
        if self.made.borrow().is_some() {
            anyhow::bail!("This window already has a view.");
        }
        let view = WebKitView::related(&self.opener);
        *self.made.borrow_mut() = Some(view.clone());
        Ok(view)
    }
}

fn request_of(request: &webkit2gtk::URIRequest) -> Request {
    Request {
        uri: request.uri().map(Into::into),
        http_method: request.http_method().map(Into::into),
    }
}

fn policy_decision(decision: &webkit2gtk::PolicyDecision) -> PolicyDecision {
    let navigation_action = decision
        .downcast_ref::<NavigationPolicyDecision>()
        .and_then(|decision| decision.navigation_action())
        .map(|mut action| NavigationAction {
            navigation_type: match action.navigation_type() {
                webkit2gtk::NavigationType::LinkClicked => NavigationType::LinkClicked,
                webkit2gtk::NavigationType::FormSubmitted => NavigationType::FormSubmitted,
                webkit2gtk::NavigationType::BackForward => NavigationType::BackForward,
                webkit2gtk::NavigationType::Reload => NavigationType::Reload,
                webkit2gtk::NavigationType::FormResubmitted => NavigationType::FormResubmitted,
                _ => NavigationType::Other,
            },
            frame_name: action.frame_name().map(Into::into),
            modifiers: gdk::ModifierType::from_bits_truncate(action.modifiers()),
            mouse_button: action.mouse_button(),
            user_gesture: action.is_user_gesture(),
            request: action.request().as_ref().map(request_of),
        });
    let response = decision
        .downcast_ref::<ResponsePolicyDecision>()
        .map(|decision| ResponseDetails {
            main_frame_main_resource: decision.is_main_frame_main_resource(),
            request: decision.request().as_ref().map(request_of),
            response: decision.response().map(|response| Response {
                status_code: response.status_code(),
                content_disposition: response
                    .http_headers()
                    .and_then(|headers| headers.content_disposition())
                    .map(|(disposition, _)| disposition.into()),
            }),
            mime_type_supported: decision.is_mime_type_supported(),
        });
    PolicyDecision::new(
        Rc::new(Decision(decision.clone())),
        navigation_action,
        response,
    )
}

struct Decision(webkit2gtk::PolicyDecision);

impl Decide for Decision {
    fn use_(&self) {
        self.0.use_();
    }
    fn ignore(&self) {
        self.0.ignore();
    }
    fn download(&self) {
        self.0.download();
    }
}

struct Dialog(webkit2gtk::ScriptDialog);

impl ScriptDialog for Dialog {
    fn dialog_type(&self) -> ScriptDialogType {
        match self.0.dialog_type() {
            webkit2gtk::ScriptDialogType::Confirm => ScriptDialogType::Confirm,
            webkit2gtk::ScriptDialogType::Prompt => ScriptDialogType::Prompt,
            webkit2gtk::ScriptDialogType::BeforeUnloadConfirm => {
                ScriptDialogType::BeforeUnloadConfirm
            }
            // An unknown kind asks nothing Canopy answers, as an alert.
            _ => ScriptDialogType::Alert,
        }
    }
    fn confirm_set_confirmed(&self, confirmed: bool) {
        self.0.confirm_set_confirmed(confirmed);
    }
}

struct Permission(PermissionRequest);

impl Responder for Permission {
    fn refuse(&self) {
        self.0.deny();
    }
}

struct FileChooser(FileChooserRequest);

impl Responder for FileChooser {
    fn refuse(&self) {
        self.0.cancel();
    }
}

struct Authentication(AuthenticationRequest);

impl Responder for Authentication {
    fn refuse(&self) {
        use webkit2gtk::AuthenticationRequestExt;
        self.0.cancel();
    }
}
