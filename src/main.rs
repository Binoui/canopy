use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use gtk::prelude::*;

const HELP: &str = "Canopy

Usage: canopy [--data-dir PATH]

Ctrl+K    Search your forest or go to an address
Ctrl+L    Same, starting from the current address
Ctrl+T    New tree
Ctrl+1…9  Switch to a tree
F6        Move between the page and the tree strip
Alt+←     Back to the parent page
Ctrl+R    Reload the page
Ctrl+S    Remember the selection
Ctrl+Q    Quit

Your profile and browsing are kept in XDG_DATA_HOME/canopy.";

fn main() {
    if let Err(error) = run() {
        eprintln!("Canopy: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut data_dir = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => {
                data_dir = Some(PathBuf::from(
                    args.next().context("--data-dir expects a path.")?,
                ))
            }
            Some("--help" | "-h") => {
                println!("{HELP}");
                return Ok(());
            }
            _ => bail!("Unknown option: {}. Use --help.", arg.to_string_lossy()),
        }
    }
    let data_dir = match data_dir {
        Some(path) => path,
        None => std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .context("Set XDG_DATA_HOME or use --data-dir.")?
            .join("canopy"),
    };
    let new_directory = !data_dir.exists();
    std::fs::create_dir_all(&data_dir)?;
    if new_directory {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    canopy::ui::register_font(&data_dir)?;
    gtk::init()
        .context("Could not open a window. Start Canopy from your Linux desktop session.")?;
    let ui = canopy::ui::Ui::new(&data_dir)?;
    ui.window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });
    ui.window.show_all();
    ui.resume()?;
    gtk::main();
    Ok(())
}
