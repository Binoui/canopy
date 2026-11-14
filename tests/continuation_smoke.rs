use anyhow::Result;
use canopy::{
    browser::{Browser, Navigation, NavigationTarget, SourceContext, UsualPath, Visibility},
    chrome::{ChromeHost, ChromeIntent},
    pages::{PageEvent, Pages},
    ui::Ui,
};
use gtk::prelude::*;
use std::{
    cell::RefCell,
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use webkit2gtk::WebViewExt;
mod common;
use common::*;

fn main() -> Result<()> {
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
                let mut bytes = [0; 8192];
                let Ok(n) = stream.read(&mut bytes) else {
                    return;
                };
                let request = String::from_utf8_lossy(&bytes[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                *requests.lock().unwrap().entry(path.clone()).or_default() += 1;
                if path.contains("slow") {
                    thread::sleep(Duration::from_millis(800));
                }
                if path.contains("fail") {
                    return;
                }
                let body =
                    format!("<!doctype html><title>{path}</title><p>Continuation fixture</p>");
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            });
        }
    });
    let cases = [
        "branch_own",
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
        let ui = Ui::new(directory.path())?;
        ui.window.show_all();
        ready(&ui)?;
        let first = if case == "path_failed" {
            "fail"
        } else {
            "slow"
        };
        if case.starts_with("branch") || case == "settle_pending_branch" {
            ui.dispatch(ChromeIntent::BranchFromQueued(root));
        } else {
            ui.dispatch(ChromeIntent::OpenPath(UsualPath {
                first: url(first),
                second: url("second"),
                visits: 1,
            }));
        }
        let request_path = format!("/{case}/{first}");
        wait("first request issued", || {
            hits.lock().unwrap().contains_key(&request_path)
        })?;
        if case.ends_with("replaced") {
            // This bypasses chrome dispatch, as a native page navigation can.
            if case == "branch_replaced" {
                ui.pages.selected_page().restore(root)?;
            } else {
                ui.pages
                    .selected_page()
                    .navigate(url("replacement"), NavigationTarget::Root)?;
            }
        }
        if case.starts_with("settle_pending") {
            ui.dispatch(ChromeIntent::SettleTree(tree));
        }
        if case == "branch_own" {
            wait("own attempt opens branch palette", || {
                visible(&ui, "#palette")
            })?;
            assert_eq!(ui.snapshot()?.current, Some(root));
        } else if case == "path_own" {
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
            assert!(parent.url.path().ends_with("/slow"));
        }
        pump(1600);
        if case != "branch_own" {
            assert!(
                !visible(&ui, "#palette"),
                "{case} must not open the branch palette"
            );
        }
        if case != "path_own" {
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
