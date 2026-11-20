#[path = "common/http.rs"]
mod http;
use anyhow::Result;
use canopy::{
    browser::{Browser, Navigation, NavigationTarget, SourceContext, Visibility},
    chrome::{ChromeHost, ChromeIntent},
    pages::{PageEvent, Pages},
    ui::Ui,
};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::Write,
    net::TcpListener,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use webkit2gtk::WebViewExt;
mod common;
use common::*;

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let hits = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
    let requests = hits.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let requests = requests.clone();
            thread::spawn(move || {
                let Ok(mut stream) = stream else { return };
                let Ok(bytes) = http::request(&mut stream) else {
                    return;
                };
                let request = String::from_utf8_lossy(&bytes);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                *requests.lock().unwrap().entry(path.clone()).or_default() += 1;
                if path.ends_with("/meta") {
                    thread::sleep(Duration::from_millis(300));
                }
                if path.contains("slow") {
                    thread::sleep(Duration::from_millis(800));
                }
                if path.contains("fail") {
                    return;
                }
                let body = if path.ends_with("/meta") {
                    "<!doctype html><meta http-equiv=refresh content=\"0;url=landing\">".to_owned()
                } else {
                    format!("<!doctype html><title>{path}</title><p>Continuation fixture</p>")
                };
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            });
        }
    });
    let cases = [
        "branch_own",
        "branch_back_root",
        "branch_reload_empty",
        "path_meta_refresh",
        "branch_replaced",
        "path_own",
        "path_failed",
        "path_replaced",
        "settle_pending_branch",
        "settle_pending_path",
    ];
    for case in cases {
        if std::env::var("CONTINUATION_CASE").is_ok_and(|only| only != case) {
            continue;
        }
        let directory = tempfile::tempdir()?;
        let url = |path: &str| format!("http://{address}/{case}/{path}").parse().unwrap();
        let mut browser = Browser::open(&directory.path().join("forest.sqlite3"))?;
        let mut navigation = Navigation::default();
        let base_url = browser.navigate(&mut navigation, url("base"), NavigationTarget::Root)?;
        let ticket = navigation.pending_ticket().unwrap();
        navigation.started(ticket);
        browser.commit_load(
            &mut navigation,
            ticket,
            base_url.clone(),
            Visibility::Selected,
        )?;
        let base = browser.snapshot()?.current.unwrap();
        let root = browser.queue_link(
            base,
            url("slow"),
            SourceContext {
                page_url: base_url,
                citation_url: None,
                link_text: String::new(),
                excerpt: String::new(),
            },
        )?;
        let tree = browser.tree_states()?[0].root;
        drop(browser);
        let root = if case == "branch_reload_empty" {
            std::fs::remove_file(directory.path().join("forest.sqlite3"))?;
            let mut browser = Browser::open(&directory.path().join("forest.sqlite3"))?;
            browser.add_roots(&[url("slow")])?[0]
        } else {
            root
        };
        let ui = Ui::new(directory.path())?;
        ui.window.show_all();
        ready(&ui)?;
        let first = if case == "path_meta_refresh" {
            "meta"
        } else if case == "path_failed" {
            "fail"
        } else {
            "slow"
        };
        if case.starts_with("branch") || case == "settle_pending_branch" {
            ui.dispatch(ChromeIntent::BranchFromQueued(root));
        } else {
            ui.dispatch(ChromeIntent::OpenPath {
                first: url(first),
                second: url("second"),
            });
        }
        let request_path = format!("/{case}/{first}");
        wait("first request issued", || {
            hits.lock().unwrap().contains_key(&request_path)
        })?;
        if case == "path_meta_refresh" {
            let deadline = Instant::now() + Duration::from_secs(2);
            // WebKit sources run before HIGH_IDLE; continuation idles run after it.
            glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
                if Instant::now() < deadline {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }
        if case.ends_with("replaced") {
            ui.pages
                .selected_page()
                .native
                .load_uri(url("replacement").as_str());
        }
        if case == "branch_back_root" {
            key("alt+Left")?;
        }
        if case == "branch_reload_empty" {
            key("F5")?;
        }

        if case.starts_with("settle_pending") {
            ui.dispatch(ChromeIntent::PlaceTree {
                tree,
                to: canopy::browser::Move::History,
            });
        }
        if case.starts_with("branch") && !case.ends_with("replaced") {
            wait("own attempt opens branch palette", || {
                visible(&ui, "#palette")
            })?;
            assert_eq!(ui.snapshot()?.current, Some(root));
        } else if case == "path_own" || case == "path_meta_refresh" {
            wait("second path committed", || {
                ui.snapshot().is_ok_and(|s| {
                    s.current
                        .and_then(|id| {
                            s.visits
                                .iter()
                                .find(|v| v.id == id)
                                .map(|v| v.url.path().ends_with("/second"))
                        })
                        .unwrap_or(false)
                })
            })?;
            let snapshot = ui.snapshot()?;
            let second = snapshot
                .visits
                .iter()
                .find(|v| v.url.path().ends_with("/second"))
                .unwrap();
            let parent = snapshot
                .visits
                .iter()
                .find(|v| Some(v.id) == second.parent)
                .unwrap();
            assert!(
                parent.url.path().ends_with(if case == "path_meta_refresh" {
                    "/meta"
                } else {
                    "/slow"
                }),
                "second attaches to its own committed visit"
            );
            if case == "path_meta_refresh" {
                assert!(
                    hits.lock()
                        .unwrap()
                        .contains_key(&format!("/{case}/landing")),
                    "refresh starts before the continuation idle"
                );
            }
        }
        pump(1600);
        if !case.starts_with("branch") || case.ends_with("replaced") {
            assert!(
                !visible(&ui, "#palette"),
                "{case} must not open the branch palette"
            );
        }
        if case != "path_own" && case != "path_meta_refresh" {
            assert_eq!(
                hits.lock()
                    .unwrap()
                    .get(&format!("/{case}/second"))
                    .copied()
                    .unwrap_or(0),
                0,
                "{case} must not issue the second URL"
            );
        }
        if case.starts_with("settle_pending") {
            assert!(
                ui.snapshot()?.current.is_none(),
                "settling the last tree must cancel its pending continuation and load"
            );
        }
        if case == "path_replaced" {
            assert!(
                ui.pages
                    .selected_page()
                    .native
                    .uri()
                    .is_some_and(|uri| uri.ends_with("/replacement")),
                "replacement stays on screen"
            );
        }
        println!("PASS {case}");
        ui.window.close();
        pump(100);
    }
    for case in ["remount_reentrant_destroy", "released_page_idle"] {
        if std::env::var("CONTINUATION_CASE").is_ok_and(|only| only != case) {
            continue;
        }
        let directory = tempfile::tempdir()?;
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        let fixed = gtk::Fixed::new();
        window.add(&fixed);
        let browser = Browser::open(&directory.path().join("forest.sqlite3"))?;
        let pages = Pages::mount(&fixed, browser, &directory.path().join("profile"))?;
        window.show_all();
        let observed = Rc::new(Cell::new(false));
        let seen = observed.clone();
        let weak = Rc::downgrade(&pages);
        if case == "remount_reentrant_destroy" {
            pages.selected_page().native.connect_destroy(move |_| {
                let pages = weak.upgrade().unwrap();
                let _replacement = pages.selected_page();
                seen.set(true);
            });
            pages.mount_selected_page(&fixed)?;
            assert!(observed.get(), "destroy can reenter the page owner");
        } else {
            pages.observe(move |event| {
                assert!(
                    !matches!(event, PageEvent::OpenBranchPalette { .. }),
                    "released page must not run its continuation"
                );
                if matches!(event, PageEvent::Loading(false)) {
                    if let Some(pages) = weak.upgrade() {
                        pages.release_selected_page();
                        seen.set(true);
                    }
                }
            });
            pages.open_path(
                format!("http://{address}/released/first").parse()?,
                format!("http://{address}/released/second").parse()?,
            )?;
            wait("ready page released before idle", || observed.get())?;
            pump(200);
            assert!(!hits.lock().unwrap().contains_key("/released/second"));
        }
        window.close();
        pump(100);
        println!("PASS {case}");
    }
    if std::env::var("CONTINUATION_CASE")
        .map_or(true, |case| case == "owner_survives_reentrant_observer")
    {
        let directory = tempfile::tempdir()?;
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        let fixed = gtk::Fixed::new();
        window.add(&fixed);
        let browser = Browser::open(&directory.path().join("forest.sqlite3"))?;
        let pages = Pages::mount(&fixed, browser, &directory.path().join("profile"))?;
        let native = pages.selected_page().native.downgrade();
        let holder = Rc::new(RefCell::new(Some(pages.clone())));
        let weak = Rc::downgrade(&holder);
        pages.observe(move |event| {
            if matches!(event, PageEvent::Loading(false)) {
                if let Some(holder) = weak.upgrade() {
                    holder.borrow_mut().take();
                }
            }
        });
        window.show_all();
        pages.go(canopy::pages::Go::Address {
            url: format!("http://{address}/owner/page").parse()?,
            target: NavigationTarget::Root,
        })?;
        drop(pages);
        wait("observer released owner and page", || {
            holder.borrow().is_none() && native.upgrade().is_none()
        })?;
        window.close();
        pump(100);
        println!("PASS owner_survives_reentrant_observer");
    }
    Ok(())
}
