//! WebKitGTK as Canopy's page engine.

mod context;
mod events;
mod rules;
mod view;

pub(crate) use context::WebKitEngine;
#[cfg(feature = "gui-smoke")]
pub(crate) use view::webkit_view_for_tests;

/// WebKit's own view type, for the GUI test accessors only.
#[cfg(feature = "gui-smoke")]
pub type NativeView = webkit2gtk::WebView;
