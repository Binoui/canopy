//! A password manager reached through a helper program.
//!
//! Canopy runs the helper once per question, with the question as its only argument, a
//! JSON request on its input and a JSON answer on its output:
//!
//! - `get` with `url` and `origin` answers `{"logins": [{"username": "…", "password": "…"}]}`,
//!   the accounts the manager holds for that page.
//! - `store` with `url`, `origin`, `username` and `password` keeps that login and answers `{}`.
//! - `unlock` asks the user to unlock the manager, in the manager's own way, and answers
//!   `{"state": "…"}`, or `{"cancelled": true}`.
//!
//! Any answer may instead be `{"locked": true}` or `{"error": "…"}`. Every request also
//! carries `state`: the text the last `unlock` answered, or null. Canopy keeps it in memory
//! only, so a helper can hold an unlocked session without writing it anywhere.
//!
//! The helper decides which accounts match a page. Canopy asks only for pages loaded over
//! HTTPS, or from this computer.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::OsStr,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use gio::prelude::*;
use serde_json::{json, Value};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    address::WebOrigin,
    logins::{Done, Login, LoginId, LoginProvider, Reply, SavedLogin},
    secret::Secret,
};

/// Helpers are programs named `canopy-login-<name>`.
const PREFIX: &str = "canopy-login-";
/// The file in the data folder that names the chosen helper.
const SETTING: &str = "login-helper";
/// How many lookups keep their passwords at hand for a fill that follows.
const RECENT: usize = 4;
/// The most accounts one page is offered.
const MOST: usize = 20;

/// The helper named in the data folder, if the user chose one.
pub fn configured(data_dir: &Path) -> Option<String> {
    let name = std::fs::read_to_string(data_dir.join(SETTING)).ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Chooses the helper Canopy reads logins from at its next start. `None` goes back to the
/// vault on this computer.
pub fn configure(data_dir: &Path, name: Option<&str>) -> Result<()> {
    let path = data_dir.join(SETTING);
    match name {
        Some(name) => std::fs::write(path, format!("{name}\n"))?,
        None if path.exists() => std::fs::remove_file(path)?,
        None => {}
    }
    Ok(())
}

pub struct LoginHelper {
    inner: Rc<Inner>,
}

struct Inner {
    program: PathBuf,
    name: String,
    /// What the last `unlock` answered, handed back with every request.
    state: RefCell<Option<Zeroizing<String>>>,
    /// The accounts of the last few lookups, newest last.
    found: RefCell<VecDeque<Vec<Found>>>,
    next_id: Cell<i64>,
}

struct Found {
    login: SavedLogin,
    password: Secret,
}

impl LoginHelper {
    /// Finds the helper called `name`: a path, or the `<name>` of a `canopy-login-<name>`
    /// program next to Canopy or on the PATH.
    pub fn find(name: &str) -> Result<Self> {
        let command = Self::command(name);
        let program = if name.contains('/') {
            command
        } else {
            let beside = std::env::current_exe()
                .ok()
                .and_then(|exe| Some(exe.parent()?.to_owned()));
            let path = std::env::var_os("PATH").unwrap_or_default();
            beside
                .into_iter()
                .chain(std::env::split_paths(&path))
                .map(|directory| directory.join(&command))
                .find(|candidate| candidate.is_file())
                .with_context(|| {
                    format!("The login helper {name} was not found. Canopy looks for a program called {} next to itself and on your PATH.", command.display())
                })?
        };
        if !program.is_file() {
            bail!("The login helper {} was not found.", program.display());
        }
        Ok(Self {
            inner: Rc::new(Inner {
                name: Self::display_name(&program),
                program,
                state: RefCell::new(None),
                found: RefCell::default(),
                next_id: Cell::new(1),
            }),
        })
    }

    pub(crate) fn configured_display_name(name: &str) -> String {
        Self::display_name(&Self::command(name))
    }

    fn command(name: &str) -> PathBuf {
        if name.contains('/') {
            name.into()
        } else {
            format!("{PREFIX}{name}").into()
        }
    }

    fn display_name(command: &Path) -> String {
        let stem = command
            .file_stem()
            .map(|stem| stem.to_string_lossy())
            .unwrap_or_default();
        let short = stem.strip_prefix(PREFIX).unwrap_or(&stem);
        let mut letters = short.chars();
        letters
            .next()
            .map(|first| first.to_uppercase().chain(letters).collect())
            .unwrap_or_else(|| "the login helper".to_owned())
    }
}

impl Inner {
    /// Runs the helper with one question and hands its answer to `done`, on this thread,
    /// once it exits or `patience` runs out.
    fn ask(
        self: &Rc<Self>,
        verb: &str,
        mut request: Value,
        patience: Duration,
        done: impl FnOnce(Result<Value>) + 'static,
    ) {
        use gio::SubprocessFlags as Flags;
        request["state"] = match self.state.borrow().as_ref() {
            Some(state) => json!(state.as_str()),
            None => Value::Null,
        };
        let input = Zeroizing::new(request.to_string());
        forget(&mut request);
        let process = match gio::Subprocess::newv(
            &[self.program.as_os_str(), OsStr::new(verb)],
            Flags::STDIN_PIPE | Flags::STDOUT_PIPE,
        ) {
            Ok(process) => process,
            Err(error) => {
                return done(Err(anyhow!(
                    "Could not start {}'s login helper. {}",
                    self.name,
                    error.message()
                )))
            }
        };
        let done = Rc::new(RefCell::new(Some(done)));
        let cancel = gio::Cancellable::new();
        let timer = {
            let (done, cancel, process, name) = (
                done.clone(),
                cancel.clone(),
                process.clone(),
                self.name.clone(),
            );
            glib::timeout_add_local_once(patience, move || {
                if let Some(done) = done.borrow_mut().take() {
                    cancel.cancel();
                    process.force_exit();
                    done(Err(anyhow!("{name} took too long to answer.")));
                }
            })
        };
        let name = self.name.clone();
        process.communicate_utf8_async(Some(input.to_string()), Some(&cancel), move |result| {
            let Some(done) = done.borrow_mut().take() else {
                return;
            };
            timer.remove();
            done(match result {
                Ok((Some(output), _)) => answer(&name, &output),
                Ok(_) => Err(anyhow!("{name}'s login helper gave no answer.")),
                Err(error) => Err(anyhow!(
                    "{name}'s login helper stopped. {}",
                    error.message()
                )),
            });
        });
    }
}

impl LoginProvider for LoginHelper {
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    fn for_page(&self, page: &Url, done: Done<Vec<SavedLogin>>) {
        let Some(origin) = WebOrigin::of(page).filter(|_| asked_about(page)) else {
            return done(Ok(Reply::Ready(Vec::new())));
        };
        let inner = self.inner.clone();
        let request = json!({ "url": page.as_str(), "origin": origin.as_str() });
        self.inner
            .ask("get", request, Duration::from_secs(30), move |answer| {
                done(answer.map(|mut answer| {
                    if answer["locked"] == true {
                        return Reply::Locked;
                    }
                    let found = found(&answer, &origin, &inner.next_id);
                    forget(&mut answer);
                    let logins = found.iter().map(|found| found.login.clone()).collect();
                    let mut recent = inner.found.borrow_mut();
                    recent.push_back(found);
                    while recent.len() > RECENT {
                        recent.pop_front();
                    }
                    Reply::Ready(logins)
                }));
            });
    }

    fn password(&self, login: &SavedLogin) -> Result<Secret> {
        self.inner
            .found
            .borrow()
            .iter()
            .flatten()
            .find(|found| {
                found.login.id == login.id
                    && found.login.origin == login.origin
                    && found.login.username == login.username
            })
            .map(|found| Secret::new(found.password.expose().to_owned()))
            .context("That login is no longer at hand. Reload the page and try again.")
    }

    fn save(&self, login: Login, done: Done<()>) {
        let request = json!({
            "url": format!("{}/", login.origin.as_str()),
            "origin": login.origin.as_str(),
            "username": login.username,
            "password": login.password.expose(),
        });
        self.inner
            .ask("store", request, Duration::from_secs(60), move |answer| {
                done(answer.map(|answer| {
                    if answer["locked"] == true {
                        Reply::Locked
                    } else {
                        Reply::Ready(())
                    }
                }));
            });
    }

    fn unlock(&self, done: Box<dyn FnOnce(Result<bool>)>) {
        let inner = self.inner.clone();
        // The user may take a while over a master password.
        self.inner.ask(
            "unlock",
            json!({}),
            Duration::from_secs(300),
            move |answer| {
                done(answer.map(|mut answer| {
                    if answer["cancelled"] == true || answer["locked"] == true {
                        return false;
                    }
                    if let Some(state) = answer["state"].as_str() {
                        *inner.state.borrow_mut() = Some(Zeroizing::new(state.to_owned()));
                    }
                    forget(&mut answer);
                    true
                }));
            },
        );
    }
}

/// Whether the helper is asked about `page` at all: a manager matches accounts by site
/// name, so a page that anyone on the network could have rewritten is not offered one.
fn asked_about(page: &Url) -> bool {
    let local = match page.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    page.scheme() == "https" || local
}

/// The helper's answer as JSON, or the error it reports.
fn answer(name: &str, output: &str) -> Result<Value> {
    let answer: Value = serde_json::from_str(output.trim())
        .ok()
        .filter(Value::is_object)
        .with_context(|| format!("{name}'s login helper gave an answer Canopy cannot read."))?;
    if let Some(error) = answer["error"].as_str() {
        let shown: String = error.chars().take(300).collect();
        bail!("{name}: {shown}");
    }
    Ok(answer)
}

/// The accounts in a `get` answer, each given a number of its own.
fn found(answer: &Value, origin: &WebOrigin, next_id: &Cell<i64>) -> Vec<Found> {
    let Some(logins) = answer["logins"].as_array() else {
        return Vec::new();
    };
    logins
        .iter()
        .filter_map(|login| {
            let password = login["password"].as_str().filter(|p| !p.is_empty())?;
            let id = next_id.get();
            next_id.set(id + 1);
            Some(Found {
                login: SavedLogin {
                    id: LoginId::new(id),
                    origin: origin.clone(),
                    username: login["username"].as_str().unwrap_or_default().to_owned(),
                },
                password: Secret::new(password.to_owned()),
            })
        })
        .take(MOST)
        .collect()
}

/// Wipes the passwords and session text a request or an answer carried.
fn forget(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(forget),
        Value::Object(fields) => fields.values_mut().for_each(forget),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_an_object_or_the_helpers_error() {
        assert_eq!(
            answer("Test", " {\"locked\": true}\n").unwrap()["locked"],
            true
        );
        let error = answer("Test", r#"{"error": "not signed in"}"#).unwrap_err();
        assert_eq!(error.to_string(), "Test: not signed in");
        for unreadable in ["", "nope", "[]", "3"] {
            assert!(answer("Test", unreadable).is_err(), "{unreadable}");
        }
    }

    #[test]
    fn found_accounts_belong_to_the_page_and_keep_their_passwords() {
        let origin = WebOrigin::of(&Url::parse("https://example.test/").unwrap()).unwrap();
        let next = Cell::new(7);
        let answer = json!({ "logins": [
            { "username": "alice", "password": "fake-a" },
            { "username": "empty", "password": "" },
            { "password": "fake-b" },
            "junk",
        ] });
        let found = found(&answer, &origin, &next);
        let seen: Vec<_> = found
            .iter()
            .map(|f| (f.login.username.as_str(), f.password.expose()))
            .collect();
        assert_eq!(seen, [("alice", "fake-a"), ("", "fake-b")]);
        assert!(found.iter().all(|f| f.login.origin == origin));
        assert_ne!(found[0].login.id, found[1].login.id);
        assert_eq!(next.get(), 9);
        assert!(super::found(&json!({}), &origin, &next).is_empty());
    }

    #[test]
    fn only_secure_or_local_pages_are_asked_about() {
        let asked = |page: &str| asked_about(&Url::parse(page).unwrap());
        assert!(asked("https://example.test/login"));
        assert!(asked("http://localhost:8080/"));
        assert!(asked("http://127.0.0.1:8080/"));
        assert!(asked("http://[::1]/"));
        assert!(!asked("http://example.test/login"));
        assert!(!asked("http://localhost.example.test/"));
    }

    #[test]
    fn the_chosen_helper_is_remembered_in_the_data_folder() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(configured(directory.path()), None);
        configure(directory.path(), Some("bitwarden")).unwrap();
        assert_eq!(configured(directory.path()).as_deref(), Some("bitwarden"));
        configure(directory.path(), None).unwrap();
        assert_eq!(configured(directory.path()), None);
        configure(directory.path(), None).unwrap();
    }

    #[test]
    fn a_secret_carried_in_json_is_wiped() {
        let mut value = json!({ "logins": [{ "password": "fake" }], "state": "session" });
        forget(&mut value);
        assert_eq!(
            value,
            json!({ "logins": [{ "password": "" }], "state": "" })
        );
    }
}
