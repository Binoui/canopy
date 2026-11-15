//! Saved logins in the window: which accounts the page is offered, filling the one the
//! user picks, and offering to keep a login the user signed in with.

use std::{
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use url::Url;

use super::Ui;
use crate::{
    chrome::{SaveOffer, Toast},
    login_helper::{self, LoginHelper},
    logins::{Login, LoginProvider, Logins, Reply, SavedLogin},
    pages::LoginEvent,
};

/// The provider the user chose: the helper named in the data folder, or the vault on this
/// computer. A helper that cannot be found falls back to the vault, with the reason.
pub(super) fn provider(data_dir: &Path) -> (Option<Rc<dyn LoginProvider>>, Option<String>) {
    let mut problem = None;
    if let Some(name) = login_helper::configured(data_dir) {
        match LoginHelper::find(&name) {
            Ok(helper) => return (Some(Rc::new(helper)), None),
            Err(error) => {
                problem = Some(format!(
                    "{error} Saved logins come from the vault on this computer for now."
                ))
            }
        }
    }
    // Opening the vault reads no secret; the keyring is only asked when a login is filled.
    match Logins::open(data_dir) {
        Ok(vault) => (Some(Rc::new(vault)), problem),
        Err(error) => {
            eprintln!("Canopy: saved logins are unavailable. {error}");
            (None, problem)
        }
    }
}

impl Ui {
    /// Reads saved logins from `provider` from now on.
    pub fn set_login_provider(&self, provider: Rc<dyn LoginProvider>) {
        *self.logins.borrow_mut() = Some(provider);
        self.offer_logins(None);
    }

    fn login_provider(&self) -> Option<Rc<dyn LoginProvider>> {
        self.logins.borrow().clone()
    }

    pub(super) fn login_event(&self, event: LoginEvent) {
        match event {
            LoginEvent::NewDocument => {
                self.save_watch.borrow_mut().new_document();
                self.offer_logins(None);
            }
            LoginEvent::Fields {
                page,
                username,
                password,
                asks_password,
            } => {
                self.offer_logins((username || password).then_some(&page));
                let accepted = self
                    .save_watch
                    .borrow_mut()
                    .fields(asks_password, Instant::now());
                if let Some(login) = accepted {
                    self.offer_save(login);
                }
            }
            LoginEvent::Sent(login) => self.save_watch.borrow_mut().sent(login, Instant::now()),
        }
    }

    /// Looks up the accounts saved for `page`, which shows sign-in fields, and offers them
    /// when the provider answers. `None` takes the offer away.
    fn offer_logins(&self, page: Option<&Url>) {
        let lookup = self.lookups.get() + 1;
        self.lookups.set(lookup);
        *self.login_page.borrow_mut() = page.cloned();
        let (Some(page), Some(provider)) = (page, self.login_provider()) else {
            self.show_logins(Vec::new(), false);
            return;
        };
        let weak = self.me.clone();
        provider.for_page(
            page,
            Box::new(move |reply| {
                let Some(ui) = weak.upgrade().filter(|ui| ui.lookups.get() == lookup) else {
                    return;
                };
                match reply {
                    Ok(Reply::Ready(logins)) => ui.show_logins(logins, false),
                    Ok(Reply::Locked) => ui.show_logins(Vec::new(), true),
                    Err(error) => {
                        ui.show_logins(Vec::new(), false);
                        ui.report_error(&error.to_string());
                    }
                }
            }),
        );
    }

    /// Puts these accounts, or the request to unlock the provider, on the key button.
    fn show_logins(&self, logins: Vec<SavedLogin>, locked: bool) {
        if self.logins_locked.replace(locked) != locked {
            let name = self.login_provider().map(|provider| provider.name());
            self.chrome
                .logins_locked(name.as_deref().filter(|_| locked));
        }
        let offer = {
            let mut offered = self.offered.borrow_mut();
            if logins.is_empty() && offered.1.is_empty() {
                return;
            }
            *offered = (offered.0 + 1, logins);
            offered.0
        };
        let offered = self.offered.borrow();
        let names: Vec<&str> = offered.1.iter().map(|l| l.username.as_str()).collect();
        self.chrome.offer_logins(offer, &names);
    }

    /// Asks the provider to unlock, then looks the page's accounts up again.
    pub(super) fn unlock_logins(&self) {
        let Some(provider) = self.login_provider() else {
            return;
        };
        let weak = self.me.clone();
        provider.unlock(Box::new(move |unlocked| {
            let Some(ui) = weak.upgrade() else { return };
            match unlocked {
                Ok(true) => {
                    let page = ui.login_page.borrow().clone();
                    ui.offer_logins(page.as_ref());
                }
                Ok(false) => {}
                Err(error) => ui.report_error(&error.to_string()),
            }
        }));
    }

    /// Fills the account the user picked from offer number `offer`.
    pub(super) fn fill_login(&self, offer: u64, index: usize) -> Result<()> {
        let login = {
            let offered = self.offered.borrow();
            (offered.0 == offer)
                .then(|| offered.1.get(index).cloned())
                .flatten()
                .context("That login is no longer offered for this page.")?
        };
        let password = self
            .login_provider()
            .context("Saved logins are unavailable.")?
            .password(&login)?;
        let weak = self.me.clone();
        self.pages
            .fill_login(&login.origin, &login.username, &password, move |result| {
                if let (Some(ui), Err(error)) = (weak.upgrade(), result) {
                    ui.report_error(&error.to_string());
                }
            })
    }

    /// Offers to keep a login the page accepted, unless it is already saved as it is.
    /// A locked provider cannot say, so it brings no offer.
    fn offer_save(&self, login: Login) {
        let (Some(provider), Ok(site)) = (self.login_provider(), Url::parse(login.origin.as_str()))
        else {
            return;
        };
        let weak = self.me.clone();
        let asked = provider.clone();
        provider.for_page(
            &site,
            Box::new(move |reply| {
                if let (Some(ui), Ok(Reply::Ready(saved))) = (weak.upgrade(), reply) {
                    ui.offer_unsaved(&*asked, login, &saved);
                }
            }),
        );
    }

    fn offer_unsaved(&self, provider: &dyn LoginProvider, mut login: Login, saved: &[SavedLogin]) {
        let Some(update) = unsaved(provider, &mut login, saved) else {
            return;
        };
        let id = self.save_offers.get() + 1;
        self.save_offers.set(id);
        let offer = SaveOffer {
            id,
            site: Url::parse(login.origin.as_str())
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default(),
            username: login.username.clone(),
            update,
            keeper: provider.name(),
        };
        *self.saving.borrow_mut() = Some((id, login));
        self.chrome.offer_save(Some(&offer));
        // An unanswered offer does not keep the password in memory for the whole session.
        let weak = self.me.clone();
        glib::timeout_add_local_once(Duration::from_secs(300), move || {
            if let Some(ui) = weak.upgrade() {
                let waiting = ui
                    .saving
                    .borrow_mut()
                    .take_if(|(waiting, _)| *waiting == id);
                if waiting.is_some() {
                    ui.chrome.offer_save(None);
                }
            }
        });
    }

    /// Keeps, or lets go of, the login of save offer number `offer`.
    pub(super) fn answer_save(&self, offer: u64, save: bool) -> Result<()> {
        let (_, login) = self
            .saving
            .borrow_mut()
            .take_if(|(waiting, _)| *waiting == offer)
            .context("That login is no longer waiting to be saved.")?;
        self.chrome.offer_save(None);
        if !save {
            return Ok(());
        }
        let provider = self
            .login_provider()
            .context("Saved logins are unavailable.")?;
        let name = provider.name();
        let weak = self.me.clone();
        provider.save(
            login,
            Box::new(move |reply| {
                let Some(ui) = weak.upgrade() else { return };
                match reply {
                    Ok(Reply::Ready(())) => ui
                        .chrome
                        .toast(Toast::Info(format!("Login saved in {name}."))),
                    Ok(Reply::Locked) => ui.report_error(&format!(
                        "{name} is locked, so the login was not saved. Unlock it and sign in again."
                    )),
                    Err(error) => ui.report_error(&error.to_string()),
                }
            }),
        );
        Ok(())
    }
}

/// Whether `login` differs from what is saved: `Some(true)` for a new password on a saved
/// account, `Some(false)` for a new account, `None` when there is nothing to save. A login
/// sent without a user name belongs to the site's only saved account.
fn unsaved(provider: &dyn LoginProvider, login: &mut Login, saved: &[SavedLogin]) -> Option<bool> {
    if login.username.is_empty() {
        match saved {
            [] => {}
            [only] => login.username.clone_from(&only.username),
            _ => return None,
        }
    }
    let mut update = false;
    for account in saved.iter().filter(|s| s.username == login.username) {
        // A password that cannot be read cannot be compared; saving would fail too.
        if provider.password(account).ok()?.expose() == login.password.expose() {
            return None;
        }
        update = true;
    }
    Some(update)
}
