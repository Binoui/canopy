use anyhow::Result;
use canopy::{
    browser::{Browser, ThemeMode},
    chrome::{ChromeHost, ChromeIntent},
    ui::Ui,
};
use gtk::prelude::*;
mod common;
use common::*;

fn main() -> Result<()> {
    let _runtime = private_runtime()?;
    let dir = tempfile::tempdir()?;
    let mut b = Browser::open(&dir.path().join("forest.sqlite3"))?;
    let roots = b.add_roots(
        &(0..8)
            .map(|i| url::Url::parse(&format!("http://fixture.localhost:9/{i}")).unwrap())
            .collect::<Vec<_>>(),
    )?;
    for (i, id) in roots.iter().enumerate() {
        let tree = b
            .tree_states()?
            .into_iter()
            .find(|s| s.root.root() == *id)
            .unwrap()
            .root;
        b.rename_tree(tree, &format!("Project {i}"))?;
        if i >= 4 {
            b.settle_tree(tree)?;
        }
        if i == 0 || i == 4 {
            b.set_pinned(tree, true)?;
        }
    }
    drop(b);
    start(dir.path())?;
    let ui = Ui::new(dir.path())?;
    ui.window.resize(1240, 800);
    ui.window.show_all();
    ready(&ui)?;
    key("ctrl+e")?;
    wait("Trees", || visible(&ui, "#forest-trees .tree-card"))?;
    assert_eq!(
        text(&ui, "#forest-count"),
        "4 open · 2 saved · 3 in history"
    );
    assert_eq!(
        chrome(
            &ui,
            "[...document.querySelectorAll('#forest-trees .tree-card')].map(c=>c.dataset.id)"
        )?,
        serde_json::json!(
            [roots[0], roots[1], roots[2], roots[3], roots[0], roots[4]].map(|id| id.to_string())
        )
    );
    assert!(text(&ui, "#forest-trees").contains("Saved"));
    for width in [1240, 2560, 600] {
        ui.window.resize(width, 800);
        pump(200);
        assert_eq!(chrome(&ui,"(()=>{const r=$('forest').querySelector('.forest-pop').getBoundingClientRect();return [r.left,innerWidth-r.right]})()")?,serde_json::json!([22,22]));
        assert!(rect(&ui, "#forest-trees .tree-card")?.2 <= 240.);
    }
    ui.window.resize(1240, 800);
    pump(200);
    click_chrome(&ui, "#forest-views [data-view=history]")?;
    wait("History", || visible(&ui, ".forest-row"))?;
    assert_eq!(
        chrome(&ui, "document.querySelectorAll('.forest-row').length")?,
        3
    );
    assert!(
        chrome(
            &ui,
            "document.querySelector('.forest-row').getAttribute('aria-selected') === 'true'"
        )? == true
    );
    key("Tab")?;
    assert!(chrome(&ui, "!!document.activeElement.closest('#forest')")? == true);
    key("Escape")?;
    wait("closed", || !visible(&ui, "#forest"))?;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        ui.dispatch(ChromeIntent::SetTheme(mode));
        pump(100);
    }
    println!("PASS forest toolbar totals, ordered Open/Saved bands, bounded cards, width, history membership and focus containment.");
    ui.window.close();
    Ok(())
}
