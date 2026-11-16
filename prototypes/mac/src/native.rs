//! Everything the prototype needs from AppKit and WKWebView that wry does not offer itself.
//! Each function here is a call the real Mac backend would need too.

use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow, NSWindowOrderingMode};
use objc2_foundation::{NSData, NSDate, NSDefaultRunLoopMode, NSPoint, NSRunLoop};
use objc2_web_kit::WKWebView;
use wry::WebViewExtMacOS;

/// Lets the main run loop work for a moment: page callbacks, drawing and timers all run here.
pub fn pump(seconds: f64) {
    let until = NSDate::dateWithTimeIntervalSinceNow(seconds);
    unsafe {
        NSRunLoop::currentRunLoop().runMode_beforeDate(NSDefaultRunLoopMode, &until);
    }
}

/// Pumps the run loop until `done` says so or the time is up. Returns whether it finished.
pub fn wait(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if done() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        pump(0.02);
    }
}

pub fn wk(view: &wry::WebView) -> Retained<WKWebView> {
    Retained::into_super(view.webview())
}

fn responds(view: &WKWebView, selector: Sel) -> bool {
    unsafe { msg_send![view, respondsToSelector: selector] }
}

pub struct History {
    pub back: usize,
    pub forward: usize,
    pub can_go_back: bool,
}

pub fn history(view: &wry::WebView) -> History {
    let wk = wk(view);
    unsafe {
        let list = wk.backForwardList();
        History {
            back: list.backList().count(),
            forward: list.forwardList().count(),
            can_go_back: wk.canGoBack(),
        }
    }
}

pub fn go_back(view: &wry::WebView) {
    unsafe {
        let _ = wk(view).goBack();
    }
}

pub fn is_loading(view: &wry::WebView) -> bool {
    unsafe { wk(view).isLoading() }
}

pub fn title(view: &wry::WebView) -> String {
    unsafe { wk(view).title() }.map(|t| t.to_string()).unwrap_or_default()
}

/// A page's saved session: its back and forward list with scroll positions and form state.
pub struct Checkpoint {
    state: Retained<AnyObject>,
    pub class: String,
    pub bytes: Option<usize>,
}

/// Reads the view's session state (`interactionState`, macOS 12 and later).
pub fn checkpoint(view: &wry::WebView) -> Option<Checkpoint> {
    let wk = wk(view);
    if !responds(&wk, sel!(interactionState)) {
        return None;
    }
    let state = unsafe { wk.interactionState() }?;
    let class = state.class().name().to_string_lossy().into_owned();
    let bytes = state.downcast_ref::<NSData>().map(|data| data.len());
    Some(Checkpoint { state, class, bytes })
}

pub fn restore(view: &wry::WebView, checkpoint: &Checkpoint) {
    unsafe { wk(view).setInteractionState(Some(&checkpoint.state)) }
}

/// The process that renders this view's page. Private selector, so it may be missing.
pub fn page_process(view: &wry::WebView) -> Option<i32> {
    let wk = wk(view);
    let selector = sel!(_webProcessIdentifier);
    if !responds(&wk, selector) {
        return None;
    }
    let pid: i32 = unsafe { msg_send![&*wk, _webProcessIdentifier] };
    (pid > 0).then_some(pid)
}

/// Whether the page is making sound right now. Private selector, so it may be missing.
pub fn playing_audio(view: &wry::WebView) -> Option<bool> {
    let wk = wk(view);
    let selector = sel!(_isPlayingAudio);
    if !responds(&wk, selector) {
        return None;
    }
    Some(unsafe { msg_send![&*wk, _isPlayingAudio] })
}

/// Closes the page for good, which lets its process exit. wry keeps a dropped view alive on
/// purpose, so without this the page's memory is never returned. Private selector.
pub fn close(view: &wry::WebView) -> bool {
    let wk = wk(view);
    let selector = sel!(_close);
    if !responds(&wk, selector) {
        return false;
    }
    let _: () = unsafe { msg_send![&*wk, _close] };
    true
}

pub fn process_alive(pid: i32) -> bool {
    footprint_mib(pid).is_some()
}

/// Memory the system charges to a process, in MiB (the figure Activity Monitor shows).
pub fn footprint_mib(pid: i32) -> Option<f64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    let result = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, info.as_mut_ptr().cast()) };
    if result != 0 {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some(info.ri_phys_footprint as f64 / (1024.0 * 1024.0))
}

/// WebKit's shared helper processes on this machine (network, graphics), as `(name, pid)`.
pub fn helper_processes() -> Vec<(String, i32)> {
    let Ok(output) = std::process::Command::new("ps").args(["-axo", "pid=,comm="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(' ')?;
            let name = command.trim().rsplit('/').next()?;
            let wanted = name.starts_with("com.apple.WebKit.Networking") || name.starts_with("com.apple.WebKit.GPU");
            wanted.then(|| Some((name.to_string(), pid.parse().ok()?))).flatten()
        })
        .collect()
}

pub fn content_view(window: &NSWindow) -> Retained<NSView> {
    window.contentView().expect("the window has a content view")
}

/// Puts `view` above every other view in the window. A view created later lands on top, so the
/// chrome has to be raised again after each new page view.
pub fn raise(window: &NSWindow, view: &wry::WebView) {
    let wk = wk(view);
    content_view(window).addSubview_positioned_relativeTo(&wk, NSWindowOrderingMode::Above, None);
}

/// Converts a point measured from the window's top-left corner to AppKit's window coordinates.
fn window_point(window: &NSWindow, x: f64, y: f64) -> NSPoint {
    // Window coordinates start at the bottom-left, and the content view sits at the bottom.
    let height = content_view(window).frame().size.height;
    NSPoint::new(x, height - y)
}

/// Which of `candidates` AppKit would hand a click at this point to, by index.
pub fn hit(window: &NSWindow, x: f64, y: f64, candidates: &[&wry::WebView]) -> Option<usize> {
    let content = content_view(window);
    let in_window = window_point(window, x, y);
    let in_content = content.convertPoint_fromView(in_window, None);
    let in_parent = match unsafe { content.superview() } {
        Some(parent) => content.convertPoint_toView(in_content, Some(&parent)),
        None => in_content,
    };
    let target = content.hitTest(in_parent)?;
    candidates.iter().position(|candidate| target.isDescendantOf(&wk(candidate)))
}

/// Sends a real mouse press and release through the window, as a user's click would arrive.
pub fn click(window: &NSWindow, x: f64, y: f64) {
    let location = window_point(window, x, y);
    for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            location,
            NSEventModifierFlags::empty(),
            objc2_foundation::NSProcessInfo::processInfo().systemUptime(),
            window.windowNumber(),
            None,
            0,
            1,
            1.0,
        );
        if let Some(event) = event {
            window.sendEvent(&event);
        }
        pump(0.05);
    }
}

/// Saves a picture of the window as the screen shows it. Returns false when the system refuses
/// (screen recording not allowed).
pub fn screenshot(window: &NSWindow, path: &std::path::Path) -> bool {
    let id = window.windowNumber().to_string();
    let saved = |arguments: &[&str]| {
        let status = std::process::Command::new("screencapture").args(arguments).arg(path).status();
        matches!(status, Ok(status) if status.success()) && path.metadata().map(|m| m.len() > 0).unwrap_or(false)
    };
    // The window alone if the system allows it, the whole screen otherwise.
    saved(&["-x", "-o", "-l", &id]) || saved(&["-x"])
}
