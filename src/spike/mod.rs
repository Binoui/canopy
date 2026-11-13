//! Spike: the tree strip drawn in HTML over the page, to test a cross-platform chrome.
//!
//! The window comes from tao and both web views from wry, the stack Tauri uses. Rust keeps the
//! tree and the layout (`chrome::graph::strip_layout`); the strip view only draws what it is sent
//! and reports hovers, clicks and typed addresses back over IPC.
//!
//! On Linux the views sit in a `gtk::Fixed`, which wry needs to stack web views on Wayland.
//! On macOS and Windows they would be child views of the window, with no GTK at all.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Window, WindowBuilder},
};
use url::Url;
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder,
};

use crate::{
    address::{destination, Destination},
    browser::{Browser, NavigationTarget, Visit, VisitId, VisitState},
    chrome::graph::{self, EdgeKind, NodeKind, OPEN_HEIGHT, REST_HEIGHT},
    forest::ForestProjection,
};

const STRIP_HTML: &str = include_str!("strip.html");
const HOME_HTML: &str = "<!doctype html><body style=\"font:15px system-ui;color:#787774;\
display:grid;place-items:center;height:90vh\">Type an address in the strip and press Enter.</body>";
/// Room under the open strip for its shadow and rounded corners.
const SHADOW: f64 = 28.;

enum Message {
    Strip(String),
    Load { finished: bool, url: String },
    Title(String),
}

struct Spike {
    browser: Browser,
    window: Window,
    page: WebView,
    strip: WebView,
    open: bool,
    /// The URL the page reported when its current load started.
    started: Option<Url>,
}

pub fn run(data_dir: &Path, start: Option<&str>) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let browser = Browser::open(&data_dir.join("forest.sqlite3"))?;
    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Canopy (HTML strip spike)")
        .with_inner_size(tao::dpi::LogicalSize::new(1240., 820.))
        .build(&event_loop)?;

    #[cfg(target_os = "linux")]
    let host = {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        let fixed = gtk::Fixed::new();
        // wry sizes its views with size requests; without this wrapper they would set the
        // window's minimum size, so a tiling compositor could never shrink it below them.
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::External)
            .build();
        scroller.add(&fixed);
        let vbox = window
            .default_vbox()
            .context("The window has no GTK box.")?;
        vbox.pack_start(&scroller, true, true, 0);
        scroller.show_all();
        fixed
    };
    let build = |builder: WebViewBuilder| -> wry::Result<WebView> {
        #[cfg(target_os = "linux")]
        {
            use wry::WebViewBuilderExtUnix;
            builder.build_gtk(&host)
        }
        #[cfg(not(target_os = "linux"))]
        builder.build_as_child(&window)
    };

    let (width, height) = logical_size(&window);
    let mut context = WebContext::new(Some(data_dir.join("profile")));
    let proxy = event_loop.create_proxy();
    // Built first so the strip, built second, stacks on top of it.
    let page = build(
        WebViewBuilder::new_with_web_context(&mut context)
            .with_bounds(rect(0., REST_HEIGHT, width, height - REST_HEIGHT))
            .with_html(HOME_HTML)
            .with_on_page_load_handler({
                let proxy = proxy.clone();
                move |event, url| {
                    let finished = matches!(event, PageLoadEvent::Finished);
                    let _ = proxy.send_event(Message::Load { finished, url });
                }
            })
            .with_document_title_changed_handler(send(&proxy, Message::Title)),
    )?;
    let strip = build(
        WebViewBuilder::new()
            .with_bounds(rect(0., 0., width, REST_HEIGHT))
            .with_transparent(true)
            .with_html(STRIP_HTML)
            .with_ipc_handler(send(&proxy, |request: wry::http::Request<String>| {
                Message::Strip(request.into_body())
            })),
    )?;

    let mut spike = Spike {
        browser,
        window,
        page,
        strip,
        open: false,
        started: None,
    };
    if let Some(start) = start {
        spike.go(start, false)?;
    } else if let Some(current) = spike.browser.snapshot()?.current_visit().map(|v| v.id) {
        let url = spike.browser.restore(current)?;
        spike.page.load_url(url.as_str())?;
    }

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let result = match event {
            Event::UserEvent(message) => spike.handle(message),
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => spike.resize(),
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *flow = ControlFlow::Exit;
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            eprintln!("Canopy spike: {error:#}");
            let _ = spike.toast(&format!("{error:#}"));
        }
    });
}

impl Spike {
    fn handle(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Strip(body) => {
                let message: Value = serde_json::from_str(&body)?;
                match message["t"].as_str().unwrap_or_default() {
                    "ready" => {
                        // Test hook: rest and lift a simulated pointer, for runs without a mouse.
                        if let Some(at) = std::env::var_os("CANOPY_SPIKE_HOVER") {
                            let at: u32 = at.to_str().and_then(|s| s.parse().ok()).unwrap_or(3000);
                            self.strip.evaluate_script(&format!(
                                "setTimeout(() => hover(true), {at}); setTimeout(() => hover(false), {})",
                                at + 3000
                            ))?;
                        }
                        self.render()
                    }
                    "open" => {
                        self.open = true;
                        let (width, _) = logical_size(&self.window);
                        self.strip
                            .set_bounds(rect(0., 0., width, OPEN_HEIGHT + SHADOW))?;
                        self.render()
                    }
                    "close" => {
                        self.open = false;
                        self.render()
                    }
                    // Sent once the strip finishes shrinking, so its view never clips the motion.
                    "collapsed" if !self.open => self.resize(),
                    "go" => {
                        let id: VisitId = message["id"]
                            .as_str()
                            .context("A strip click lacks its page.")?
                            .parse()?;
                        let url = self.browser.restore(id)?;
                        self.page.load_url(url.as_str())?;
                        Ok(())
                    }
                    "address" => self.go(
                        message["text"].as_str().unwrap_or_default(),
                        message["root"].as_bool().unwrap_or(false),
                    ),
                    _ => Ok(()),
                }
            }
            Message::Load { finished, url } => {
                let Ok(url) = Url::parse(&url) else {
                    return Ok(());
                };
                if !matches!(url.scheme(), "http" | "https") {
                    return Ok(());
                }
                if !finished {
                    self.browser.started(url.clone())?;
                    self.started = Some(url);
                    return Ok(());
                }
                if self.started.take().is_some_and(|started| started != url) {
                    self.browser.redirected(url.clone());
                }
                if self.browser.committed(url)? {
                    self.render()?;
                }
                Ok(())
            }
            Message::Title(title) => {
                if let Some(url) = self.page.url().ok().and_then(|url| Url::parse(&url).ok()) {
                    self.browser.title_changed(&url, &title)?;
                    self.render()?;
                }
                Ok(())
            }
        }
    }

    fn go(&mut self, text: &str, root: bool) -> Result<()> {
        let url = match destination(text)? {
            Destination::Address(url) | Destination::Search(url) => url,
        };
        let current = self
            .browser
            .snapshot()?
            .current_visit()
            .map(|visit| visit.id);
        let target = match current {
            Some(parent) if !root => NavigationTarget::ChildOf(parent),
            _ => NavigationTarget::Root,
        };
        let url = self.browser.navigate(url, target)?;
        self.page.load_url(url.as_str())?;
        Ok(())
    }

    fn resize(&mut self) -> Result<()> {
        let (width, height) = logical_size(&self.window);
        self.page
            .set_bounds(rect(0., REST_HEIGHT, width, height - REST_HEIGHT))?;
        let strip_height = if self.open {
            OPEN_HEIGHT + SHADOW
        } else {
            REST_HEIGHT
        };
        self.strip.set_bounds(rect(0., 0., width, strip_height))?;
        self.render()
    }

    fn render(&self) -> Result<()> {
        let (width, _) = logical_size(&self.window);
        let layout = layout_json(&self.browser, width, self.open)?;
        self.strip.evaluate_script(&format!("render({layout})"))?;
        Ok(())
    }

    fn toast(&self, text: &str) -> Result<()> {
        self.strip
            .evaluate_script(&format!("toast({})", Value::from(text)))?;
        Ok(())
    }
}

/// The strip as the HTML view draws it: Rust places every node; the view only paints.
fn layout_json(browser: &Browser, width: f64, open: bool) -> Result<Value> {
    let forest = ForestProjection::from_snapshot(browser.snapshot()?, browser.tree_states()?)?;
    let height = if open { OPEN_HEIGHT } else { REST_HEIGHT };
    let Some(current) = forest.current() else {
        return Ok(json!({ "open": open, "height": height, "pan": 0, "nodes": [], "edges": [] }));
    };
    let visit = forest
        .visit(current)
        .context("The current page is missing.")?;
    let layout = graph::strip_layout(&forest, current, width, open, pill_width(visit));
    let nodes: Vec<Value> = layout
        .nodes
        .iter()
        .filter_map(|node| {
            let visit = forest.visit(node.id)?;
            let host = visit.url.host_str().unwrap_or_default();
            Some(json!({
                "id": node.id.to_string(),
                "x": node.rect.x, "y": node.rect.y, "w": node.rect.w, "s": node.rect.s,
                "kind": match node.kind {
                    NodeKind::Pill => "pill",
                    NodeKind::Tile => "tile",
                    NodeKind::Raw => "raw",
                    NodeKind::Bud => "bud",
                },
                "path": node.on_path,
                "pending": visit.state == VisitState::Queued,
                "label": visit.label(),
                "site": host.strip_prefix("www.").unwrap_or(host),
                "hidden": node.hidden.len(),
            }))
        })
        .collect();
    let edges: Vec<Value> = layout
        .edges
        .iter()
        .filter_map(|edge| {
            let (a, b) = (layout.node(edge.from)?, layout.node(edge.to)?);
            Some(json!({
                "id": format!("{}-{}", edge.from, edge.to),
                "kind": match edge.kind {
                    EdgeKind::Path => "path",
                    EdgeKind::Pending => "pending",
                    EdgeKind::Plain => "plain",
                },
                "c": graph::edge_curve(edge, a.rect, b.rect),
            }))
        })
        .collect();
    Ok(json!({
        "open": open,
        "height": layout.height,
        "pan": graph::pan(&layout, width),
        "nodes": nodes,
        "edges": edges,
    }))
}

/// Pango measures the pill in the GTK chrome; an estimate is enough for the spike.
fn pill_width(visit: &Visit) -> f64 {
    let chars = visit.label().chars().count().max(16) as f64;
    (chars * 6.8 + 56.).clamp(140., 300.)
}

fn logical_size(window: &Window) -> (f64, f64) {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    (size.width, size.height)
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width, height.max(1.)).into(),
    }
}

/// A web view callback that forwards to the event loop, where the spike's state lives.
fn send<A: 'static>(
    proxy: &EventLoopProxy<Message>,
    make: impl Fn(A) -> Message + 'static,
) -> impl Fn(A) + 'static {
    let proxy = proxy.clone();
    move |a| {
        let _ = proxy.send_event(make(a));
    }
}
