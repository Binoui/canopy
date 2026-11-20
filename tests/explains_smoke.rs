use anyhow::Result;
use canopy::{
    browser::ThemeMode,
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
use common::*;

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    wait("first-run help", || visible(&ui, "#trees-help"))?;
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(200);
        assert!(text(&ui, "#trees-help").contains("waits as a tab after the current page"));
        assert!(text(&ui, "#trees-help").contains("Ctrl+Tab"));
        assert!(text(&ui, "#trees-help").contains("Ctrl+Shift+T"));
        assert!(text(&ui, "#trees-help").contains("still open"));
        assert_eq!(
            chrome(&ui, "document.querySelectorAll('.help-legend-mark').length")?,
            5
        );
        assert_eq!(chrome(&ui, "(() => { const r=$('trees-help').getBoundingClientRect(); return r.bottom < $('start-content').getBoundingClientRect().top; })()")?, true);
    }
    assert_eq!(chrome(&ui, "(() => { const r=$('trees-help').getBoundingClientRect(), s=$('start-search').getBoundingClientRect(); return r.x === s.x && r.width === s.width; })()")?, true);
    assert!(text(&ui, ".help-legend-group:last-child").contains("read on other branches"));
    click_chrome(&ui, "#trees-help-dismiss")?;
    wait("help dismissed", || !visible(&ui, "#trees-help"))?;
    assert!(!ui.pages.read(|b| b.trees_help())?);
    ui.window.close();
    pump(100);
    drop(ui);
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    assert!(!visible(&ui, "#trees-help"));
    click_chrome(&ui, "#settings-button")?;
    click_chrome(&ui, "#show-trees-help")?;
    wait("Settings reopens help", || visible(&ui, "#trees-help"))?;
    click_chrome(&ui, "#trees-help-dismiss")?;
    key("F1")?;
    wait("F1 reopens help", || visible(&ui, "#trees-help"))?;
    println!("PASS fresh data shows help above pins and plain and waiting branch counts in both themes; dismissal survives restart; Settings and F1 reopen it.");
    ui.window.close();
    Ok(())
}
