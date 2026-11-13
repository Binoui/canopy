use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use gtk::prelude::*;

const HELP: &str = "Canopy

Usage: canopy [--data-dir PATH]
       canopy [--data-dir PATH] --import-chrome [PROFILE]

--import-chrome reads a Chrome profile's history, bookmarks, cookies and passwords,
then exits. PROFILE defaults to ~/.config/google-chrome/Default. Running it again
refreshes pages and adds cookies and encrypted passwords. Pages are not loaded.
The system keyring protects the password vault. Page filling is not available yet.

Ctrl+K    Search your forest or go to an address
Ctrl+L    Same, starting from the current address
Ctrl+E    Forest view
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
    let args = std::env::args_os().skip(1);
    let mut data_dir = None;
    let mut import = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--import-chrome") => {
                let profile = match args.peek().and_then(|next| next.to_str()) {
                    Some(next) if !next.starts_with("--") => PathBuf::from(args.next().unwrap()),
                    _ => PathBuf::from(
                        std::env::var_os("HOME").context("Set HOME or pass a profile path.")?,
                    )
                    .join(".config/google-chrome/Default"),
                };
                import = Some(profile);
            }
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
    if let Some(profile) = import {
        return import_chrome(&data_dir, &profile);
    }
    canopy::init_desktop()?;
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

fn import_chrome(data_dir: &std::path::Path, profile: &std::path::Path) -> Result<()> {
    let chrome = canopy::import::chrome_profile(profile)?;
    let bookmarks = chrome.pages.iter().filter(|p| p.folder.is_some()).count();
    canopy::browser::Browser::open(&data_dir.join("forest.sqlite3"))?
        .replace_imported(&chrome.source, &chrome.pages)?;
    println!(
        "Imported {} pages from Chrome profile {}, {bookmarks} bookmarked.",
        chrome.pages.len(),
        chrome.name
    );
    let mut failed = false;
    match chrome.logins {
        Ok(read) => {
            let result = canopy::logins::Logins::open(data_dir)
                .and_then(|mut vault| vault.upsert_imported(&chrome.source, &read.items));
            match result {
                Ok(count) => println!(
                    "Imported {count} passwords into the encrypted vault{}.",
                    skipped(&read.skipped)
                ),
                Err(error) => {
                    failed = true;
                    println!("Passwords not imported. {error}");
                }
            }
        }
        Err(error) => {
            failed = true;
            println!("Passwords not imported. {error}");
        }
    }
    match chrome.cookies {
        Ok(read) => match canopy::cookies::Inbox::new(data_dir).post(&read.items) {
            Ok(count) => println!(
                "Imported {count} cookies{}. Canopy adds them now if it is open, or when it starts.",
                skipped(&read.skipped)
            ),
            Err(_) => { failed = true; println!("Cookies not imported. Could not post imported cookies."); }
        },
        Err(error) => { failed = true; println!("Cookies not imported. {error}"); }
    }
    if failed {
        bail!("Part of the profile was not imported. Resolve the section errors and run the command again.");
    }
    Ok(())
}

/// ", skipped 12 expired, 3 duplicate", or nothing when no row was skipped.
fn skipped(counts: &std::collections::BTreeMap<canopy::import::SkipReason, usize>) -> String {
    if counts.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(reason, count)| format!("{count} {}", reason.label()))
        .collect();
    format!(", skipped {}", parts.join(", "))
}
