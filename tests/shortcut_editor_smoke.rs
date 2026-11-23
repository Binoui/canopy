mod common;
#[path = "common/http.rs"]
mod http;

use std::{fs, io::Write, net::TcpListener, path::Path, process::Command, thread};

use anyhow::{ensure, Result};
use canopy::{address::web_url, ui::Ui};
use common::*;
use gtk::prelude::*;
use serde_json::{json, Value};
use webkit2gtk::WebViewExt;

const EDITOR: &str = "#shortcut-editor";
const SEARCH: &str = "#shortcut-editor-search";
const FOREST: &str = "[data-shortcut-row=forest]";
const FOREST_KEY: &str = "[data-shortcut-row=forest] [data-slot='0']";
const FOREST_ADD: &str = "[data-shortcut-row=forest] [data-key=add]";
const FOCUS_ROW: &str = "[data-shortcut-row=toggle-focus]";
const RESET_ALL: &str = "[data-shortcut-action=ask-reset-all]";

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [phase, directory, base] = args.as_slice() {
        let directory = Path::new(directory);
        start(directory)?;
        let ui = Ui::new(directory)?;
        ui.window.show_all();
        ready(&ui)?;
        let urls = if phase == "edit" {
            vec![
                web_url(&format!("{base}/one"))?,
                web_url(&format!("{base}/two"))?,
            ]
        } else {
            vec![]
        };
        ui.resume(&urls)?;
        wait("fixture page", || {
            ui.snapshot().is_ok_and(|s| s.current_visit().is_some())
                && !ui.pages.selected_page().native.is_loading()
                && chrome(&ui, "canopy.state().surface").is_ok_and(|v| v == "reading")
        })?;
        match phase.as_str() {
            "edit" => changes(&ui, directory)?,
            "restart" => restart(&ui)?,
            "hand-edit" => hand_edit(&ui)?,
            _ => anyhow::bail!("unknown shortcut smoke phase {phase}"),
        }
        ui.window.close();
        return Ok(());
    }
    let profile = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}", listener.local_addr()?);
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            if http::request(&mut stream).is_err() {
                continue;
            }
            let body = r#"<!doctype html><title>Shortcut editor fixture</title>
              <button id="fill" onclick="setTimeout(() => document.body.requestFullscreen(), 2200)">Fullscreen</button>
              <script>window.keys = []; document.addEventListener('keydown', e => keys.push([e.key, e.ctrlKey]));</script>"#;
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    for phase in ["edit", "restart", "hand-edit"] {
        if phase == "hand-edit" {
            fs::write(
                profile.path().join("shortcuts"),
                "forest = Ctrl+j\nreload = Ctrl+Hyperspace\nfind = F8\n",
            )?;
        }
        let status = Command::new(std::env::current_exe()?)
            .arg(phase)
            .arg(profile.path())
            .arg(&base)
            .status()?;
        ensure!(status.success(), "shortcut editor phase {phase} failed");
    }
    Ok(())
}

fn open_editor(ui: &Ui) -> Result<()> {
    open_settings(ui)?;
    click_chrome(ui, "#menu-shortcuts")?;
    wait("editor gives focus to search", || {
        visible(ui, EDITOR) && chrome_focused(ui, SEARCH)
    })
}

fn search(ui: &Ui, query: &str) -> Result<()> {
    click_chrome(ui, SEARCH)?;
    key("ctrl+a")?;
    type_text(query)?;
    wait("search query", || {
        chrome(ui, "$('shortcut-editor-search').value").is_ok_and(|v| v == query)
    })
}

fn rows(ui: &Ui) -> Result<Value> {
    chrome(
        ui,
        "[...document.querySelectorAll('[data-shortcut-row]')].map(el => el.dataset.shortcutRow)",
    )
}

fn assert_row(ui: &Ui, row: &str, keys: &[&str]) -> Result<()> {
    assert_eq!(
        chrome(
            ui,
            &format!("[...document.querySelectorAll('{row} kbd')].map(el => el.textContent)")
        )?,
        json!(keys)
    );
    Ok(())
}

fn recording(ui: &Ui) -> bool {
    visible(ui, "[data-capturing]")
}

fn begin(ui: &Ui, control: &str) -> Result<()> {
    click_chrome(ui, control)?;
    wait("Press keys", || recording(ui))
}

fn scrim(ui: &Ui) -> Result<()> {
    let (x, y) = page_point(ui, 10, 210)?;
    click_at(x, y)
}

fn close(ui: &Ui) -> Result<()> {
    key("Escape")?;
    wait("editor closes to Settings", || {
        !visible(ui, EDITOR) && chrome_focused(ui, "#settings-button")
    })
}

fn expect_forest(ui: &Ui, combo: &str) -> Result<()> {
    ui.pages.selected_page().focus();
    key(combo)?;
    wait("rebound chord opens Forest", || visible(ui, "#forest"))?;
    key("Escape")?;
    wait("Forest closed", || !visible(ui, "#forest"))
}

fn labels(ui: &Ui, expected: &str, chip: &str) -> Result<()> {
    assert_eq!(chrome(ui, "$('forest-button').title")?, expected);
    open_settings(ui)?;
    assert_eq!(text(ui, "#menu-forest kbd"), chip);
    assert_eq!(visible(ui, "#menu-forest kbd"), !chip.is_empty());
    key("Escape")?;
    wait("menu closes", || !visible(ui, "#popover"))
}

fn changes(ui: &Ui, profile: &Path) -> Result<()> {
    let path = profile.join("shortcuts");
    open_editor(ui)?;
    assert_eq!(
        chrome(
            ui,
            "[...document.querySelectorAll('#shortcut-editor h3')].map(el => el.textContent)"
        )?,
        json!(["Navigation", "Trees", "Page", "Browser"])
    );
    key("Tab")?;
    let first = "[data-shortcut-row=search]";
    assert!(chrome_focused(ui, &format!("{first} [data-slot]")));
    key("Right")?;
    assert!(chrome_focused(ui, &format!("{first} [data-key=add]")));
    key("Down")?;
    assert!(chrome_focused(
        ui,
        "[data-shortcut-row=edit-address] [data-key=add]"
    ));
    key("Left")?;
    assert!(chrome_focused(
        ui,
        "[data-shortcut-row=edit-address] [data-slot]"
    ));
    close(ui)?;

    key("Return")?;
    wait("keyboard opens Settings", || visible(ui, "#menu-shortcuts"))?;
    key("End")?;
    for _ in 0..24 {
        if chrome_focused(ui, "#menu-shortcuts") {
            break;
        }
        key("Up")?;
    }
    assert!(chrome_focused(ui, "#menu-shortcuts"));
    key("Return")?;
    wait("keyboard opens editor", || chrome_focused(ui, SEARCH))?;
    search(ui, "reload")?;
    assert_eq!(rows(ui)?, json!(["reload"]));
    assert_row(
        ui,
        "[data-shortcut-row=reload]",
        &["Ctrl R", "Ctrl Shift R", "F5"],
    )?;
    search(ui, "ctrl e")?;
    assert_eq!(rows(ui)?, json!(["forest"]));
    search(ui, "zebra-unmatched")?;
    assert!(visible(ui, "#shortcut-editor-empty"));
    search(ui, "ctrl+3")?;
    assert_eq!(rows(ui)?, json!(["tree-digit"]));
    assert!(text(ui, "[data-shortcut-row=tree-digit]").contains("Cannot be changed"));
    key("Down")?;
    assert!(chrome_focused(ui, "[data-shortcut-row=tree-digit]"));
    search(ui, "Escape")?;
    assert!(text(ui, "[data-shortcut-row=escape]").contains("Cannot be changed"));
    assert!(!visible(ui, "[data-shortcut-row=escape] button"));
    let selected = ui.snapshot()?.current;
    key("ctrl+1")?;
    key("ctrl+e")?;
    key("ctrl+q")?;
    assert_eq!(ui.snapshot()?.current, selected);
    assert!(visible(ui, EDITOR) && !visible(ui, "#forest"));
    close(ui)?;
    key("ctrl+1")?;
    wait("fixed digit selects another tree", || {
        ui.snapshot().is_ok_and(|s| s.current != selected)
    })?;
    println!("PASS the editor opens by pointer and keyboard, contains window commands, and finds grouped and fixed rows.");

    open_editor(ui)?;
    search(ui, "forest")?;
    assert_eq!(
        chrome(ui, "document.querySelector('[data-shortcut-row=forest] [data-slot]').getAttribute('aria-label')")?,
        "Change Ctrl+E for Forest"
    );
    begin(ui, FOREST_KEY)?;
    xdo(&["keydown", "ctrl"])?;
    pump(150);
    assert!(recording(ui));
    xdo(&["keyup", "ctrl"])?;
    key("a")?;
    assert!(text(ui, FOREST).contains("Add Ctrl or Alt"));
    key("ctrl+v")?;
    assert!(text(ui, FOREST).contains("reserved for text editing"));
    ensure!(Command::new("setxkbmap")
        .args(["-layout", "fr"])
        .status()?
        .success());
    key("ctrl+shift+eacute")?;
    let fixed = text(ui, FOREST).contains("belongs to a fixed shortcut");
    ensure!(Command::new("setxkbmap")
        .args(["-layout", "us"])
        .status()?
        .success());
    assert!(fixed, "shifted AZERTY digit is reserved");
    key("ctrl+Tab")?;
    wait("switch chord conflict", || {
        text(ui, FOREST).contains("Switch to a waiting or read page.")
    })?;
    assert!(!visible(ui, "#switcher"));
    scrim(ui)?;
    assert!(visible(ui, EDITOR) && !visible(ui, "[data-shortcut-action=move-here]"));
    assert_row(ui, FOREST, &["Ctrl E"])?;
    println!("PASS modifier-only presses keep capture open, unsafe keys are refused, and recording Ctrl+Tab never opens the filmstrip.");

    begin(ui, FOREST_KEY)?;
    key("Escape")?;
    assert!(visible(ui, EDITOR) && !recording(ui));
    begin(ui, FOREST_KEY)?;
    let (x, y) = chrome_point(ui, "#shortcut-editor-title")?;
    move_to(x, y)?;
    xdo(&["mousedown", "1", "key", "ctrl+j", "mouseup", "1"])?;
    wait("pointer cancels capture before the next key", || {
        !recording(ui)
    })?;
    assert_row(ui, FOREST, &["Ctrl E"])?;
    assert!(!path.exists());
    begin(ui, FOREST_ADD)?;
    scrim(ui)?;
    assert!(visible(ui, EDITOR) && !recording(ui));
    scrim(ui)?;
    wait("second scrim press closes", || {
        !visible(ui, EDITOR) && chrome_focused(ui, "#settings-button")
    })?;
    open_editor(ui)?;
    search(ui, "forest")?;
    begin(ui, FOREST_KEY)?;
    click_chrome(ui, "#shortcut-editor-close")?;
    wait("Close abandons recording", || !visible(ui, EDITOR))?;
    println!("PASS Escape, scrim, and native pointer press cancel capture before another key can save a binding.");

    open_editor(ui)?;
    search(ui, "forest")?;
    key("Down")?;
    assert!(chrome_focused(ui, FOREST_KEY));
    xdo(&["keydown", "Return"])?;
    wait("Enter began recording", || recording(ui))?;
    pump(790);
    assert!(recording(ui) && !text(ui, FOREST).contains("Add Ctrl or Alt"));
    key("ctrl+j")?;
    xdo(&["keyup", "Return"])?;
    assert_row(ui, FOREST, &["Ctrl J"])?;
    assert_eq!(fs::read_to_string(&path)?, "forest = Ctrl+j\n");
    assert_eq!(chrome(ui, "$('forest-button').title")?, "Canopy · Ctrl+J");
    close(ui)?;
    labels(ui, "Canopy · Ctrl+J", "Ctrl J")?;
    key("ctrl+e")?;
    assert!(!visible(ui, "#forest"));
    expect_forest(ui, "ctrl+j")?;
    fullscreen(ui)?;
    println!("PASS held Enter cannot record itself, the new chord dispatches immediately, and the old chord, menu, and tooltip follow the change.");

    open_editor(ui)?;
    search(ui, "forest")?;
    begin(ui, FOREST_ADD)?;
    key("Super+F9")?;
    assert_row(ui, FOREST, &["Ctrl J", "Super F9"])?;
    close(ui)?;
    expect_forest(ui, "Super+F9")?;
    open_editor(ui)?;
    search(ui, "forest")?;
    key("Down")?;
    key("Right")?;
    key("Delete")?;
    assert_row(ui, FOREST, &["Ctrl J"])?;
    assert!(chrome_focused(ui, FOREST_KEY));
    key("BackSpace")?;
    assert!(text(ui, FOREST).contains("Unassigned"));
    assert!(chrome_focused(ui, FOREST_ADD));
    close(ui)?;
    labels(ui, "Canopy", "")?;
    println!("PASS Add records Super, Delete and Backspace remove focused chips, and clearing a row removes its labels.");

    open_editor(ui)?;
    search(ui, "forest")?;
    begin(ui, FOREST_ADD)?;
    key("F6")?;
    wait("occupied F6 names its owner", || {
        text(ui, FOREST).contains("F6 is used by Move between page and tree strip.")
    })?;
    assert_eq!(fs::read_to_string(&path)?, "forest =\n");
    key("Right")?;
    assert!(chrome_focused(ui, "[data-shortcut-action=cancel]"));
    key("Return")?;
    assert!(chrome_focused(ui, FOREST_ADD));
    begin(ui, FOREST_ADD)?;
    xdo(&["keydown", "F6"])?;
    wait("F6 conflicts with focus", || {
        visible(ui, "[data-shortcut-action=move-here]")
    })?;
    click_chrome(ui, "[data-shortcut-action=move-here]")?;
    assert_row(ui, FOREST, &["F6"])?;
    close(ui)?;
    pump(790);
    assert!(
        !visible(ui, "#forest"),
        "a held captured key repeats after close"
    );
    xdo(&["keyup", "F6"])?;
    expect_forest(ui, "F6")?;
    open_editor(ui)?;
    search(ui, "move between")?;
    assert!(text(ui, FOCUS_ROW).contains("Unassigned"));
    click_chrome(ui, &format!("{FOCUS_ROW} [data-shortcut-action=reset]"))?;
    assert_row(ui, FOCUS_ROW, &["F6"])?;
    search(ui, "forest")?;
    assert!(text(ui, FOREST).contains("Unassigned"));
    click_chrome(ui, RESET_ALL)?;
    assert!(chrome_focused(
        ui,
        "#shortcut-editor-footer [data-key=cancel]"
    ));
    key("Escape")?;
    assert!(text(ui, FOREST).contains("Unassigned"));
    click_chrome(ui, RESET_ALL)?;
    scrim(ui)?;
    assert!(visible(ui, EDITOR) && !visible(ui, "[data-shortcut-action=confirm-reset-all]"));
    click_chrome(ui, RESET_ALL)?;
    click_chrome(ui, "[data-shortcut-action=confirm-reset-all]")?;
    assert_row(ui, FOREST, &["Ctrl E"])?;
    assert_eq!(fs::read_to_string(&path)?, "");
    begin(ui, FOREST_KEY)?;
    key("ctrl+j")?;
    close(ui)?;
    println!("PASS Cancel preserves an occupied key, Move transfers it, row reset reclaims it, and Reset all needs one confirmation.");
    Ok(())
}

fn fullscreen(ui: &Ui) -> Result<()> {
    let (x, y) = element_point(ui, "fill")?;
    click_at(x, y)?;
    open_editor(ui)?;
    begin(ui, "[data-shortcut-row=search] [data-slot='0']")?;
    wait("page fullscreen", || ui.pages.is_fullscreen())?;
    key("ctrl+j")?;
    wait("fullscreen page receives rebound chord", || {
        eval(ui, "JSON.stringify(keys.at(-1))").is_ok_and(|v| v == json!(["j", true]))
    })?;
    eval(ui, "document.exitFullscreen(); JSON.stringify(true)")?;
    wait("page leaves fullscreen", || !ui.pages.is_fullscreen())?;
    assert!(!visible(ui, EDITOR) && !visible(ui, "#forest"));
    println!("PASS fullscreen dismisses active recording and receives the rebound shortcut.");
    Ok(())
}

fn restart(ui: &Ui) -> Result<()> {
    labels(ui, "Canopy · Ctrl+J", "Ctrl J")?;
    key("ctrl+e")?;
    assert!(!visible(ui, "#forest"));
    expect_forest(ui, "ctrl+j")?;
    println!("PASS a new process loads the saved shortcut and labels before input.");
    Ok(())
}

fn hand_edit(ui: &Ui) -> Result<()> {
    expect_forest(ui, "ctrl+j")?;
    key("F8")?;
    wait("hand-edited Find binding", || visible(ui, "#find-bar"))?;
    key("Escape")?;
    open_settings(ui)?;
    click_chrome(ui, "#recent-errors")?;
    let errors = text(ui, "#popover");
    assert_eq!(
        errors.matches("invalid shortcut entry on line 2").count(),
        1,
        "{errors}"
    );
    assert_eq!(
        chrome(ui, "S.errors.recent.length")?,
        1,
        "a malformed entry reports one Recent errors line"
    );
    println!("PASS both good hand-edited rows dispatch around a bad entry, which reports once in Recent errors.");
    Ok(())
}
