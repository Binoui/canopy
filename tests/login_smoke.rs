#[path = "common/http.rs"]
mod http;

use std::{cell::RefCell, io::Write, net::TcpListener, rc::Rc, thread};

use anyhow::{Context, Result};
use canopy::{
    address::{web_url, WebOrigin},
    browser::NavigationTarget,
    login_helper::LoginHelper,
    logins::{Done, Login, LoginId, LoginProvider, Reply, SavedLogin},
    secret::Secret,
    ui::Ui,
};
use gtk::prelude::*;
use serde_json::json;
use url::Url;
use webkit2gtk::WebViewExt;

mod common;
use common::*;

/// A provider that keeps the accounts of one site in memory, shared with the test.
struct Accounts {
    origin: WebOrigin,
    accounts: Store,
}
type Store = Rc<RefCell<Vec<(String, String)>>>;

fn store(accounts: &[(&str, &str)]) -> Store {
    Rc::new(RefCell::new(
        accounts
            .iter()
            .map(|(username, password)| ((*username).to_owned(), (*password).to_owned()))
            .collect(),
    ))
}

impl LoginProvider for Accounts {
    fn name(&self) -> String {
        "the test accounts".into()
    }

    fn for_page(&self, page: &Url, done: Done<Vec<SavedLogin>>) {
        let mut found = Vec::new();
        if WebOrigin::of(page).as_ref() == Some(&self.origin) {
            for (index, (username, _)) in self.accounts.borrow().iter().enumerate() {
                found.push(SavedLogin {
                    id: LoginId::new(index as i64),
                    origin: self.origin.clone(),
                    username: username.clone(),
                });
            }
        }
        done(Ok(Reply::Ready(found)));
    }

    fn password(&self, login: &SavedLogin) -> Result<Secret> {
        self.accounts
            .borrow()
            .iter()
            .find(|(username, _)| *username == login.username)
            .map(|(_, password)| Secret::new(password.clone()))
            .context("no such account")
    }

    fn save(&self, login: Login, done: Done<()>) {
        let mut accounts = self.accounts.borrow_mut();
        accounts.retain(|(username, _)| *username != login.username);
        accounts.push((login.username.clone(), login.password.expose().to_owned()));
        done(Ok(Reply::Ready(())));
    }
}

const WATCH: &str = "<script>window.seen=[];document.addEventListener('input',e=>seen.push(e.target.id+'='+e.target.value));</script>";

fn page(path: &str) -> String {
    let body = match path {
        "/login" => "<title>Sign in</title><form><input id=search placeholder=Search></form><form method=post action=/welcome><input id=user name=user><input id=pass type=password name=pass><button id=send>Sign in</button></form>".to_owned(),
        "/retry" => "<title>Try again</title><form method=post action=/retry><input id=user name=user><input id=pass type=password name=pass><button id=send>Sign in</button></form>".to_owned(),
        "/app" => "<title>App</title><form id=f onsubmit='return false'><input id=user><input id=pass type=password><button id=show type=button>Show</button><button id=go type=button>Sign in</button></form><script>go.onclick=()=>f.remove()</script>".to_owned(),
        "/late" => "<title>Late</title><div id=slot></div><script>setTimeout(()=>{slot.innerHTML='<form><input id=user><input id=pass type=password></form>'},600)</script>".to_owned(),
        "/signup" => "<title>Sign up</title><form><input id=user><input id=pass type=password><input id=again type=password></form>".to_owned(),
        "/first-step" => "<title>First step</title><form><input id=user type=email autocomplete=username><input id=pass type=password style=display:none></form>".to_owned(),
        "/sign-in" => "<title>Email first</title><form><input id=search name=q><input id=user type=email></form>".to_owned(),
        "/claim" => "<title>Passkey field</title><form><input id=user name=email autocomplete=webauthn></form>".to_owned(),
        "/component" => "<title>Components</title><x-field id=user kind=text></x-field><x-field id=pass kind=password></x-field><script>customElements.define('x-field',class extends HTMLElement{connectedCallback(){this.attachShadow({mode:'open'}).innerHTML=`<input type=${this.getAttribute('kind')}>`}get value(){return this.shadowRoot.querySelector('input').value}})</script>".to_owned(),
        "/newsletter" => "<title>Newsletter</title><form><input id=user type=email autocomplete=email></form>".to_owned(),
        _ => "<title>Plain</title><p>Nothing to sign in to</p>".to_owned(),
    };
    // The padding keeps the fields clear of the strip, which opens over the top of the
    // page when the pointer crosses it.
    format!("<!doctype html><body style='padding-top:160px'>{body}{WATCH}")
}

fn open(ui: &Ui, base: &Url, path: &str) -> Result<()> {
    let url = base.join(path)?;
    // A new tree loads out of sight while the old page stays on screen, and the old page
    // can be on this very address. Mark it, so only a new document passes.
    eval(ui, "globalThis.__leaving = true; 'null'")?;
    ui.pages
        .selected_page()
        .navigate(url.clone(), NavigationTarget::Root)?;
    // The view names the new address before the old document is gone, so ask the document.
    let loaded = json!(format!("{url} complete false"));
    wait(&format!("{path} loads"), || {
        !ui.pages.selected_page().native.is_loading()
            && eval(
                ui,
                "JSON.stringify(`${location.href} ${document.readyState} ${globalThis.__leaving === true}`)",
            )
            .is_ok_and(|state| state == loaded)
    })
}

fn offered(ui: &Ui) -> serde_json::Value {
    chrome(ui, "canopy.state().logins").unwrap_or_default()
}

/// What the page's own fields hold, and the input events its scripts saw.
fn fields(ui: &Ui) -> Result<serde_json::Value> {
    eval(
        ui,
        "JSON.stringify({user:document.getElementById('user')?.value??null,pass:document.getElementById('pass')?.value??null,again:document.getElementById('again')?.value??null,search:document.getElementById('search')?.value??null,seen})",
    )
}

fn main() -> Result<()> {
    let _runtime = common::private_runtime()?;
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = web_url(&format!("http://{}/", listener.local_addr()?))?;
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            // One thread a connection: the browser also opens connections it sends nothing
            // on, and waiting for one of those must not hold up the next request.
            thread::spawn(move || {
                // Read the whole request: closing on an unread form body resets the
                // connection and the browser never sees the answer.
                let Ok(request) = http::request(&mut stream) else {
                    return;
                };
                let text = String::from_utf8_lossy(&request);
                let path = text.split(' ').nth(1).unwrap_or("/");
                let body = page(path);
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            });
        }
    });
    let origin = WebOrigin::of(&base).unwrap();
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.set_login_provider(Rc::new(Accounts {
        origin: origin.clone(),
        accounts: store(&[("alice", "fake-alice")]),
    }));

    // A second origin holds its response until both fill paths have been checked.
    open(&ui, &base, "/login")?;
    wait("saved login before pending switch", || {
        offered(&ui) == json!(["alice"])
    })?;
    let pending = TcpListener::bind("127.0.0.1:0")?;
    let pending_url = web_url(&format!("http://{}/login", pending.local_addr()?))?;
    let (arrived, hit) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = pending.accept() {
            let _ = http::request(&mut stream);
            let _ = arrived.send(());
            let _ = gate.recv();
            let body = page("/login");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    ui.pages.open_roots(std::slice::from_ref(&pending_url))?;
    wait("other origin is pending", || hit.try_recv().is_ok())?;
    click_chrome(&ui, "#login-chip")?;
    wait(
        "click fills the visible page during a pending switch",
        || fields(&ui).is_ok_and(|f| f["pass"] == "fake-alice"),
    )?;
    eval(&ui, "user.value='';pass.value='';JSON.stringify(true)")?;
    key("ctrl+shift+l")?;
    wait(
        "shortcut fills the visible page during a pending switch",
        || fields(&ui).is_ok_and(|f| f["pass"] == "fake-alice"),
    )?;
    release.send(())?;
    wait("other origin lands", || {
        ui.pages
            .selected_page()
            .native
            .uri()
            .is_some_and(|u| u == pending_url.as_str())
            && !ui.pages.selected_page().native.is_loading()
    })?;
    assert_eq!(
        fields(&ui)?["pass"],
        "",
        "incoming origin must receive no saved password"
    );
    println!("PASS saved login fills only the visible page while another origin is pending.");

    open(&ui, &base, "/plain")?;
    pump(700);
    assert_eq!(offered(&ui), json!([]));
    assert!(!visible(&ui, "#login-chip"));
    println!("PASS a page with no sign-in fields offers no login.");

    open(&ui, &base, "/login")?;
    wait("the saved login is offered", || {
        offered(&ui) == json!(["alice"])
    })?;
    assert!(visible(&ui, "#login-chip"));
    assert_eq!(text(&ui, "#login-chip"), "alice");
    if let Ok(dir) = std::env::var("CANOPY_LOGIN_SHOTS") {
        save_window(&ui, &format!("{dir}/login-chip.png"))?;
    }
    assert_eq!(
        fields(&ui)?["pass"],
        "",
        "Nothing is filled before the user asks"
    );
    click_chrome(&ui, "#login-chip")?;
    wait("the login is filled", || {
        fields(&ui).is_ok_and(|f| f["pass"] == "fake-alice")
    })?;
    let filled = fields(&ui)?;
    assert_eq!(filled["user"], "alice");
    assert_eq!(filled["search"], "", "The search box is not the user name");
    assert_eq!(filled["seen"], json!(["user=alice", "pass=fake-alice"]));
    wait("the page has the keyboard after a fill", || {
        ui.pages.selected_page().native.has_focus()
    })?;
    assert_eq!(
        eval(&ui, "JSON.stringify(document.activeElement.id)")?,
        "pass"
    );
    println!("PASS one click fills the saved login and the page's scripts see it.");

    open(&ui, &base, "/plain")?;
    wait("the offer leaves with the page", || {
        offered(&ui) == json!([])
    })?;
    assert!(!visible(&ui, "#login-chip"));
    println!("PASS leaving the sign-in page takes the offer away.");

    for path in ["/signup", "/newsletter"] {
        open(&ui, &base, path)?;
        pump(700);
        assert_eq!(offered(&ui), json!([]), "{path}");
    }
    println!("PASS sign-up and newsletter forms are not offered a login.");

    open(&ui, &base, "/first-step")?;
    wait("the first step is offered", || {
        offered(&ui) == json!(["alice"])
    })?;
    ui.pages.selected_page().focus();
    key("ctrl+shift+l")?;
    wait("the user name is filled", || {
        fields(&ui).is_ok_and(|f| f["user"] == "alice")
    })?;
    assert_eq!(
        fields(&ui)?["pass"],
        "",
        "A hidden password field stays empty"
    );
    println!("PASS Ctrl+Shift+L fills the user name of a two-step sign-in.");

    for path in ["/sign-in", "/claim"] {
        open(&ui, &base, path)?;
        wait(&format!("{path} is offered"), || {
            offered(&ui) == json!(["alice"])
        })?;
        click_chrome(&ui, "#login-chip")?;
        wait(&format!("{path} is filled"), || {
            fields(&ui).is_ok_and(|f| f["user"] == "alice")
        })?;
        assert_ne!(fields(&ui)?["search"], "alice");
    }
    println!("PASS an email-first sign-in address and a passkey field take the user name.");

    open(&ui, &base, "/component")?;
    wait("fields inside components are offered", || {
        offered(&ui) == json!(["alice"])
    })?;
    click_chrome(&ui, "#login-chip")?;
    wait("fields inside components are filled", || {
        fields(&ui).is_ok_and(|f| f["user"] == "alice" && f["pass"] == "fake-alice")
    })?;
    assert_eq!(
        fields(&ui)?["seen"],
        json!(["user=alice", "pass=fake-alice"]),
        "The page hears input events from inside its components"
    );
    println!("PASS sign-in fields inside web components are found and filled.");

    ui.set_login_provider(Rc::new(Accounts {
        origin: origin.clone(),
        accounts: store(&[("alice", "fake-alice"), ("bob", "fake-bob")]),
    }));
    open(&ui, &base, "/late")?;
    wait("both accounts are offered for a form added later", || {
        offered(&ui) == json!(["alice", "bob"])
    })?;
    assert_eq!(text(&ui, "#login-chip"), "2 logins");
    ui.pages.selected_page().focus();
    key("ctrl+shift+l")?;
    wait("the list of logins", || {
        visible(&ui, "#popover [data-login='1']")
    })?;
    assert!(chrome_focused(&ui, "[data-login='0']"));
    assert_eq!(fields(&ui)?["pass"], "", "Opening the list fills nothing");
    if let Ok(dir) = std::env::var("CANOPY_LOGIN_SHOTS") {
        save_window(&ui, &format!("{dir}/login-list.png"))?;
    }
    click_chrome(&ui, "#popover [data-login='1']")?;
    wait("the chosen login is filled", || {
        fields(&ui).is_ok_and(|f| f["pass"] == "fake-bob")
    })?;
    assert_eq!(fields(&ui)?["user"], "bob");
    assert!(!visible(&ui, "#popover"));
    println!("PASS several saved logins open a list, and the chosen one is filled.");

    // A choice made on an offer that was since replaced fills nothing.
    let stale = chrome(&ui, "S.logins.offer")?;
    open(&ui, &base, "/login")?;
    wait("a new offer", || {
        offered(&ui) == json!(["alice", "bob"])
            && chrome(&ui, "S.logins.offer").is_ok_and(|o| o != stale)
    })?;
    ui.chrome.evaluate(&format!(
        "window.ipc.postMessage(JSON.stringify({{t:'intent',kind:'fill-login',offer:{stale},index:0}}))"
    ))?;
    wait("the stale choice is refused", || {
        toast_text(&ui).is_some_and(|t| t.contains("no longer offered"))
    })?;
    assert_eq!(fields(&ui)?["pass"], "");
    println!("PASS a choice from a replaced offer fills nothing.");

    // Settling the last tree leaves the start page, which has nothing to sign in to.
    assert_eq!(offered(&ui), json!(["alice", "bob"]));
    let forest = canopy::forest::ForestProjection::from_snapshot(
        ui.pages.read(|browser| browser.snapshot())?,
        ui.pages.read(|browser| browser.tree_states())?,
    )?;
    for tree in forest.open() {
        ui.pages.settle_tree(tree.id)?;
    }
    wait("the offer leaves with the last tree", || {
        offered(&ui) == json!([])
    })?;
    println!("PASS settling the last tree takes the offer away.");

    saving(&ui, &base, origin)?;
    helper(&ui, &base, directory.path())?;
    ui.window.close();
    Ok(())
}

/// Puts a login in the page's form and presses its button, as a person would: WebKit
/// drops a second scripted submission to the same address.
fn sign_in(ui: &Ui, base: &Url, path: &str, username: &str, password: &str) -> Result<()> {
    open(ui, base, path)?;
    eval(
        ui,
        &format!("document.getElementById('user').value='{username}';document.getElementById('pass').value='{password}';'null'"),
    )?;
    let (x, y) = element_point(ui, "send")?;
    click_at(x, y)?;
    wait("the form is sent", || {
        let native = &ui.pages.selected_page().native;
        !native.is_loading() && fields(ui).is_ok_and(|f| f["pass"] != password)
    })
}

fn save_offer(ui: &Ui) -> serde_json::Value {
    chrome(ui, "canopy.state().save").unwrap_or_default()
}

fn saving(ui: &Ui, base: &Url, origin: WebOrigin) -> Result<()> {
    let accounts = store(&[("alice", "fake-alice")]);
    ui.set_login_provider(Rc::new(Accounts {
        origin,
        accounts: accounts.clone(),
    }));
    let host = base.host_str().unwrap();

    sign_in(ui, base, "/login", "carol", "fake-carol")?;
    wait("a new login is offered for saving", || {
        save_offer(ui)["username"] == "carol"
    })?;
    let offer = save_offer(ui);
    assert_eq!(offer["update"], false);
    assert_eq!(offer["site"], host);
    assert_eq!(text(ui, "#login-chip"), "Save login?");
    assert_eq!(
        accounts.borrow().len(),
        1,
        "Nothing is saved before the user says so"
    );
    click_chrome(ui, "#login-chip")?;
    wait("the save question", || {
        visible(ui, "#popover [data-save='1']")
    })?;
    assert_eq!(text(ui, "#save-login-account .grow"), "carol");
    if let Ok(dir) = std::env::var("CANOPY_LOGIN_SHOTS") {
        save_window(ui, &format!("{dir}/login-save.png"))?;
    }
    click_chrome(ui, "#popover [data-save='1']")?;
    wait("the login is saved", || accounts.borrow().len() == 2)?;
    assert_eq!(
        accounts.borrow()[1],
        ("carol".to_owned(), "fake-carol".to_owned())
    );
    wait("the offer is answered", || save_offer(ui).is_null())?;
    assert!(!visible(ui, "#popover") && !visible(ui, "#login-chip"));
    println!("PASS a new login is saved once the user agrees.");

    sign_in(ui, base, "/login", "carol", "fake-carol")?;
    pump(900);
    assert!(save_offer(ui).is_null());
    println!("PASS a login saved as it is brings no offer.");

    sign_in(ui, base, "/login", "carol", "fake-changed")?;
    wait("a changed password is offered as an update", || {
        save_offer(ui)["update"] == true
    })?;
    assert_eq!(text(ui, "#login-chip"), "Update password?");
    click_chrome(ui, "#login-chip")?;
    wait("the update question", || {
        visible(ui, "#popover [data-save='0']")
    })?;
    click_chrome(ui, "#popover [data-save='0']")?;
    wait("the offer is declined", || save_offer(ui).is_null())?;
    assert_eq!(accounts.borrow()[1].1, "fake-carol");
    sign_in(ui, base, "/login", "carol", "fake-changed")?;
    wait("the update is offered again", || {
        save_offer(ui)["update"] == true
    })?;
    click_chrome(ui, "#login-chip")?;
    wait("the update question", || {
        visible(ui, "#popover [data-save='1']")
    })?;
    assert_eq!(text(ui, "#popover [data-save='1']"), "Update");
    click_chrome(ui, "#popover [data-save='1']")?;
    wait("the password is updated", || {
        accounts
            .borrow()
            .iter()
            .any(|(u, p)| u == "carol" && p == "fake-changed")
    })?;
    assert_eq!(accounts.borrow().len(), 2);
    println!("PASS a changed password is offered as an update, and Not now keeps the old one.");

    sign_in(ui, base, "/retry", "dave", "fake-wrong")?;
    pump(1200);
    assert!(save_offer(ui).is_null(), "The sign-in page came back");
    println!("PASS a login the site sends back to its sign-in page is not offered.");

    open(ui, base, "/app")?;
    eval(
        ui,
        "document.getElementById('user').value='erin';document.getElementById('pass').value='fake-erin';'null'",
    )?;
    let (x, y) = element_point(ui, "show")?;
    click_at(x, y)?;
    pump(1200);
    assert!(save_offer(ui).is_null(), "The form is still there");
    let (x, y) = element_point(ui, "go")?;
    click_at(x, y)?;
    wait("a login sent without leaving the page is offered", || {
        save_offer(ui)["username"] == "erin"
    })?;
    click_chrome(ui, "#login-chip")?;
    wait("the save question", || {
        visible(ui, "#popover [data-save='1']")
    })?;
    click_chrome(ui, "#popover [data-save='1']")?;
    wait("the login is saved", || {
        accounts
            .borrow()
            .iter()
            .any(|(u, p)| u == "erin" && p == "fake-erin")
    })?;
    println!("PASS a login sent by a page that stays is offered once its form goes away.");
    Ok(())
}

/// A password manager's helper that keeps its accounts in a file next to itself and
/// answers only once unlocked.
const HELPER: &str = r#"#!/usr/bin/env python3
import json, os, sys
verb, request = sys.argv[1], json.load(sys.stdin)
kept = os.path.join(os.path.dirname(os.path.abspath(__file__)), "kept.json")
logins = json.load(open(kept))
if verb == "unlock":
    answer = {"state": "open-sesame"}
elif request.get("state") != "open-sesame":
    answer = {"locked": True}
elif verb == "get":
    answer = {"error": "asked about " + request["url"]} if request["url"].endswith("/retry") else {"logins": logins}
else:
    logins = [l for l in logins if l["username"] != request["username"]]
    logins.append({"username": request["username"], "password": request["password"]})
    json.dump(logins, open(kept, "w"))
    answer = {}
print(json.dumps(answer))
"#;

fn helper(ui: &Ui, base: &Url, directory: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let program = directory.join("canopy-login-test");
    let kept = directory.join("kept.json");
    std::fs::write(&program, HELPER)?;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(&kept, r#"[{"username": "zoe", "password": "fake-zoe"}]"#)?;
    let found = LoginHelper::find(program.to_str().context("a readable path")?)?;
    assert_eq!(found.name(), "Test");
    assert!(LoginHelper::find("no-such-helper-here").is_err());
    ui.set_login_provider(Rc::new(found));

    open(ui, base, "/login")?;
    wait("the locked manager asks to be unlocked", || {
        chrome(ui, "canopy.state().locked").is_ok_and(|keeper| keeper == "Test")
    })?;
    assert_eq!(text(ui, "#login-chip"), "Unlock Test");
    assert_eq!(offered(ui), json!([]));
    if let Ok(dir) = std::env::var("CANOPY_LOGIN_SHOTS") {
        save_window(ui, &format!("{dir}/login-unlock.png"))?;
    }
    click_chrome(ui, "#login-chip")?;
    wait("the unlocked manager offers its account", || {
        offered(ui) == json!(["zoe"])
    })?;
    assert!(chrome(ui, "canopy.state().locked")?.is_null());
    assert_eq!(text(ui, "#login-chip"), "zoe");
    click_chrome(ui, "#login-chip")?;
    wait("the manager's login is filled", || {
        fields(ui).is_ok_and(|f| f["user"] == "zoe" && f["pass"] == "fake-zoe")
    })?;
    println!("PASS a helper's manager is unlocked from the key button, then fills its login.");

    sign_in(ui, base, "/login", "yan", "fake-yan")?;
    wait("the manager is offered the new login", || {
        save_offer(ui)["username"] == "yan"
    })?;
    assert_eq!(save_offer(ui)["keeper"], "Test");
    click_chrome(ui, "#login-chip")?;
    wait("the save question", || {
        visible(ui, "#popover [data-save='1']")
    })?;
    assert_eq!(text(ui, "#popover .menu-hint"), "Kept in Test.");
    click_chrome(ui, "#popover [data-save='1']")?;
    wait("the helper stores the login", || {
        std::fs::read_to_string(&kept).is_ok_and(|kept| kept.contains("fake-yan"))
    })?;
    wait("the save is confirmed", || {
        toast_text(ui).is_some_and(|toast| toast == "Login saved in Test.")
    })?;
    println!("PASS a new login is stored through the helper.");

    open(ui, base, "/retry")?;
    wait("the helper's error is shown", || {
        toast_text(ui).is_some_and(|toast| toast.starts_with("Test: asked about "))
    })?;
    assert_eq!(offered(ui), json!([]));
    println!("PASS a helper's error is shown and offers nothing.");
    Ok(())
}
