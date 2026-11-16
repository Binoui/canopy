//! The prototype's window: a transparent HTML chrome on top, three page slots underneath.
//! Same shape as Canopy on Linux, so what works here tells us what the Mac backend can rely on.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use serde_json::{json, Value};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use wry::dpi::{LogicalPosition, LogicalSize as WrySize};
use wry::{PageLoadEvent, Rect, WebViewBuilder, WebViewExtMacOS};

use crate::native::{self, Checkpoint};
use crate::server::Site;

/// Height of the chrome when only the strip shows.
pub const STRIP: f64 = 44.0;
const SLOTS: usize = 3;

#[derive(Default)]
pub struct PageLog {
    pub started: Cell<u32>,
    pub finished: Cell<u32>,
    pub title: RefCell<String>,
}

pub struct Page {
    pub view: wry::WebView,
    pub log: Rc<PageLog>,
}

pub struct App {
    pub window: Window,
    pub ns_window: Retained<NSWindow>,
    pub chrome: wry::WebView,
    pub pages: Vec<Option<Page>>,
    pub saved: Vec<Option<Checkpoint>>,
    pub selected: usize,
    pub expanded: bool,
    pub note: String,
    pub site: Site,
    commands: Rc<RefCell<VecDeque<String>>>,
    proxy: EventLoopProxy<()>,
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: WrySize::new(width.max(1.0), height.max(1.0)).into(),
    }
}

/// Runs a script in a view and waits for its value. Null when the page gave no answer.
pub fn eval(view: &wry::WebView, script: &str) -> Value {
    let answer = Arc::new(Mutex::new(None));
    let sink = answer.clone();
    if view
        .evaluate_script_with_callback(script, move |text| *sink.lock().unwrap() = Some(text))
        .is_err()
    {
        return Value::Null;
    }
    native::wait(Duration::from_secs(3), || answer.lock().unwrap().is_some());
    let text = answer.lock().unwrap().take().unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

impl App {
    pub fn size(&self) -> (f64, f64) {
        let size = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        (size.width, size.height)
    }

    pub fn layout(&self) {
        let (width, height) = self.size();
        for page in self.pages.iter().flatten() {
            let _ = page.view.set_bounds(rect(0.0, STRIP, width, height - STRIP));
        }
        let chrome_height = if self.expanded { height } else { STRIP };
        let _ = self.chrome.set_bounds(rect(0.0, 0.0, width, chrome_height));
    }

    pub fn page(&self, slot: usize) -> &Page {
        self.pages[slot].as_ref().expect("the slot holds a live page")
    }

    /// Builds a page view in a slot. Without an address it stays empty, ready for a restore.
    pub fn open(&mut self, slot: usize, url: Option<&str>, visible: bool) {
        let log = Rc::new(PageLog::default());
        let (width, height) = self.size();
        let mut builder = WebViewBuilder::new()
            .with_bounds(rect(0.0, STRIP, width, height - STRIP))
            .with_visible(visible)
            .with_autoplay(true)
            .with_accept_first_mouse(true)
            .with_on_page_load_handler({
                let log = log.clone();
                let proxy = self.proxy.clone();
                move |event, _url| {
                    match event {
                        PageLoadEvent::Started => log.started.set(log.started.get() + 1),
                        PageLoadEvent::Finished => log.finished.set(log.finished.get() + 1),
                    }
                    let _ = proxy.send_event(());
                }
            })
            .with_document_title_changed_handler({
                let log = log.clone();
                let proxy = self.proxy.clone();
                move |title| {
                    *log.title.borrow_mut() = title;
                    let _ = proxy.send_event(());
                }
            });
        if let Some(url) = url {
            builder = builder.with_url(url);
        }
        let view = builder.build_as_child(&self.window).expect("the page view is built");
        self.pages[slot] = Some(Page { view, log });
        // A new view lands on top of everything, the chrome included.
        native::raise(&self.ns_window, &self.chrome);
    }

    /// Shows one slot's page and hides the others.
    pub fn select(&mut self, slot: usize) {
        self.selected = slot;
        for (index, page) in self.pages.iter().enumerate() {
            if let Some(page) = page {
                let _ = page.view.set_visible(index == slot);
            }
        }
    }

    /// Saves a page's history and destroys its view. Returns the page process it had.
    pub fn drop_page(&mut self, slot: usize, close: bool) -> Option<i32> {
        let page = self.pages[slot].take()?;
        self.saved[slot] = native::checkpoint(&page.view);
        let process = native::page_process(&page.view);
        if close {
            native::close(&page.view);
        }
        drop(page);
        process
    }

    /// Rebuilds a dropped page from its saved history.
    pub fn revive(&mut self, slot: usize, visible: bool) -> bool {
        let Some(saved) = self.saved[slot].take() else { return false };
        self.open(slot, None, visible);
        native::restore(&self.page(slot).view, &saved);
        true
    }

    pub fn memory_note(&self) -> String {
        let mut lines = Vec::new();
        for (slot, page) in self.pages.iter().enumerate() {
            let name = ["A", "B", "C"][slot];
            match page {
                Some(page) => {
                    let process = native::page_process(&page.view);
                    let memory = process.and_then(native::footprint_mib);
                    lines.push(match (process, memory) {
                        (Some(pid), Some(mib)) => format!("{name}: {mib:.0} MiB (process {pid})"),
                        _ => format!("{name}: live, memory not readable"),
                    });
                }
                None => lines.push(format!("{name}: no live page")),
            }
        }
        let own = native::footprint_mib(std::process::id() as i32).unwrap_or(0.0);
        lines.push(format!("Canopy prototype itself: {own:.0} MiB"));
        for (name, pid) in native::helper_processes() {
            if let Some(mib) = native::footprint_mib(pid) {
                lines.push(format!("{name}: {mib:.0} MiB (shared by every WebKit app)"));
            }
        }
        lines.join("\n")
    }

    /// Sends the current state to the chrome so it can repaint.
    pub fn publish(&self) {
        let pages: Vec<Value> = (0..SLOTS)
            .map(|slot| match &self.pages[slot] {
                Some(page) => json!({
                    "state": "live",
                    "title": page.log.title.borrow().clone(),
                    "url": page.view.url().unwrap_or_default(),
                }),
                None if self.saved[slot].is_some() => json!({ "state": "saved" }),
                None => json!({ "state": "empty" }),
            })
            .collect();
        let state = json!({
            "selected": self.selected,
            "expanded": self.expanded,
            "pages": pages,
            "note": self.note,
        });
        let _ = self.chrome.evaluate_script(&format!("update({state})"));
    }

    fn handle(&mut self, command: &str) {
        let slot = self.selected;
        match command {
            "back" => {
                if let Some(page) = &self.pages[slot] {
                    native::go_back(&page.view);
                }
            }
            "checkpoint" => {
                if self.pages[slot].is_some() {
                    self.drop_page(slot, true);
                    self.publish();
                    // Leave the empty slot on screen for a moment so the rebuild is visible.
                    native::wait(Duration::from_millis(700), || false);
                    self.revive(slot, true);
                }
            }
            "drop" => {
                self.drop_page(slot, true);
            }
            "video" => {
                let url = self.site.url("/video");
                self.load(slot, &url);
            }
            "memory" => {
                self.note = self.memory_note();
                self.expanded = true;
            }
            "palette:open" => {
                self.note.clear();
                self.expanded = true;
            }
            "palette:close" => self.expanded = false,
            _ => {
                if let Some(target) = command.strip_prefix("tab:").and_then(|n| n.parse::<usize>().ok()) {
                    if self.pages[target].is_none() && !self.revive(target, true) {
                        self.open(target, Some("https://www.youtube.com/"), true);
                    }
                    self.select(target);
                } else if let Some(address) = command.strip_prefix("go:") {
                    let url = if address.contains("://") { address.to_string() } else { format!("https://{address}") };
                    self.load(slot, &url);
                }
            }
        }
        self.layout();
        self.publish();
    }

    fn load(&mut self, slot: usize, url: &str) {
        match &self.pages[slot] {
            Some(page) => {
                let _ = page.view.load_url(url);
            }
            None => self.open(slot, Some(url), true),
        }
    }
}

pub fn run() {
    let mut arguments = std::env::args().skip(1);
    let mut self_test = None;
    while let Some(argument) = arguments.next() {
        if argument == "--self-test" {
            self_test = Some(std::path::PathBuf::from(arguments.next().unwrap_or_else(|| "out".into())));
        }
    }

    let site = Site::start().expect("the local test site starts");
    let event_loop = EventLoopBuilder::<()>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Canopy · Mac prototype")
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .build(&event_loop)
        .expect("the window opens");

    let commands = Rc::new(RefCell::new(VecDeque::new()));
    let chrome = WebViewBuilder::new()
        .with_transparent(true)
        .with_html(include_str!("../assets/chrome.html"))
        .with_bounds(rect(0.0, 0.0, 1200.0, STRIP))
        .with_accept_first_mouse(true)
        .with_ipc_handler({
            let commands = commands.clone();
            let proxy = proxy.clone();
            move |request| {
                commands.borrow_mut().push_back(request.body().clone());
                let _ = proxy.send_event(());
            }
        })
        .build_as_child(&window)
        .expect("the chrome view is built");
    let ns_window = chrome.ns_window();

    let mut app = App {
        window,
        ns_window,
        chrome,
        pages: (0..SLOTS).map(|_| None).collect(),
        saved: (0..SLOTS).map(|_| None).collect(),
        selected: 0,
        expanded: false,
        note: String::new(),
        site,
        commands,
        proxy,
    };

    if self_test.is_none() {
        // A is the local test page. B loads a real site in the background, never shown until
        // its tab is clicked. C stays empty until clicked.
        let first = app.site.url("/p/1");
        app.open(0, Some(&first), true);
        app.open(1, Some("https://en.wikipedia.org/wiki/Tree_(graph_theory)"), false);
    }
    app.layout();

    let started = Instant::now();
    let mut pending_test = self_test;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *control_flow = ControlFlow::Exit,
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => app.layout(),
            Event::MainEventsCleared => {
                if pending_test.is_some() && started.elapsed() > Duration::from_millis(1500) {
                    let out = pending_test.take().unwrap();
                    let failures = crate::selftest::run(&mut app, &out);
                    std::process::exit(if failures == 0 { 0 } else { 1 });
                }
                let queued: Vec<String> = app.commands.borrow_mut().drain(..).collect();
                for command in &queued {
                    app.handle(command);
                }
                if pending_test.is_none() {
                    app.publish();
                }
            }
            _ => {}
        }
    });
}
