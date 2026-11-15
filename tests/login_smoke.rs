use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use anyhow::Result;
use canopy::{
    address::{web_url, WebOrigin},
    browser::NavigationTarget,
    logins::{LoginId, LoginProvider, LoginsError, SavedLogin},
    secret::Secret,
    ui::Ui,
};
use gtk::prelude::*;
use serde_json::json;
use url::Url;
use webkit2gtk::WebViewExt;

mod common;
use common::*;

/// A provider with a fixed list of accounts for one site.
struct Accounts {
    origin: WebOrigin,
    accounts: Vec<(&'static str, &'static str)>,
}

impl LoginProvider for Accounts {
    fn for_page(&self, page: &Url) -> Result<Vec<SavedLogin>, LoginsError> {
        if WebOrigin::of(page).as_ref() != Some(&self.origin) {
            return Ok(Vec::new());
        }
        Ok(self
            .accounts
            .iter()
            .enumerate()
            .map(|(index, (username, _))| SavedLogin {
                id: LoginId::new(index as i64),
                origin: self.origin.clone(),
                username: (*username).to_owned(),
            })
            .collect())
    }

    fn password(&self, login: &SavedLogin) -> Result<Secret, LoginsError> {
        self.accounts
            .iter()
            .find(|(username, _)| *username == login.username)
            .map(|(_, password)| Secret::new((*password).to_owned()))
            .ok_or(LoginsError::Corrupt)
    }
}

const WATCH: &str = "<script>window.seen=[];document.addEventListener('input',e=>seen.push(e.target.id+'='+e.target.value));</script>";

fn page(path: &str) -> String {
    let body = match path {
        "/login" => "<title>Sign in</title><form><input id=search placeholder=Search></form><form><input id=user name=user><input id=pass type=password name=pass></form>".to_owned(),
        "/late" => "<title>Late</title><div id=slot></div><script>setTimeout(()=>{slot.innerHTML='<form><input id=user><input id=pass type=password></form>'},600)</script>".to_owned(),
        "/signup" => "<title>Sign up</title><form><input id=user><input id=pass type=password><input id=again type=password></form>".to_owned(),
        "/first-step" => "<title>First step</title><form><input id=user type=email autocomplete=username><input id=pass type=password style=display:none></form>".to_owned(),
        "/sign-in" => "<title>Email first</title><form><input id=search name=q><input id=user type=email></form>".to_owned(),
        "/claim" => "<title>Passkey field</title><form><input id=user name=email autocomplete=webauthn></form>".to_owned(),
        "/component" => "<title>Components</title><x-field id=user kind=text></x-field><x-field id=pass kind=password></x-field><script>customElements.define('x-field',class extends HTMLElement{connectedCallback(){this.attachShadow({mode:'open'}).innerHTML=`<input type=${this.getAttribute('kind')}>`}get value(){return this.shadowRoot.querySelector('input').value}})</script>".to_owned(),
        "/newsletter" => "<title>Newsletter</title><form><input id=user type=email autocomplete=email></form>".to_owned(),
        _ => "<title>Plain</title><p>Nothing to sign in to</p>".to_owned(),
    };
    format!("<!doctype html>{body}{WATCH}")
}

fn open(ui: &Ui, base: &Url, path: &str) -> Result<()> {
    let url = base.join(path)?;
    ui.pages
        .selected_page()
        .navigate(url.clone(), NavigationTarget::Root)?;
    wait(&format!("{path} loads"), || {
        let native = &ui.pages.selected_page().native;
        native.uri().as_deref() == Some(url.as_str()) && !native.is_loading()
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
    let directory = tempfile::tempdir()?;
    start(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = web_url(&format!("http://{}/", listener.local_addr()?))?;
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0; 4096];
            let size = stream.read(&mut request).unwrap_or(0);
            let text = String::from_utf8_lossy(&request[..size]);
            let path = text.split(' ').nth(1).unwrap_or("/");
            let body = page(path);
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let origin = WebOrigin::of(&base).unwrap();
    let ui = Ui::new(directory.path())?;
    ui.window.show_all();
    ready(&ui)?;
    ui.set_login_provider(Box::new(Accounts {
        origin: origin.clone(),
        accounts: vec![("alice", "fake-alice")],
    }));

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

    ui.set_login_provider(Box::new(Accounts {
        origin,
        accounts: vec![("alice", "fake-alice"), ("bob", "fake-bob")],
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

    ui.window.close();
    Ok(())
}
