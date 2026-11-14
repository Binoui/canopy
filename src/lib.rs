pub mod address;
pub mod browser;
pub mod download;
pub mod forest;
pub mod import;
pub mod instance;
pub mod naming;

#[cfg(feature = "desktop")]
pub mod chrome;
#[cfg(feature = "desktop")]
pub mod engine;
#[cfg(feature = "desktop")]
pub mod ui;

pub mod cookies;
mod key_store;
pub mod logins;
pub mod secret;

/// Initializes WebKit's sandbox policy and GTK. Call before starting other threads.
#[cfg(feature = "desktop")]
pub fn init_desktop() -> anyhow::Result<()> {
    use anyhow::Context;

    // wry creates subprocesses before exposing its native contexts.
    std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    gtk::init().context("Could not open a window. Start Canopy from your Linux desktop session.")
}
