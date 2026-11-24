//! WebKitGTK as Canopy's page engine.

mod context;
mod view;

pub(crate) use context::WebKitEngine;
#[cfg(feature = "gui-smoke")]
pub(crate) use view::webkit_view_for_tests;
pub(crate) use view::{entry_id, entry_id_of_raw, temporary_webkit_view};
