use anyhow::Result;
use canopy::ui::Ui;
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};
use webkit2gtk::{
    ContextMenuAction, ContextMenuExt, ContextMenuItemExt, HitTestResultExt, WebViewExt,
};
mod common;
use common::*;

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    gtk::Settings::default()
        .unwrap()
        .set_property("gtk-enable-animations", true);
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    assert!(chrome(&ui, "!document.getElementById('agent')")? == true);
    println!("PASS no disabled Agent button.");
    let menus = Rc::new(Cell::new(0));
    let seen = menus.clone();
    let copies = Rc::new(Cell::new(0));
    let copy_seen = copies.clone();
    ui.chrome
        .native()
        .connect_context_menu(move |_, menu, _, hit| {
            assert!(hit.context_is_editable() || hit.context_is_selection());
            if hit.context_is_selection() {
                assert!(menu
                    .items()
                    .iter()
                    .any(|item| item.stock_action() == ContextMenuAction::Copy));
                copy_seen.set(copy_seen.get() + 1);
            }
            seen.set(seen.get() + 1);
            false
        });
    hover_chrome(&ui, "#start-search")?;
    xdo(&["click", "3"])?;
    pump(250);
    assert_eq!(menus.get(), 0);
    click_chrome(&ui, "#start-search")?;
    wait("palette input", || visible(&ui, "#palette-input"))?;
    hover_chrome(&ui, "#palette-input")?;
    xdo(&["click", "3"])?;
    wait("native editing menu", || menus.get() == 1)?;
    key("Escape")?;
    key("Escape")?;
    println!(
        "PASS right-click suppresses chrome navigation menus and preserves native editing menus."
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}", listener.local_addr()?);
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for mut stream in listener.incoming().flatten() {
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let body = "<title>Tooltip fixture</title><a id='waiting' href='/waiting'>An article to read later</a> <a id='child' href='/child'>Follow a branch</a>";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    ui.pages.selected_page().navigate(
        canopy::address::web_url(&url)?,
        canopy::browser::NavigationTarget::Root,
    )?;
    wait("fixture loaded", || {
        text(&ui, ".tab-name").contains("Tooltip fixture")
    })?;
    let root = chrome(&ui, "current()")?;
    chrome(
        &ui,
        r#"(() => {
        const root = current();
        const observer = new MutationObserver(() => {
            const pill = document.querySelector('#strip-nodes .pill');
            if (current() !== root && pill?.style.opacity === '0') {
                window.branchStartedHidden = true;
                observer.disconnect();
            }
        });
        observer.observe($('strip-nodes'), {subtree: true, attributes: true, childList: true});
    })()"#,
    )?;
    let (x, y) = element_point(&ui, "child")?;
    move_to(x, y)?;
    xdo(&["click", "1"])?;
    wait("new child fades in at the end of its connector", || {
        chrome(&ui, "window.branchStartedHidden && document.querySelector('#strip-nodes .pill')?.style.opacity === '1'")
            .is_ok_and(|v| v == true)
    })?;
    key("alt+Left")?;
    wait("return to parent", || {
        chrome(&ui, "current()").is_ok_and(|v| v == root)
    })?;
    let (x, y) = element_point(&ui, "waiting")?;
    move_to(x, y)?;
    xdo(&["click", "2"])?;
    wait("waiting counter", || visible(&ui, "#waiting-pin"))?;
    for enabled in [false, true] {
        gtk::Settings::default()
            .unwrap()
            .set_property("gtk-enable-animations", enabled);
        wait("motion preference applied", || {
            chrome(&ui, "reducedMotion.matches").is_ok_and(|v| v == !enabled)
        })?;
        key("ctrl+e")?;
        wait("forest current page", || {
            visible(&ui, ".canopy-item[data-current=true]:not([data-kind=tree])")
        })?;
        let animation = chrome(&ui, "getComputedStyle(document.querySelector('.canopy-item[data-current=true]:not([data-kind=tree])')).animationName")?;
        assert_eq!(animation, if enabled { "forest-locate" } else { "none" });
        key("Escape")?;
        wait("forest closed", || !visible(&ui, "#forest"))?;
        move_to(1300, 900)?;
        item_list(&ui, "#strip-nodes .after")?;
        let animation = chrome(&ui, "getComputedStyle($('strip-list')).animationName")?;
        assert_eq!(animation, if enabled { "list-enter" } else { "none" });
        key("Escape")?;
    }
    for theme in ["dark", "light"] {
        chrome(&ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
        wait("theme", || {
            chrome(&ui, "S.frame.theme").is_ok_and(|v| v == theme)
        })?;
        for (name, selector, button) in [
            ("waiting", "#waiting-pin", None),
            ("settings", "#settings-button", Some("1")),
            ("overflow", "#tab-overflow", Some("1")),
            ("tab", ".tab", Some("3")),
        ] {
            chrome(&ui, "(closePopover(false), true)")?;
            if name == "overflow" {
                chrome(&ui, "($('tab-overflow').hidden=false, true)")?;
            }
            hover_chrome(&ui, selector)?;
            pump(100);
            if let Some(button) = button {
                xdo(&["click", button])?;
            }
            wait("open anchored menu", || visible(&ui, "#popover"))?;
            if name == "tab" {
                chrome(&ui, "(renderTabs(), true)")?;
            }
            pump(1000);
            let shown = tooltip_query(&ui, selector)?;
            assert!(
                !shown,
                "{theme} {name} tooltip query must refuse the stationary pointer"
            );
            shot(&ui, &format!("{theme}-{name}-held"))?;
            chrome(&ui, "(closePopover(false), true)")?;
            move_to(1300, 900)?;
        }
        println!("PASS {theme} stationary pointer tooltip queries refuse waiting, settings, overflow and refreshed tab menus after 1 s.");
        hover_chrome(&ui, "#settings-button")?;
        pump(1000);
        assert!(
            tooltip_query(&ui, "#settings-button")?,
            "tooltips must resume after menus close"
        );
        for layer in ["palette", "overlay"] {
            if layer == "palette" {
                key("ctrl+k")?;
            } else {
                key("ctrl+e")?;
            }
            pump(1000);
            assert!(
                !tooltip_query(&ui, "#settings-button")?,
                "{layer} must refuse tooltips"
            );
            key("Escape")?;
        }
        chrome(&ui, "(canopy.reader({title:'Saved article',kind:'Page',at:0,url:'https://example.org',text:'A saved passage to select and copy.'}), true)")?;
        wait("reader", || visible(&ui, "#capture-text"))?;
        let (x, y) = chrome_point(&ui, "#capture-text")?;
        let (_, _, w, h) = rect(&ui, "#capture-text")?;
        let (left, top) = (x - w as i32 / 2 + 2, y - h as i32 / 2 + 10);
        move_to(left, top)?;
        xdo(&["mousedown", "1"])?;
        move_to(left + 220, top)?;
        xdo(&["mouseup", "1"])?;
        wait("reader selection", || {
            chrome(&ui, "getSelection().toString().length > 0").is_ok_and(|v| v == true)
        })?;
        move_to(left + 20, top)?;
        let before = copies.get();
        xdo(&["click", "3"])?;
        wait("reader Copy menu", || copies.get() == before + 1)?;
        shot(&ui, &format!("{theme}-reader-copy"))?;
        key("Escape")?;
        key("Escape")?;
        println!("PASS {theme} selected reader text offers native Copy.");
        chrome(&ui, "(closePopover(false), true)")?;
        item_list(&ui, "#strip-nodes .after")?;
        wait("waiting list", || {
            visible(&ui, "#strip-list .menu-row[data-id]")
        })?;
        assert!(text(&ui, "#strip-list").contains("Waiting"));
        assert!(!text(&ui, "#strip-list").contains("pending"));
        shot(&ui, &format!("{theme}-waiting-list"))?;
        hover_chrome(&ui, "#strip-list .menu-row[data-id]")?;
        xdo(&["click", "3"])?;
        wait("page actions", || {
            visible(&ui, "#popover [data-node-action]")
        })?;
        pump(1000);
        assert!(!tooltip_query(&ui, "#strip-list .menu-row[data-id]")?);
        assert!(text(&ui, "#popover").contains("Dismiss"));
        shot(&ui, &format!("{theme}-waiting-menu"))?;
        key("Escape")?;
        key("Escape")?;
        move_to(1300, 900)?;
        pump(500);
        println!("PASS {theme} attached waiting list uses waiting wording.");
        for (kind, anchor) in [
            ("settings", "settings-button"),
            ("trees", "tab-overflow"),
            ("memory", "memory-dot"),
        ] {
            assert!(chrome(&ui, &format!("(() => {{const a=$('{anchor}'); a.title='Original tooltip'; openPopover('{kind}',a,null); return !a.hasAttribute('title');}})()"))? == true);
            chrome(&ui, "(closePopover(false), true)")?;
            assert_eq!(
                chrome(&ui, &format!("$('{anchor}').title"))?,
                "Original tooltip"
            );
        }
        chrome(
            &ui,
            "(openPopover('settings', $('settings-button'), null), true)",
        )?;
        shot(&ui, &format!("{theme}-settings"))?;
        chrome(&ui, "(closePopover(false), true)")?;
        ui.chrome.toast(canopy::chrome::Toast::Error(
            "Could not load this page".into(),
        ));
        wait("error painted", || visible(&ui, "#error-toast"))?;
        shot(&ui, &format!("{theme}-error"))?;
        click_chrome(&ui, "#toast-close")?;
    }
    assert!(chrome(&ui, "(() => {canopy.logins({offer:1,names:['First','Second']}); const a=$('login-chip');openPopover('logins',a,null); canopy.logins({offer:1,names:['Updated','Second']});return !a.hasAttribute('title');})()")? == true);
    chrome(&ui, "(closePopover(false), true)")?;
    assert!(
        chrome(
            &ui,
            "$ ('login-chip').title.includes('Choose a saved login')"
        )? == true
    );
    key("ctrl+t")?;
    wait("start data", || {
        chrome(&ui, "!!S.frame.start?.bookmarks").is_ok_and(|v| v == true)
    })?;
    assert!(chrome(&ui, "(() => {const b=S.frame.start.bookmarks;b.folders=[{name:'Research',count:1,rows:[]}];renderStart();openPopover('folder',document.querySelector('.folder-chip'),{folder:'Research'});renderStart();return !S.popover.anchor.hasAttribute('title');})()")? == true);
    chrome(&ui, "(closePopover(false), true)")?;
    assert_eq!(
        chrome(&ui, "document.querySelector('.folder-chip').title")?,
        "Research"
    );
    println!("PASS shared menu lifecycle suppresses and restores native tooltips in both themes.");
    gtk::Settings::default()
        .unwrap()
        .set_property("gtk-enable-animations", false);
    wait("desktop reduced motion", || {
        chrome(&ui, "reducedMotion.matches").is_ok_and(|v| v == true)
    })?;
    assert!(chrome(&ui, "(animateStrip(300), !strip.anim)")? == true);
    chrome(&ui, "canopy.waiting('Waiting quietly')")?;
    assert_eq!(
        chrome(&ui, "document.querySelectorAll('.waiting-pulse').length")?,
        0
    );
    chrome(&ui, "(canopy.toast('info', 'Reduced motion'), true)")?;
    wait("reduced-motion CSS", || {
        chrome(&ui, "getComputedStyle($('toast')).transitionDuration").is_ok_and(|v| v == "0s")
    })?;
    println!("PASS desktop reduced motion completes strip moves and CSS transitions immediately.");
    ui.window.close();
    Ok(())
}

fn shot(ui: &Ui, name: &str) -> Result<()> {
    if let Ok(folder) = std::env::var("CANOPY_POLISH_SHOTS") {
        std::fs::create_dir_all(&folder)?;
        save_window(ui, &format!("{folder}/{name}.png"))?;
        {
            let status = std::process::Command::new("import")
                .args(["-window", "root", &format!("{folder}/{name}-composed.png")])
                .status()?;
            assert!(status.success());
        }
    }
    Ok(())
}

fn tooltip_query(ui: &Ui, selector: &str) -> Result<bool> {
    let (x, y, w, h) = rect(ui, selector)?;
    let tooltip: gtk::Tooltip = glib::Object::new();
    Ok(ui.chrome.native().emit_by_name::<bool>(
        "query-tooltip",
        &[
            &((x + w / 2.) as i32),
            &((y + h / 2.) as i32),
            &false,
            &tooltip,
        ],
    ))
}
