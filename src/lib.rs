pub mod address;
pub mod browser;
pub mod budget;
pub mod download;
pub mod forest;
pub mod import;
pub mod instance;
pub mod naming;
pub mod suggest;

#[cfg(feature = "desktop")]
pub mod chrome;
#[cfg(feature = "desktop")]
pub mod login_helper;
#[cfg(feature = "desktop")]
pub mod pages;
#[cfg(feature = "desktop")]
pub mod ui;

pub mod cookies;
mod key_store;
pub mod logins;
pub mod secret;

/// NVIDIA's decoders, which cannot start inside the sandbox. Left at their normal rank,
/// some of them fail without letting a software decoder take over (AV1 does).
#[cfg(feature = "desktop")]
const NVIDIA_DECODERS_OFF: &str = "nvh264dec:NONE,nvh265dec:NONE,nvav1dec:NONE,nvvp9dec:NONE,\
     nvvp8dec:NONE,nvmpeg2videodec:NONE,nvmpeg4videodec:NONE,nvmpegvideodec:NONE,nvjpegdec:NONE";

/// With NVIDIA's driver, a sandboxed page decodes video in software, and GStreamer then
/// hands those frames to the graphics card through a path the driver rejects. The video
/// plays with sound and paints solid green. Measured with WebKitGTK 2.54.2, GStreamer
/// 1.28.8 and driver 615.78. Both switches are read by the page process, so they must be
/// set before it starts. A value the user already set wins.
#[cfg(feature = "desktop")]
fn keep_video_colours_on_nvidia() {
    if !std::path::Path::new("/dev/nvidiactl").exists() {
        return;
    }
    for (name, value) in [
        ("GST_PLUGIN_FEATURE_RANK", NVIDIA_DECODERS_OFF),
        // Frames reach WebKit as plain memory instead of through GStreamer's OpenGL upload.
        ("WEBKIT_GST_DISABLE_GL_SINK", "1"),
    ] {
        if std::env::var_os(name).is_none() {
            std::env::set_var(name, value);
        }
    }
}

/// Initializes WebKit's sandbox policy and GTK. Call before starting other threads.
#[cfg(feature = "desktop")]
pub fn init_desktop() -> anyhow::Result<()> {
    use anyhow::Context;

    budget::Budget::of_machine()?;
    // wry creates subprocesses before exposing its native contexts.
    std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    keep_video_colours_on_nvidia();
    gtk::init().context("Could not open a window. Start Canopy from your Linux desktop session.")
}
