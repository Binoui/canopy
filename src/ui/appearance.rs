//! Whether the desktop prefers dark, and telling web pages which appearance Canopy shows.
//!
//! WebKitGTK 3 answers `prefers-color-scheme` from GTK's `gtk-application-prefer-dark-theme`
//! and the theme name only. Desktops now say dark through the settings portal's
//! `color-scheme`, which GTK 3 never reads, so pages saw light on a dark desktop.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// The desktop's light or dark preference, kept current.
pub struct Desktop {
    /// The portal's `color-scheme`: 0 no preference, 1 dark, 2 light. `None` without a portal.
    scheme: Cell<Option<u32>>,
    /// GTK's prefer-dark flag as the desktop set it, read before Canopy overrides it.
    gtk_prefer_dark: bool,
    _proxy: Option<gio::DBusProxy>,
}

impl Desktop {
    /// Reads the preference now and calls `changed` whenever the portal reports a new one.
    pub fn watch(changed: impl Fn() + 'static) -> Rc<Self> {
        let gtk_prefer_dark =
            gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme());
        let proxy = gio::DBusProxy::for_bus_sync(
            gio::BusType::Session,
            gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES,
            None,
            PORTAL,
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            None::<&gio::Cancellable>,
        )
        .ok();
        let scheme = proxy.as_ref().and_then(read_scheme);
        let desktop = Rc::new(Desktop {
            scheme: Cell::new(scheme),
            gtk_prefer_dark,
            _proxy: proxy.clone(),
        });
        if let Some(proxy) = proxy {
            let weak = Rc::downgrade(&desktop);
            proxy.connect_local("g-signal", false, move |args| {
                let name = args.get(2).and_then(|v| v.get::<String>().ok());
                let params = args.get(3).and_then(|v| v.get::<glib::Variant>().ok());
                if let (Some(desktop), Some("SettingChanged"), Some(params)) =
                    (weak.upgrade(), name.as_deref(), params)
                {
                    if let Some((namespace, key, value)) =
                        params.get::<(String, String, glib::Variant)>()
                    {
                        if namespace == NAMESPACE && key == KEY {
                            desktop.scheme.set(unwrap_u32(&value));
                            changed();
                        }
                    }
                }
                None
            });
        }
        desktop
    }

    /// Whether `Theme: System` resolves to dark.
    pub fn prefers_dark(&self) -> bool {
        match self.scheme.get() {
            Some(1) => true,
            Some(2) => false,
            _ => {
                self.gtk_prefer_dark
                    || gtk::Settings::default()
                        .and_then(|s| s.gtk_theme_name())
                        .is_some_and(|name| name.to_lowercase().contains("dark"))
            }
        }
    }
}

/// Makes web pages report `prefers-color-scheme: dark` when `dark`, light otherwise.
pub fn show_pages(dark: bool) {
    if let Some(settings) = gtk::Settings::default() {
        if settings.is_gtk_application_prefer_dark_theme() != dark {
            settings.set_gtk_application_prefer_dark_theme(dark);
        }
    }
}

fn read_scheme(proxy: &gio::DBusProxy) -> Option<u32> {
    let call = |method| {
        proxy
            .call_sync(
                method,
                Some(&(NAMESPACE, KEY).to_variant()),
                gio::DBusCallFlags::NONE,
                500,
                None::<&gio::Cancellable>,
            )
            .ok()
    };
    // `ReadOne` is the current call; older portals only have `Read`, which wraps once more.
    let reply = call("ReadOne").or_else(|| call("Read"))?;
    unwrap_u32(&reply.child_value(0))
}

/// The number inside `value`, through any layers of variant boxing.
fn unwrap_u32(value: &glib::Variant) -> Option<u32> {
    if value.is_type(glib::VariantTy::VARIANT) {
        value.as_variant().and_then(|inner| unwrap_u32(&inner))
    } else {
        value.get::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_portal_values() {
        assert_eq!(unwrap_u32(&1u32.to_variant()), Some(1));
        let boxed = glib::Variant::from_variant(&2u32.to_variant());
        assert_eq!(unwrap_u32(&glib::Variant::from_variant(&boxed)), Some(2));
        assert_eq!(unwrap_u32(&"dark".to_variant()), None);
    }
}
