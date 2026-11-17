use anyhow::Result;
use canopy::ui::Ui;
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};
use webkit2gtk::{HitTestResultExt, WebViewExt};
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
    ui.chrome
        .native()
        .connect_context_menu(move |_, _, _, hit| {
            assert!(hit.context_is_editable());
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
    for theme in ["dark", "light"] {
        chrome(&ui, &format!("(intent('theme', {{mode:'{theme}'}}), true)"))?;
        wait("theme", || {
            chrome(&ui, "S.frame.theme").is_ok_and(|v| v == theme)
        })?;
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
        chrome(
            &ui,
            "(closePopover(false), canopy.toast('error', 'Could not load this page'), true)",
        )?;
        shot(&ui, &format!("{theme}-error"))?;
        chrome(&ui, "($('toast').hidden=true, true)")?;
    }
    assert!(chrome(&ui, "(() => {canopy.logins({offer:1,names:['First','Second']}); const a=$('login-chip');openPopover('logins',a,null); canopy.logins({offer:1,names:['Updated','Second']});return !a.hasAttribute('title');})()")? == true);
    chrome(&ui, "(closePopover(false), true)")?;
    assert!(
        chrome(
            &ui,
            "$ ('login-chip').title.includes('Choose a saved login')"
        )? == true
    );
    assert!(chrome(&ui, "(() => {const b=S.frame.start.bookmarks;b.folders=[{name:'Research',count:1,rows:[]}];renderStart();openPopover('folder',document.querySelector('.folder-chip'),{folder:'Research'});S.rev++;renderStart();return !S.popover.anchor.hasAttribute('title');})()")? == true);
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
    chrome(
        &ui,
        "(canopy.toggleForest(), S.camera.zoom=.5, fitForest(true), true)",
    )?;
    let camera = chrome(&ui, "({...S.camera})")?;
    pump(80);
    assert_eq!(chrome(&ui, "({...S.camera})")?, camera);
    chrome(&ui, "(canopy.toast('error', 'Reduced motion'), true)")?;
    wait("reduced-motion CSS", || {
        chrome(&ui, "getComputedStyle($('toast')).transitionDuration").is_ok_and(|v| v == "0s")
    })?;
    println!("PASS desktop reduced motion completes strip moves, forest camera and CSS transitions immediately.");
    ui.window.close();
    Ok(())
}

fn shot(ui: &Ui, name: &str) -> Result<()> {
    if let Ok(folder) = std::env::var("CANOPY_POLISH_SHOTS") {
        std::fs::create_dir_all(&folder)?;
        save_window(ui, &format!("{folder}/{name}.png"))?;
        if name == "light-settings" {
            let status = std::process::Command::new("import")
                .args([
                    "-window",
                    "root",
                    &format!("{folder}/light-settings-composed.png"),
                ])
                .status()?;
            assert!(status.success());
        }
    }
    Ok(())
}
