mod common;
#[path = "common/http.rs"]
mod http;
use anyhow::Result;
use canopy::pages::Go;
use gtk::prelude::*;
use std::{
    collections::BTreeMap,
    io::Write,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};
use webkit2gtk::WebViewExt;

struct Fixture {
    ui: std::rc::Rc<canopy::ui::Ui>,
    origin: String,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
    _directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let ui = canopy::ui::Ui::new(directory.path())?;
        ui.window.show_all();
        common::ready(&ui)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let icon = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 16, 16)
            .unwrap();
        icon.fill(0x2050ffff);
        let favicon = icon.save_to_bufferv("png", &[])?;
        let counts = hits.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let counts = counts.clone();
                let favicon = favicon.clone();
                thread::spawn(move || {
                    let Ok(request) = http::request(&mut stream) else {
                        return;
                    };
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or("/");
                    *counts.lock().unwrap().entry(path.to_owned()).or_default() += 1;
                    if path == "/favicon.ico" {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", favicon.len());
                        let _ = stream.write_all(&favicon);
                        return;
                    }
                    let body = format!("<!doctype html><title>{path}</title><body style='background:#edf5ed;color:#234;font:24px sans-serif;padding:60px'><h1>{path}</h1><p>A local Canopy garden fixture.</p><a id=link href='/waiting' style='display:block;height:100px'>Open another garden</a>");
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        Ok(Self {
            ui,
            origin,
            hits,
            _directory: directory,
        })
    }
    fn root(&self, path: &str) -> Result<canopy::browser::VisitId> {
        self.ui.pages.open_roots(&[self.url(path)])?;
        common::wait("root visible", || {
            self.ui
                .snapshot()
                .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == path))
                && !self.ui.pages.selected_page().native.is_loading()
        })?;
        common::pump(200);
        Ok(self.ui.snapshot()?.current.unwrap())
    }
    fn url(&self, path: &str) -> url::Url {
        format!("{}{path}", self.origin).parse().unwrap()
    }
    fn select(&self, visit: canopy::browser::VisitId) -> Result<()> {
        self.ui.pages.go(Go::Visit(visit))?;
        common::wait("visit selected", || {
            self.ui.snapshot().is_ok_and(|s| s.current == Some(visit))
                && !self.ui.pages.selected_page().native.is_loading()
        })?;
        common::pump(200);
        Ok(())
    }
    fn hits(&self, path: &str) -> usize {
        *self.hits.lock().unwrap().get(path).unwrap_or(&0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.ui.window.close();
        common::pump(100);
    }
}

impl Fixture {
    fn queue(&self, path: &str) -> Result<canopy::browser::VisitId> {
        let count = self.ui.snapshot()?.visits.len();
        common::eval(
            &self.ui,
            &format!(
                "link.href={};JSON.stringify(true)",
                serde_json::to_string(path)?
            ),
        )?;
        let (x, y) = common::element_point(&self.ui, "link")?;
        common::xdo(&["mousemove", &x.to_string(), &y.to_string(), "click", "2"])?;
        common::wait("middle click queues visit", || {
            self.ui
                .snapshot()
                .is_ok_and(|s| s.visits.len() == count + 1)
        })?;
        Ok(self.ui.snapshot()?.visits.last().unwrap().id)
    }
    fn committed(&self, visit: canopy::browser::VisitId) -> Result<()> {
        common::wait("background committed", || {
            self.ui.pages.loaded_visits().contains(&visit)
                && self
                    .ui
                    .pages
                    .background_view(visit)
                    .is_some_and(|v| !v.is_loading())
        })
    }
}
impl Fixture {
    fn child(
        &self,
        parent: canopy::browser::VisitId,
        path: &str,
    ) -> Result<canopy::browser::VisitId> {
        self.ui.pages.go(Go::Address {
            url: self.url(path),
            target: canopy::browser::NavigationTarget::ChildOf(parent),
        })?;
        common::wait("child loaded", || {
            self.ui
                .snapshot()
                .is_ok_and(|s| s.current_visit().is_some_and(|v| v.url.path() == path))
                && !self.ui.pages.selected_page().native.is_loading()
        })?;
        common::pump(250);
        Ok(self.ui.snapshot()?.current.unwrap())
    }
    fn menu(&self, selector: &str) -> Result<()> {
        common::hover_chrome(&self.ui, selector)?;
        common::xdo(&["click", "3"])?;
        common::wait(&format!("node menu for {selector}"), || {
            common::visible(&self.ui, "[data-node-action='copy']")
        })
    }
    fn shot(&self, name: &str, theme: &str) -> Result<()> {
        if let Some(dir) = std::env::var_os("CANOPY_NODE_SHOTS") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir)?;
            common::pump(250);
            let file = dir.join(format!("{name}-{theme}.png"));
            anyhow::ensure!(
                std::process::Command::new("import")
                    .args(["-window", "root"])
                    .arg(file)
                    .status()?
                    .success(),
                "composed screenshot"
            );
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    common::start(directory.path())?;
    page_move_drag()?;
    page_move_picker()?;
    let f = Fixture::new()?;
    let a = f.root("/garden")?;
    let b = f.child(a, "/orchard")?;
    let c = f.child(b, "/apple")?;
    f.ui.pages.inject_recent_memory_sample(0);
    let peach = f.queue("/peach")?;
    f.committed(peach)?;
    f.select(peach)?;
    assert!(f.ui.pages.live_visits().contains(&c));
    let moved_view = f.ui.pages.selected_page().native.clone();
    f.ui.pages.move_branch(
        b,
        canopy::browser::MoveTo::OwnTree(canopy::browser::TabPlacement::End),
    )?;
    common::wait("moved tree tab selected", || {
        common::chrome(
            &f.ui,
            &format!(
                "!!document.querySelector('#tab-row .tab[aria-selected=true][data-id=\"{b}\"]')"
            ),
        )
        .is_ok_and(|v| v == true)
    })?;
    assert_eq!(f.ui.pages.selected_page().native, moved_view);
    common::key("alt+Left")?;
    common::wait("Back from moved background view", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(c))
    })?;
    common::key("alt+Left")?;
    common::wait("Back inside moved branch", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(b))
    })?;
    common::key("alt+Left")?;
    common::pump(250);
    assert_eq!(f.ui.snapshot()?.current, Some(b));
    common::eval(&f.ui, "history.back();JSON.stringify(true)")?;
    common::pump(300);
    assert_eq!(f.ui.snapshot()?.current, Some(b));
    common::key("alt+Right")?;
    common::wait("forward uses last opened child", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(c))
    })?;
    println!("PASS moved live branch retains its view and Back stays in the new tree; Alt+Right returns to the child.");
    common::key("alt+Right")?;
    common::wait("forward child of child", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(peach))
    })?;
    common::key("alt+Right")?;
    assert_eq!(f.ui.snapshot()?.current, Some(peach));
    f.select(a)?;
    let d = f.child(a, "/river")?;
    let e = f.child(d, "/brook")?;
    f.ui.pages.inject_recent_memory_sample(0);
    let waiting = f.queue("/unread")?;
    f.committed(waiting)?;
    f.select(a)?;
    let lake = f.child(a, "/lake")?;
    f.select(d)?;
    common::wait("branch selected", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(d))
    })?;
    common::key("alt+Up")?;
    common::pump(150);
    assert_eq!(f.ui.snapshot()?.current, Some(lake));
    common::key("alt+Up")?;
    assert_eq!(f.ui.snapshot()?.current, Some(lake));
    common::key("alt+Down")?;
    common::pump(150);
    assert_eq!(f.ui.snapshot()?.current, Some(d));
    f.select(e)?;
    f.ui.pages.settle_branch(d)?;
    common::wait("settle current branch lands on ancestor", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(a))
            && !f.ui.pages.selected_page().native.is_loading()
    })?;
    common::wait("settle releases live views", || {
        !f.ui.pages.live_visits().contains(&d)
            && !f.ui.pages.live_visits().contains(&e)
            && f.ui.pages.background_view(waiting).is_none()
    })?;
    common::wait("settle toast", || {
        common::text(&f.ui, "#toast").contains("1 waiting kept")
    })?;
    assert_eq!(common::chrome(&f.ui, "canopy.state().waiting.length")?, 0);
    common::click_chrome(&f.ui, "#toast button")?;
    common::wait("undo selects old page", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(e))
    })
    .map_err(|error| {
        anyhow::anyhow!(
            "{error}; snapshot {:?}; state {:?}",
            f.ui.snapshot(),
            common::chrome(
                &f.ui,
                "({state:canopy.state(),errors:S.errors,palette:S.palette})"
            )
        )
    })?;
    assert!(f.ui.pages.read(|b| b.visit(d))?.branch_settled_at.is_none());
    println!("PASS settling the selected branch lands on its ancestor, releases views, excludes waiting, and Undo returns.");
    for theme in ["dark", "light"] {
        common::chrome(&f.ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        common::wait("theme", || {
            common::chrome(&f.ui, "canopy.state().dark").is_ok_and(|v| v == (theme == "dark"))
        })?;
        f.menu(&common::node(waiting))?;
        assert!(common::text(&f.ui, ".node-menu-title").contains("unread"));
        assert_eq!(
            common::chrome(
                &f.ui,
                "document.querySelector('.node-menu-title').querySelectorAll('button').length"
            )?,
            0
        );
        f.shot("strip-row-menu", theme)?;
        common::click_chrome(&f.ui, "[data-node-action='choose-move-tree']")?;
        common::wait("attached-row tree choices", || {
            common::chrome(
                &f.ui,
                "S.popover?.kind === 'move-tree' && !!S.popover.data.choices",
            )
            .is_ok_and(|v| v == true)
        })?;
        common::pump(300);
        common::key("Escape")?;
        common::wait("picker restores attached-row focus", || {
            common::chrome(
                &f.ui,
                &format!("document.activeElement?.dataset.id === '{waiting}'"),
            )
            .is_ok_and(|v| v == true)
        })?;
        common::open_list(&f.ui)?;
        f.menu("#strip-list .bl-head")?;
        common::key("Escape")?;
        f.menu("#strip-nodes .pill")?;
        f.shot("opened-menu", theme)?;
        common::click_chrome(&f.ui, "[data-node-action='settle-branch']")?;
        common::wait("settled selected leaf", || {
            f.ui.snapshot().is_ok_and(|s| s.current == Some(d))
        })?;
        assert!(common::chrome(&f.ui, "document.getElementById('toast').getBoundingClientRect().top > document.getElementById('strip-clip').getBoundingClientRect().bottom")? == true);
        f.shot("settled-toast", theme)?;
        common::key("ctrl+shift+e")?;
        common::wait("tree sheet", || {
            common::visible(&f.ui, "#tree-list") || common::visible(&f.ui, "#tree-board")
        })?;
        common::click_chrome(&f.ui, "#tree-side [data-side='tree']")?;
        assert!(common::text(&f.ui, "#tree-nodes").contains("pages put away"));
        f.shot("settled-tree", theme)?;
        f.menu(&format!("#tree-nodes [data-id='{e}']"))?;
        assert!(!common::visible(
            &f.ui,
            "[data-node-action='settle-branch']"
        ));
        assert!(common::chrome(&f.ui, "document.getElementById('toast').getBoundingClientRect().top > document.querySelector('.sheet-head').getBoundingClientRect().bottom")? == true);
        f.shot("settled-tile-menu", theme)?;
        common::key("Escape")?;
        common::click_chrome(&f.ui, "#tree-side [data-side='list']")?;
        f.shot("settled-list", theme)?;
        f.menu(&format!("#tree-list [data-id='{e}']"))?;
        assert!(!common::visible(
            &f.ui,
            "[data-node-action='settle-branch']"
        ));
        f.shot("resettle-attempt", theme)?;
        f.ui.pages.settle_branch(e)?;
        assert!(common::text(&f.ui, "#toast").contains("1 waiting kept"));
        common::click_chrome(&f.ui, "[data-node-action='open']")?;
        common::wait("reopened settled page", || {
            f.ui.snapshot().is_ok_and(|s| s.current == Some(e))
        })?;
    }
    f.ui.pages.settle_branch(d)?;
    common::wait("settled branch again", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(a))
    })?;
    common::key("ctrl+k")?;
    common::type_text("unread")?;
    common::wait("palette result", || {
        common::text(&f.ui, "#palette-results").contains("/unread")
    })?;
    common::key("Return")?;
    common::wait("palette opens settled descendant", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(waiting))
    })?;
    assert!(f.ui.pages.read(|b| b.visit(d))?.branch_settled_at.is_none());
    println!("PASS node menus on strip and tree rows, settled stubs on both tree sides, and palette reopening.");
    f.select(e)?;
    f.ui.pages.inject_recent_memory_sample(0);
    let waiting = f.queue("/remaining")?;
    f.committed(waiting)?;
    let skipped = f.queue("/skipped")?;
    f.committed(skipped)?;
    for theme in ["dark", "light"] {
        common::chrome(&f.ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        common::pump(200);
        f.menu(&common::node(skipped))?;
        assert!(common::visible(&f.ui, "[data-node-action='dismiss']"));
        assert!(!common::visible(&f.ui, "[data-node-action='move-to-tree']"));
        f.shot("waiting-menu", theme)?;
        common::key("Escape")?;
    }
    f.ui.pages.dismiss(skipped)?;
    common::wait("dismiss releases never seen view", || {
        f.ui.pages.background_view(skipped).is_none()
    })?;
    assert!(f
        .ui
        .pages
        .read(|b| b.visit(skipped))?
        .dismissed_at
        .is_some());
    common::wait("dismiss leaves waiting queue", || {
        common::chrome(
            &f.ui,
            &format!("!canopy.state().waiting.includes('{skipped}')"),
        )
        .is_ok_and(|v| v == true)
    })?;
    for theme in ["dark", "light"] {
        common::chrome(&f.ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        common::key("ctrl+shift+e")?;
        common::wait("tree overlay", || common::visible(&f.ui, "#tree-side"))?;
        common::click_chrome(&f.ui, "#tree-side [data-side='tree']")?;
        common::wait("dismissed in tree", || {
            common::visible(
                &f.ui,
                &format!("#tree-nodes [data-id='{skipped}'].dismissed"),
            )
        })?;
        f.shot("dismissed", theme)?;
        common::key("Escape")?;
    }
    f.select(skipped)?;
    assert!(f
        .ui
        .pages
        .read(|b| b.visit(skipped))?
        .dismissed_at
        .is_none());
    assert_eq!(f.hits("/skipped"), 2);
    f.select(e)?;
    common::key("ctrl+Tab")?;
    common::wait(
        "Ctrl Tab skips dismissal and reads remaining waiting",
        || f.ui.snapshot().is_ok_and(|s| s.current == Some(waiting)),
    )?;
    println!("PASS waiting-row node menu, Dismiss drops its background view and queue entry, reopening clears the mark, Ctrl+Tab reads remaining waiting.");
    f.ui.pages.place_tree(
        f.ui.pages
            .read(|b| {
                canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)
            })?
            .tree_of(a)
            .unwrap(),
        canopy::browser::Move::History,
    )?;
    common::wait("tree settle undo", || {
        common::visible(&f.ui, "#toast button")
    })?;
    for theme in ["dark", "light"] {
        common::chrome(&f.ui, &format!("intent('theme', {{mode:'{theme}'}})"))?;
        f.shot("settled-tree-toast", theme)?;
    }
    common::click_chrome(&f.ui, "#toast button")?;
    common::wait("tree undo returns", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(waiting))
    })?;
    println!("PASS whole tree settling uses the same toast and Undo.");
    f.select(a)?;
    f.menu("#strip-nodes .pill")?;
    assert!(!common::visible(
        &f.ui,
        "[data-node-action='settle-branch']"
    ));
    assert!(!common::visible(&f.ui, "[data-node-action='move-to-tree']"));
    common::click_chrome(&f.ui, "[data-node-action='copy']")?;
    let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
    assert_eq!(
        clipboard.wait_for_text().as_deref(),
        Some(f.url("/garden").as_str())
    );
    f.menu("#strip-nodes .pill")?;
    common::click_chrome(&f.ui, "[data-node-action='new-tree']")?;
    common::wait("page opened in new tree", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.current_visit()
                .is_some_and(|v| v.id != a && v.parent.is_none() && v.url == f.url("/garden"))
        })
    })?;
    f.menu("#strip-nodes .pill")?;
    let anchor = f.ui.snapshot()?.current.unwrap();
    common::click_chrome(&f.ui, "[data-node-action='branch']")?;
    common::wait("branch palette", || {
        common::visible(&f.ui, "#palette-input")
    })?;
    common::type_text(&format!("{}/new-branch", f.origin))?;
    common::key("Return")?;
    common::wait("new branch attached to menu page", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.current_visit()
                .is_some_and(|v| v.parent == Some(anchor) && v.url.path() == "/new-branch")
        })
    })?;
    println!("PASS node menu Open in a new tree, New branch here, Copy address, and inapplicable root rows.");
    let leaf = f.ui.snapshot()?.current.unwrap();
    f.ui.pages.settle_branch(leaf)?;
    common::wait("last settle lands", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(anchor))
    })?;
    assert_eq!(
        common::chrome(
            &f.ui,
            "getComputedStyle(document.getElementById('toast')).pointerEvents"
        )?,
        "none"
    );
    assert_eq!(
        common::chrome(
            &f.ui,
            "getComputedStyle(document.querySelector('#toast button')).pointerEvents"
        )?,
        "auto"
    );
    common::pump(6300);
    assert!(!common::visible(&f.ui, "#toast"));
    common::key("ctrl+shift+t")?;
    common::wait("shortcut reopens branch after toast", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(leaf))
    })?;
    assert!(f
        .ui
        .pages
        .read(|b| b.visit(leaf))?
        .branch_settled_at
        .is_none());
    println!(
        "PASS Ctrl+Shift+T reopens a branch after its toast and only the Undo button receives pointer events."
    );
    let undo_root = f.root("/early-undo-root")?;
    let undo_branch = f.child(undo_root, "/early-undo-branch")?;
    let old_view = f.ui.pages.selected_page().native.clone();
    assert!(f.ui.pages.selected_history_visits().contains(&undo_root));
    f.ui.pages.settle_branch(undo_branch)?;
    f.ui.pages.undo_settle()?;
    common::wait("immediate Undo returns through retirement", || {
        f.ui.snapshot()
            .is_ok_and(|s| s.current == Some(undo_branch))
            && !f.ui.pages.selected_page().native.is_loading()
            && f.ui.pages.selected_page().native != old_view
    })?;
    assert!(
        f.ui.pages.selected_history_visits().contains(&undo_root),
        "immediate Undo keeps the retired page's checkpoint history"
    );
    println!("PASS immediate Undo waits for retirement and retains checkpoint history.");
    let nav_root = f.root("/navigation-root")?;
    let older = f.child(nav_root, "/older")?;
    f.select(nav_root)?;
    let settled = f.child(nav_root, "/put-away")?;
    let nested = f.child(settled, "/nested")?;
    let kept = f.queue("/kept")?;
    f.select(nav_root)?;
    f.ui.pages.settle_branch(nested)?;
    f.ui.pages.settle_branch(settled)?;
    common::wait(
        "settle counts waiting inside an earlier settled branch",
        || common::text(&f.ui, "#toast").contains("1 waiting kept"),
    )?;
    f.ui.pages.settle_branch(nested)?;
    f.ui.pages.undo_settle()?;
    assert!(f
        .ui
        .pages
        .read(|b| b.visit(settled))?
        .branch_settled_at
        .is_none());
    assert!(f
        .ui
        .pages
        .read(|b| b.visit(nested))?
        .branch_settled_at
        .is_some());
    common::wait(
        "Undo reopens the off-path branch before navigating away",
        || {
            f.ui.snapshot().is_ok_and(|s| s.current == Some(settled))
                && !f.ui.pages.selected_page().native.is_loading()
        },
    )?;
    f.select(nav_root)?;
    f.ui.pages.settle_branch(settled)?;
    let skipped_sibling = f.queue("/dismissed-sibling")?;
    f.ui.pages.dismiss(skipped_sibling)?;
    let newest = f.child(nav_root, "/newest")?;
    common::key("alt+Down")?;
    common::wait("Down skips dismissed and settled siblings", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(older))
    })?;
    common::key("alt+Up")?;
    common::wait("Up skips dismissed and settled siblings", || {
        f.ui.snapshot().is_ok_and(|s| s.current == Some(newest))
    })?;
    f.select(older)?;
    f.select(nav_root)?;
    common::key("alt+Right")?;
    common::wait(
        "Right follows a child last shown from another sibling",
        || f.ui.snapshot().is_ok_and(|s| s.current == Some(older)),
    )?;
    f.select(nav_root)?;
    f.ui.pages.move_branch(
        settled,
        canopy::browser::MoveTo::OwnTree(canopy::browser::TabPlacement::End),
    )?;
    common::wait("moved settled branch is an open tree", || {
        common::chrome(
            &f.ui,
            &format!("!!document.querySelector('#tab-row .tab[data-id=\"{settled}\"]')"),
        )
        .is_ok_and(|v| v == true)
    })?;
    assert!(f
        .ui
        .pages
        .read(|b| b.visit(settled))?
        .branch_settled_at
        .is_none());
    assert!(f
        .ui
        .pages
        .read(|b| b.visit(nested))?
        .branch_settled_at
        .is_some());
    assert!(f.ui.pages.read(|b| b.visit(kept))?.opened_at.is_none());
    println!("PASS repeat settling keeps waiting and does not replace Undo; sibling keys skip inactive pages; Right follows all arrivals; moving clears only the new root's settled mark.");
    f.ui.window.close();
    common::pump(100);
    let restart = tempfile::tempdir()?;
    let mut browser = canopy::browser::Browser::open(&restart.path().join("forest.sqlite3"))?;
    let mut nav = canopy::browser::Navigation::default();
    let mut parent = None;
    let mut root = None;
    for path in ["/restart-root", "/restart-branch"] {
        let target = parent.map_or(
            canopy::browser::NavigationTarget::Root,
            canopy::browser::NavigationTarget::ChildOf,
        );
        let url = browser.navigate(&mut nav, f.url(path), target)?;
        let ticket = nav.pending_ticket().unwrap();
        nav.started(ticket);
        browser.commit_load(&mut nav, ticket, url, canopy::browser::Visibility::Selected)?;
        parent = browser.snapshot()?.current;
        if root.is_none() {
            root = parent;
        }
    }
    let branch = parent.unwrap();
    browser.settle_branch(branch)?;
    drop(browser);
    let resumed = canopy::ui::Ui::new(restart.path())?;
    resumed.window.show_all();
    common::ready(&resumed)?;
    resumed.resume(&[])?;
    common::wait("restart lands outside settled branch", || {
        resumed.snapshot().is_ok_and(|s| s.current == root)
    })?;
    assert!(resumed
        .pages
        .read(|b| b.visit(branch))?
        .branch_settled_at
        .is_some());
    resumed.window.close();
    println!("PASS restart during a branch landing keeps the branch put away.");
    Ok(())
}

fn page_move_picker() -> Result<()> {
    let f = Fixture::new()?;
    let source = f.root("/move-source")?;
    let head = f.child(source, "/move-head")?;
    let leaf = f.child(head, "/move-leaf")?;
    let target = f.root("/move-target")?;
    f.select(leaf)?;
    let view = f.ui.pages.selected_page().native.clone();
    let hits = f.hits("/move-leaf");
    f.ui.chrome.native().grab_focus();
    common::chrome(&f.ui, &format!("(() => {{ document.querySelector(\"#strip-nodes [data-page='{head}']\").focus(); return true; }})()"))?;
    common::key("shift+F10")?;
    common::wait("keyboard page menu", || {
        common::visible(&f.ui, "[data-node-action='choose-move-tree']")
    })?;
    // Delete is the last row; Move to tree… sits just above it.
    common::key("End")?;
    common::key("Up")?;
    common::key("Return")?;
    common::wait("tree picker destination", || {
        common::visible(&f.ui, &format!("[data-move-tree='{target}']"))
    })?;
    common::key("Home")?;
    common::key("Return")?;
    common::wait("picker moves branch", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.visits
                .iter()
                .any(|v| v.id == head && v.parent == Some(target))
        })
    })?;
    assert_eq!(f.ui.snapshot()?.current, Some(leaf));
    assert_eq!(f.ui.pages.selected_page().native, view);
    assert_eq!(f.hits("/move-leaf"), hits);
    let db = rusqlite::Connection::open(f._directory.path().join("forest.sqlite3"))?;
    let parent: String = db.query_row(
        "SELECT parent_id FROM visits WHERE id=?1",
        [head.to_string()],
        |r| r.get(0),
    )?;
    assert_eq!(parent, target.to_string());
    let last: String = db.query_row(
        "SELECT last_visit_id FROM tree_state WHERE root_id=?1",
        [target.to_string()],
        |r| r.get(0),
    )?;
    assert_eq!(last, leaf.to_string());
    f.select(source)?;
    f.menu("#strip-nodes .pill")?;
    assert_eq!(
        common::text(&f.ui, "[data-node-action='choose-move-tree']"),
        "Merge into tree…"
    );
    common::click_chrome(&f.ui, "[data-node-action='choose-move-tree']")?;
    common::wait("Rust merge warning", || {
        common::text(&f.ui, "#popover").contains("drops its name and tab")
    })?;
    common::key("Home")?;
    common::key("Return")?;
    common::wait("picker merges root", || {
        f.ui.snapshot().is_ok_and(|s| {
            s.visits
                .iter()
                .any(|v| v.id == source && v.parent == Some(target))
        })
    })?;
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM tree_state WHERE root_id=?1",
            [source.to_string()],
            |r| r.get::<_, usize>(0)
        )?,
        0
    );
    f.ui.window.close();
    common::pump(200);
    let restarted = canopy::ui::Ui::new(f._directory.path())?;
    restarted.window.show_all();
    common::ready(&restarted)?;
    restarted.resume(&[])?;
    common::wait("moved tree survives restart", || {
        restarted
            .snapshot()
            .is_ok_and(|s| s.current == Some(source))
    })?;
    let snapshot = restarted.snapshot()?;
    assert_eq!(
        snapshot
            .visits
            .iter()
            .find(|v| v.id == source)
            .unwrap()
            .parent,
        Some(target)
    );
    assert_eq!(
        snapshot
            .visits
            .iter()
            .find(|v| v.id == head)
            .unwrap()
            .parent,
        Some(target)
    );
    assert_eq!(
        snapshot
            .visits
            .iter()
            .find(|v| v.id == leaf)
            .unwrap()
            .parent,
        Some(head)
    );
    restarted.pages.read(|b| {
        canopy::forest::ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?)
    })?;
    restarted.window.close();
    common::pump(100);
    println!("PASS keyboard tree picker moves a live branch, merges a root, persists parent edges and tree state across restart.");
    Ok(())
}

fn drag_branch(f: &Fixture, source: &str, end: (i32, i32), cue: &str, escape: bool) -> Result<()> {
    let start = common::chrome_point(&f.ui, source)?;
    common::move_to(start.0, start.1)?;
    common::pump(80);
    common::xdo(&["mousedown", "1"])?;
    common::wait("page choices prepared before native drag", || {
        common::chrome(&f.ui, "!!pagePress?.choices").is_ok_and(|v| v == true)
    })?;
    common::pump(180);
    for step in 1..=25 {
        common::move_to(
            start.0 + (end.0 - start.0) * step / 25,
            start.1 + (end.1 - start.1) * step / 25,
        )?;
        common::pump(50);
    }
    common::pump(150);
    assert_eq!(common::chrome(&f.ui, "nativeDrag?.kind")?, "page");
    assert_eq!(common::chrome(&f.ui, cue)?, true);
    assert_eq!(
        common::chrome(
            &f.ui,
            "Number(getComputedStyle(document.querySelector('.dragging')).opacity) < 1"
        )?,
        true
    );
    // Neither blur nor a zero-button event can release the native drag latch.
    assert_eq!(common::chrome(&f.ui, "(window.dispatchEvent(new Event('blur')), window.dispatchEvent(new MouseEvent('mousemove', {buttons:0})), true)")?, true);
    common::pump(80);
    assert_eq!(
        common::chrome(
            &f.ui,
            "!!nativeDrag && !!document.querySelector('.dragging')"
        )?,
        true
    );
    if escape {
        common::xdo(&["key", "Escape"])?;
    }
    common::xdo(&["mouseup", "1"])?;
    common::wait("native page drag cleaned up", || {
        common::chrome(&f.ui, "!nativeDrag && !document.querySelector('.dragging, .page-drop-allowed, .page-drop-refused') && !document.getElementById('tab-row').classList.contains('drop-target')")
            .is_ok_and(|value| value == true)
    })?;
    common::pump(250);
    Ok(())
}

fn drag_store(f: &Fixture) -> Result<serde_json::Value> {
    let db = rusqlite::Connection::open_with_flags(
        f._directory.path().join("forest.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let visits = db
        .prepare("SELECT id, parent_id FROM visits ORDER BY id")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let trees = db
        .prepare(
            "SELECT root_id, last_visit_id, name, place, position, pinned, placed_at
             FROM tree_state
             ORDER BY CASE place WHEN 'tab' THEN 0 WHEN 'shelf' THEN 1 ELSE 2 END,
                 pinned DESC, position, placed_at DESC, root_id",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(serde_json::json!({"visits":visits, "trees":trees}))
}

fn stored_parent(f: &Fixture, id: canopy::browser::VisitId) -> Result<Option<String>> {
    let db = rusqlite::Connection::open_with_flags(
        f._directory.path().join("forest.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    Ok(db.query_row(
        "SELECT parent_id FROM visits WHERE id = ?1",
        [id.to_string()],
        |row| row.get(0),
    )?)
}

fn page_move_drag() -> Result<()> {
    let f = Fixture::new()?;
    let a = f.root("/drag-root")?;
    let b = f.child(a, "/drag-branch")?;
    let c = f.child(b, "/drag-child")?;
    let d = f.root("/drag-target")?;
    f.select(c)?;
    let node = |id| format!("#strip-nodes .node[data-page='{id}']");
    let tab = |id| format!("#tab-row .tab[data-id='{id}']");
    let target = common::chrome_point(&f.ui, &tab(d))?;
    drag_branch(
        &f,
        &node(b),
        target,
        "!!document.querySelector('.tab.page-drop-allowed')",
        false,
    )?;
    assert_eq!(stored_parent(&f, b)?, Some(d.to_string()));
    assert_eq!(stored_parent(&f, c)?, Some(b.to_string()));
    assert_eq!(f.ui.snapshot()?.current, Some(c));
    println!("PASS page drag onto a tab moves the branch and preserves the selected page.");

    let target = common::chrome_point(&f.ui, &node(d))?;
    drag_branch(
        &f,
        &node(c),
        target,
        "!!document.querySelector('.node.page-drop-allowed')",
        false,
    )?;
    assert_eq!(stored_parent(&f, c)?, Some(d.to_string()));
    println!("PASS page drag onto a node reparents the branch.");

    let blank = common::screen_point(&f.ui, &f.ui.chrome.native(), 850, 68)?;
    drag_branch(
        &f,
        &node(c),
        blank,
        "document.getElementById('tab-row').classList.contains('drop-target')",
        false,
    )?;
    assert_eq!(stored_parent(&f, c)?, None);
    assert_eq!(
        drag_store(&f)?["trees"].as_array().unwrap().last().unwrap()[0],
        c.to_string()
    );
    println!("PASS empty strip creates a tree at the end of the stored tab order.");

    let target = common::chrome_point(&f.ui, &tab(d))?;
    drag_branch(&f, &node(c), target, "!!document.querySelector('.tab.page-drop-allowed') && !document.getElementById('page-drag-warning').hidden && document.getElementById('page-drag-warning').textContent.includes('drops its name and tab')", false)?;
    assert_eq!(stored_parent(&f, c)?, Some(d.to_string()));
    assert!(!drag_store(&f)?["trees"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row[0] == c.to_string()));
    println!("PASS root drag merges its tree and shows the Rust merge warning.");

    let before = drag_store(&f)?;
    let target = common::chrome_point(&f.ui, &node(c))?;
    drag_branch(
        &f,
        &node(d),
        target,
        "!!document.querySelector('.node.page-drop-refused')",
        false,
    )?;
    assert_eq!(drag_store(&f)?, before);
    println!("PASS dropping a root onto its child is refused without stored changes.");

    let outside = common::page_point(&f.ui, 600, 400)?;
    drag_branch(&f, &node(c), outside, "!document.querySelector('.page-drop-allowed') && !document.getElementById('tab-row').classList.contains('drop-target')", false)?;
    assert_eq!(drag_store(&f)?, before);
    println!("PASS release over the page view does not move a branch.");

    let target = common::chrome_point(&f.ui, &tab(a))?;
    drag_branch(
        &f,
        &node(c),
        target,
        "!!document.querySelector('.tab.page-drop-allowed')",
        true,
    )?;
    assert_eq!(drag_store(&f)?, before);
    println!("PASS Escape cancels a page drag without stored changes.");

    common::hover_chrome(&f.ui, &node(d))?;
    let row = format!("#strip-list .menu-row[data-id='{b}']");
    common::wait("attached-list branch is draggable", || {
        common::visible(&f.ui, &row)
    })?;
    let target = common::chrome_point(&f.ui, &tab(a))?;
    drag_branch(
        &f,
        &row,
        target,
        "!!document.querySelector('.tab.page-drop-allowed')",
        false,
    )?;
    assert_eq!(stored_parent(&f, b)?, Some(a.to_string()));
    println!("PASS attached-list rows can start a branch drag and survive native blur.");

    let (x, _, _, _) = common::rect(&f.ui, &tab(a))?;
    let gap = common::screen_point(&f.ui, &f.ui.chrome.native(), x as i32 + 2, 5)?;
    drag_branch(
        &f,
        &node(d),
        gap,
        "document.getElementById('tab-row').classList.contains('drop-target')",
        false,
    )?;
    assert_eq!(drag_store(&f)?["trees"][0][0], d.to_string());
    assert_eq!(stored_parent(&f, d)?, None);
    println!("PASS root on a tab-row gap reorders its tab without merging.");
    let waiting = f.queue("/drag-waiting")?;
    let target = common::chrome_point(&f.ui, &tab(d))?;
    drag_branch(
        &f,
        &node(waiting),
        target,
        "!!document.querySelector('.tab.page-drop-allowed')",
        false,
    )?;
    common::wait("waiting tab moves to the drop target", || {
        stored_parent(&f, waiting).is_ok_and(|p| p == Some(d.to_string()))
    })?;
    println!("PASS a waiting tab starts the native page drag and moves to the destination tree.");

    let before = drag_store(&f)?;

    f.ui.window.close();
    common::pump(150);
    let restarted = canopy::ui::Ui::new(f._directory.path())?;
    restarted.window.show_all();
    common::ready(&restarted)?;
    assert_eq!(drag_store(&f)?, before);
    assert_eq!(restarted.snapshot()?.current, Some(c));
    restarted.window.close();
    common::pump(100);
    println!("PASS moved branch parents, tree state and selection survive restart.");
    Ok(())
}
