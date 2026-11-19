use std::{ffi::OsString, os::fd::AsRawFd, path::PathBuf};

use anyhow::{bail, Context, Result};
use canopy::{
    address::Destination,
    instance::{self, Launch, Request},
};
use gtk::prelude::*;
use url::Url;

const HELP: &str = "Canopy

Usage: canopy [--data-dir PATH] [ADDRESS...]
       canopy [--data-dir PATH] --import-chrome [PROFILE]
       canopy [--data-dir PATH] --login-helper NAME

Each ADDRESS opens as a new tree, and the last one is shown. When Canopy is already
open on the same data directory, the addresses open in that window instead.

--import-chrome reads a Chrome profile's history, bookmarks, cookies and passwords,
then exits. PROFILE defaults to ~/.config/google-chrome/Default. Running it again
refreshes pages and adds cookies and encrypted passwords. Pages are not loaded.
The system keyring protects the password vault.

--login-helper chooses where saved logins are kept, from the next start of Canopy.
NAME is a password manager's helper, a program called canopy-login-NAME next to
Canopy or on your PATH, or the path of one. \"vault\" goes back to the vault on this
computer.

Ctrl+K          Search your forest or go to an address
Ctrl+L          Same, starting from the current address
Ctrl+E          Forest view
Ctrl+Shift+E    Current-tree workbench
Ctrl+T          New tree
Ctrl+W          Settle the tree, or close the open overlay
Ctrl+Shift+T    Reopen the most recently settled tree or branch
Ctrl+1…9        Switch to a tree
Ctrl+Shift+Tab  Switch to the previously shown open tree
Ctrl+Tab        Next waiting page, else the last read one; hold Ctrl to pick
F1              How trees work
F6              Move between the page and the tree strip
Alt+←           Back to the parent page
Alt+→           Forward to the last opened child
Alt+↑/↓         Previous or next sibling
Ctrl+R          Reload the page
Ctrl+D          Toggle the page bookmark
Ctrl+S          Remember the selection
Ctrl+Shift+L    Fill the saved login for the page
Ctrl+Q          Quit

Your profile and browsing are kept in XDG_DATA_HOME/canopy.";

fn main() {
    if let Err(error) = run() {
        eprintln!("Canopy: {error:#}");
        std::process::exit(1);
    }
}

#[derive(Debug, PartialEq)]
struct Invocation {
    data_dir: Option<PathBuf>,
    command: Command,
}

#[derive(Debug, PartialEq)]
enum Command {
    Help,
    /// `None` means the default Chrome profile.
    ImportChrome(Option<PathBuf>),
    /// `None` means the vault on this computer.
    LoginHelper(Option<String>),
    Browse(Vec<Url>),
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Invocation> {
    let mut data_dir = None;
    let mut import = None;
    let mut helper = None;
    let mut urls = Vec::new();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--import-chrome") => {
                import = Some(match args.peek().and_then(|next| next.to_str()) {
                    Some(next) if !next.starts_with("--") => args.next().map(PathBuf::from),
                    _ => None,
                });
            }
            Some("--login-helper") => {
                let name = args
                    .next()
                    .and_then(|name| name.into_string().ok())
                    .filter(|name| !name.is_empty() && !name.starts_with("--"))
                    .context("--login-helper expects a helper's name, or vault.")?;
                helper = Some((name != "vault").then_some(name));
            }
            Some("--data-dir") => {
                data_dir = Some(PathBuf::from(
                    args.next().context("--data-dir expects a path.")?,
                ))
            }
            Some("--help" | "-h") => {
                return Ok(Invocation {
                    data_dir,
                    command: Command::Help,
                })
            }
            Some(option) if option.starts_with('-') => {
                bail!("Unknown option: {option}. Use --help.")
            }
            Some(address) => match canopy::address::destination(address) {
                Ok(Destination::Address(url)) => urls.push(url),
                Ok(Destination::Search(_)) => {
                    bail!("Cannot open {address}. It is not a web address.")
                }
                Err(error) => bail!("Cannot open {address}. {error}"),
            },
            None => bail!(
                "Cannot open {}. It is not a web address.",
                arg.to_string_lossy()
            ),
        }
    }
    let command = match (import, helper) {
        (Some(_), Some(_)) => bail!("Use --import-chrome and --login-helper one at a time."),
        (Some(_), _) if !urls.is_empty() => bail!("--import-chrome does not open addresses."),
        (_, Some(_)) if !urls.is_empty() => bail!("--login-helper does not open addresses."),
        (Some(profile), None) => Command::ImportChrome(profile),
        (None, Some(helper)) => Command::LoginHelper(helper),
        (None, None) => Command::Browse(urls),
    };
    Ok(Invocation { data_dir, command })
}

fn run() -> Result<()> {
    let Invocation { data_dir, command } = parse_args(std::env::args_os().skip(1))?;
    let urls = match command {
        Command::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Command::ImportChrome(profile) => {
            let profile = match profile {
                Some(profile) => profile,
                None => PathBuf::from(
                    std::env::var_os("HOME").context("Set HOME or pass a profile path.")?,
                )
                .join(".config/google-chrome/Default"),
            };
            return import_chrome(&prepare_data_dir(data_dir)?, &profile);
        }
        Command::LoginHelper(name) => {
            return choose_login_helper(&prepare_data_dir(data_dir)?, name.as_deref())
        }
        Command::Browse(urls) => urls,
    };
    let data_dir = prepare_data_dir(data_dir)?;
    let request = Request {
        urls,
        activation: ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok().filter(|token| !token.is_empty())),
    };
    let instance = match instance::launch(&data_dir, &request)? {
        Launch::Primary(instance) => instance,
        Launch::Forwarded => return Ok(()),
    };
    canopy::init_desktop()?;
    let ui = canopy::ui::Ui::new(&data_dir)?;
    ui.window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });
    ui.window.show_all();
    ui.resume(&request.urls)?;
    glib::unix_fd_add_local(instance.as_raw_fd(), glib::IOCondition::IN, move |_, _| {
        let answered = instance.answer(|request| {
            let opened = ui.open(&request.urls);
            ui.bring_forward(request.activation.as_deref());
            opened
        });
        if let Err(error) = answered {
            eprintln!("Canopy: {error:#}");
        }
        glib::ControlFlow::Continue
    });
    gtk::main();
    Ok(())
}

/// Records where saved logins are kept. A helper that cannot be found is refused, so a
/// typing mistake does not leave Canopy without its logins.
fn choose_login_helper(data_dir: &std::path::Path, name: Option<&str>) -> Result<()> {
    use canopy::{login_helper, logins::LoginProvider};
    let keeper = match name {
        Some(name) => login_helper::LoginHelper::find(name)?.name(),
        None => "the vault on this computer".to_owned(),
    };
    login_helper::configure(data_dir, name)?;
    println!("Saved logins are kept in {keeper}, from the next start of Canopy.");
    Ok(())
}

fn prepare_data_dir(chosen: Option<PathBuf>) -> Result<PathBuf> {
    let data_dir = match chosen {
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
    Ok(data_dir)
}

fn import_chrome(data_dir: &std::path::Path, profile: &std::path::Path) -> Result<()> {
    let chrome = canopy::import::chrome_profile(profile)?;
    let added = canopy::browser::Browser::open(&data_dir.join("forest.sqlite3"))?.import_pages(
        &chrome.source,
        &chrome.pages,
        &chrome.bookmarks,
    )?;
    println!(
        "Imported {} history pages from Chrome profile {}, {added} new bookmarks.",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Invocation> {
        parse_args(args.iter().map(OsString::from))
    }

    fn urls(addresses: &[&str]) -> Command {
        Command::Browse(addresses.iter().map(|a| Url::parse(a).unwrap()).collect())
    }

    #[test]
    fn login_helper_takes_a_name_and_nothing_else() {
        let command = |args: &[&str]| parse(args).map(|invocation| invocation.command);
        assert_eq!(
            command(&["--login-helper", "bitwarden"]).unwrap(),
            Command::LoginHelper(Some("bitwarden".into()))
        );
        assert_eq!(
            command(&["--login-helper", "vault"]).unwrap(),
            Command::LoginHelper(None)
        );
        assert!(command(&["--login-helper"]).is_err());
        assert!(command(&["--login-helper", "--data-dir"]).is_err());
        assert!(command(&["--login-helper", "bitwarden", "example.org"]).is_err());
        assert!(command(&["--login-helper", "bitwarden", "--import-chrome"]).is_err());
    }

    #[test]
    fn addresses_open_in_order_with_the_chosen_data_dir() {
        assert_eq!(
            parse(&[
                "https://example.org/a",
                "--data-dir",
                "/tmp/c",
                "example.com",
                "localhost:8080/x"
            ])
            .unwrap(),
            Invocation {
                data_dir: Some("/tmp/c".into()),
                command: urls(&[
                    "https://example.org/a",
                    "https://example.com/",
                    "http://localhost:8080/x"
                ]),
            }
        );
        assert_eq!(parse(&[]).unwrap().command, urls(&[]));
    }

    #[test]
    fn shortcut_help_descriptions_align() {
        let columns: Vec<_> = HELP
            .lines()
            .filter(|line| {
                line.starts_with("Ctrl+")
                    || line.starts_with("Alt+")
                    || line.starts_with("F6 ")
                    || line.starts_with("F1 ")
            })
            .map(|line| {
                line.chars().position(|c| c == ' ').unwrap()
                    + line
                        .chars()
                        .skip_while(|c| *c != ' ')
                        .take_while(|c| *c == ' ')
                        .count()
            })
            .collect();
        assert!(
            columns.iter().all(|column| *column == 16),
            "shortcut description columns: {columns:?}"
        );
    }

    #[test]
    fn help_and_import_keep_their_meaning() {
        assert_eq!(
            parse(&["https://example.org", "-h"]).unwrap().command,
            Command::Help
        );
        assert_eq!(parse(&["--help"]).unwrap().command, Command::Help);
        assert_eq!(
            parse(&["--import-chrome"]).unwrap().command,
            Command::ImportChrome(None)
        );
        assert_eq!(
            parse(&["--import-chrome", "/p", "--data-dir", "/d"]).unwrap(),
            Invocation {
                data_dir: Some("/d".into()),
                command: Command::ImportChrome(Some("/p".into())),
            }
        );
    }

    #[test]
    fn anything_but_a_web_address_is_refused() {
        for args in [
            &["file:///etc/passwd"][..],
            &["mailto:someone@example.org"],
            &["some words"],
            &["notanaddress"],
            &["https://"],
            &["https://example.org", "javascript:alert(1)"],
            &["--private"],
            &["--data-dir"],
            &["--import-chrome", "/p", "https://example.org"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
        let error = parse(&["file:///etc/passwd"]).unwrap_err().to_string();
        assert!(error.contains("file:///etc/passwd"), "{error}");
    }
}
