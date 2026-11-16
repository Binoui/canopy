//! Throwaway prototype for Canopy on macOS. See README.md in this folder.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
mod selftest;
mod server;

fn main() {
    #[cfg(target_os = "macos")]
    app::run();
    #[cfg(not(target_os = "macos"))]
    {
        let site = server::Site::start().expect("the local test site starts");
        eprintln!("This prototype only runs on macOS. Test site: {} ({} hits)", site.url("/p/1"), site.hits("/p/1"));
    }
}
