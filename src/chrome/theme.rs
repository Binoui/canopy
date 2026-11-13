use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path};

use anyhow::{bail, Result};
use gtk::{cairo, gdk};

const INTER: &[u8] = include_bytes!("../../assets/fonts/InterVariable.ttf");
pub(crate) const FONT: &str = "Inter Variable";

#[link(name = "fontconfig")]
extern "C" {
    fn FcConfigGetCurrent() -> *mut std::ffi::c_void;
    fn FcConfigAppFontAddFile(config: *mut std::ffi::c_void, file: *const u8) -> i32;
}

/// Registers the bundled Inter with fontconfig. GTK must not be initialized yet,
/// because Pango caches the font map on first use.
pub fn register_font(data_dir: &Path) -> Result<()> {
    let directory = data_dir.join("fonts");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("InterVariable.ttf");
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(INTER.len() as u64) {
        std::fs::write(&path, INTER)?;
    }
    let file = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: fontconfig copies the path; the current config outlives the call.
    let added = unsafe { FcConfigAppFontAddFile(FcConfigGetCurrent(), file.as_ptr().cast()) };
    if added != 1 {
        bail!("Could not load the bundled Inter font.");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shade {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Token {
    Bg,
    Chrome,
    Border,
    Text,
    Muted,
    Faint,
    Hover,
    Tile,
    TileBorder,
    Line,
    Green,
    GreenSoft,
    GreenInk,
    Scrim,
    Shadow,
}

/// The prototype's `.tv` and `.tv.dark` tokens. GTK CSS and cairo both read this table.
const TOKENS: [(Token, &str, &str, &str); 15] = [
    (Token::Bg, "bg", "#ffffff", "#0f0f0f"),
    (Token::Chrome, "chrome", "#fbfbfa", "#141414"),
    (Token::Border, "border", "#ebebea", "#252525"),
    (Token::Text, "text", "#37352f", "#ececec"),
    (Token::Muted, "muted", "#787774", "#8f8f8f"),
    (Token::Faint, "faint", "#b3b2ad", "#5c5c5c"),
    (
        Token::Hover,
        "hover",
        "rgba(55,53,47,0.06)",
        "rgba(255,255,255,0.06)",
    ),
    (Token::Tile, "tile", "#ffffff", "#1b1b1b"),
    (Token::TileBorder, "tile_border", "#e3e3e0", "#2d2d2d"),
    (Token::Line, "line", "#d2d2ce", "#3a3a3a"),
    (Token::Green, "green", "#2f9e5f", "#3fcf7d"),
    (
        Token::GreenSoft,
        "green_soft",
        "rgba(47,158,95,0.13)",
        "rgba(63,207,125,0.15)",
    ),
    (Token::GreenInk, "green_ink", "#1f7a47", "#7fe3a9"),
    (
        Token::Scrim,
        "scrim",
        "rgba(15,15,15,0.16)",
        "rgba(0,0,0,0.5)",
    ),
    (
        Token::Shadow,
        "shadow",
        "rgba(15,15,15,0.10)",
        "rgba(0,0,0,0.55)",
    ),
];

impl Shade {
    pub fn value(self, token: Token) -> &'static str {
        let (_, _, light, dark) = TOKENS.iter().find(|entry| entry.0 == token).unwrap();
        match self {
            Shade::Light => light,
            Shade::Dark => dark,
        }
    }

    pub fn rgba(self, token: Token) -> gdk::RGBA {
        gdk::RGBA::parse(self.value(token)).expect("theme tokens are valid colors")
    }

    pub fn source(self, cr: &cairo::Context, token: Token) {
        self.source_alpha(cr, token, 1.);
    }

    pub fn source_alpha(self, cr: &cairo::Context, token: Token, alpha: f64) {
        let c = self.rgba(token);
        cr.set_source_rgba(c.red(), c.green(), c.blue(), c.alpha() * alpha);
    }

    pub fn css(self) -> String {
        let mut css: String = TOKENS
            .iter()
            .map(|&(token, name, _, _)| format!("@define-color {name} {};\n", self.value(token)))
            .collect();
        css.push_str(&RULES.replace("FONT", FONT));
        css
    }
}

const RULES: &str = r#"
* { font-family: "FONT", sans-serif; text-shadow: none; -gtk-icon-shadow: none; }
#forest-view, #start-page { background: @bg; color: @text; }
.forest-toolbar { padding: 10px 14px; border-bottom: 1px solid @border; }
.forest-path { border-color: @green; background: @green_soft; }
.start-title { font-size: 26px; font-weight: 600; color: @text; }
.start-search { min-height: 46px; border: 1px solid @border; border-radius: 12px; background: @bg; color: @muted; box-shadow: 0 1px 2px alpha(@shadow, 0.3); }
.shortcut-tile { padding: 12px 4px 10px; min-width: 70px; border: none; border-radius: 12px; background: transparent; color: @muted; box-shadow: none; font-size: 12px; }
.shortcut-tile:hover { background: @hover; }
.shortcut-icon { min-width: 44px; min-height: 44px; border-radius: 12px; background: @green_soft; color: @green_ink; font-size: 20px; }
tooltip { background: @text; border: none; border-radius: 6px; }
tooltip label { color: @bg; font-size: 11.5px; }
combobox button, menu { background: @bg; color: @text; border-color: @border; box-shadow: none; }
menuitem { color: @text; }
menuitem:hover { background: @hover; }
#canopy { background: @bg; color: @text; }
button { background-image: none; outline-color: @green; }

#tabs { background: @chrome; padding: 0 6px 0 8px; }
.tab { min-height: 32px; padding: 0 11px 0 10px; margin: 0 1px; border: none; border-radius: 9px 9px 0 0;
  background: transparent; box-shadow: none; color: @muted; font-size: 12.5px; }
.tab label { color: @muted; }
.tab:hover { background: @hover; }
.tab:hover label, .tab.on label { color: @text; }
.tab.on { background: @bg; box-shadow: inset 1px 0 @border, inset -1px 0 @border, inset 0 1px @border; }
.tab.on .tab-title { font-weight: 600; }
.tab-line { background: @green; border-radius: 2px; min-height: 2px; }
.count { font-size: 10.5px; font-weight: 600; color: @green_ink; background: @green_soft; border-radius: 5px; padding: 0 5px; }
.tab .count { color: @green_ink; }
.icon-button { min-width: 30px; min-height: 30px; padding: 0; border: none; border-radius: 8px;
  background: transparent; box-shadow: none; color: @muted; }
.icon-button:hover { background: @hover; color: @text; }
.icon-button:disabled { color: @faint; }
.search-pill { min-height: 28px; min-width: 186px; padding: 0 7px 0 10px; border: 1px solid @border; border-radius: 8px;
  background: @bg; box-shadow: none; color: @muted; font-size: 12.5px; }
.search-pill label { color: @muted; }
.search-pill:hover { border-color: @line; }
.kbd { font-size: 10.5px; font-weight: 500; color: @faint; border: 1px solid @border; border-radius: 5px; padding: 0 5px; }

.node { padding: 0; min-width: 0; min-height: 0; border: 1px solid @tile_border; border-radius: 8px;
  background: @tile; box-shadow: none; }
.node.sm { border-radius: 6px; }
.node:hover { border-color: @faint; box-shadow: 0 0 0 3px @hover; }
.node.raw { border-color: transparent; background: transparent; border-radius: 4px; }
.node.raw:hover { background: @hover; }
.node.pending { border-style: dashed; border-color: @faint; background: transparent; }
.node.bud { border: none; border-radius: 9px; background: @faint; }
.node.bud.pending { background: transparent; box-shadow: inset 0 0 0 1.5px @green; }
.node.current { border-color: @green; box-shadow: 0 0 0 3px @green_soft; padding: 0 10px 0 7px; border-radius: 9px; }
.node:focus { outline-style: solid; outline-width: 2px; outline-offset: 2px; outline-color: @green; }
.pill-title { font-weight: 500; font-size: 12.5px; color: @text; }
.pill-site { font-size: 11px; color: @muted; }
.badge { min-width: 15px; min-height: 14px; padding: 0 3px; border: 1px solid @tile_border; border-radius: 7px;
  background: @chrome; box-shadow: none; color: @muted; font-size: 9px; font-weight: 600; }
.badge label { color: @muted; }
.memory-dot { min-width: 18px; min-height: 18px; padding: 0; border: none; border-radius: 9px; background: transparent; box-shadow: none; }
.memory-dot:hover { background: @hover; }

popover { background: @bg; border: 1px solid @border; border-radius: 12px; padding: 6px; box-shadow: none; }
switch { min-width: 30px; min-height: 18px; border: none; border-radius: 9px; background: @line; box-shadow: none; }
switch:checked { background: @green; }
switch slider { min-width: 14px; min-height: 14px; margin: 2px; border: none; border-radius: 7px; background: #ffffff; box-shadow: none; }
popover label { color: @text; font-size: 12.5px; }
.menu-heading { font-size: 11px; font-weight: 600; color: @muted; padding: 6px 8px 4px; }
.menu-row { padding: 6px 8px; border: none; border-radius: 8px; background: transparent; box-shadow: none; min-height: 0; }
.menu-row:hover { background: @hover; }
.menu-detail { font-size: 11.5px; color: @muted; }
.menu-error { font-size: 11.5px; color: @text; padding: 4px 8px; }
.menu-separator { background: @border; min-height: 1px; margin: 5px 4px; }

#palette { background: @scrim; }
.palette-box { background: @bg; border: 1px solid @border; border-radius: 13px; box-shadow: 0 1px 2px alpha(@shadow, 0.5), 0 10px 30px @shadow; }
.palette-context { font-size: 11.5px; color: @muted; padding: 11px 14px 0; }
.palette-tag { font-size: 10.5px; font-weight: 500; padding: 1px 6px; border-radius: 5px; background: @green_soft; color: @green_ink; }
.palette-entry { border: none; background: transparent; box-shadow: none; padding: 13px 0; font-size: 15px; color: @text; caret-color: @text; }
.palette-entry selection { background: @green_soft; color: @text; }
.palette-results { background: transparent; border-top: 1px solid @border; padding: 6px; }
.palette-results row { padding: 0; border-radius: 8px; background: transparent; }
.palette-group { font-size: 11px; font-weight: 600; color: @muted; padding: 9px 8px 5px; }
.palette-item { padding: 7px 8px; border-radius: 8px; }
.palette-item.selected { background: @hover; }
.palette-title { font-size: 13px; color: @text; }
.palette-detail { font-size: 12px; color: @muted; }
.palette-foot { border-top: 1px solid @border; padding: 8px 14px; font-size: 11px; color: @faint; }

#toast { background: @text; border-radius: 10px; padding: 9px 14px; }
#toast label { color: @bg; font-size: 12.5px; }

.card-frame { border-radius: 11px; box-shadow: 0 1px 2px alpha(@shadow, 0.5), 0 10px 30px @shadow; }
.card { background: @bg; border: 1px solid @border; border-radius: 11px; padding: 12px 12px 10px; }
.card-title { font-weight: 600; font-size: 13px; color: @text; }
.card-meta { font-size: 11.5px; color: @muted; margin-top: 3px; }
.card-snippet { font-size: 12px; color: @muted; }
.card-tag { font-size: 10.5px; font-weight: 600; padding: 1px 6px; border-radius: 5px; background: @green_soft; color: @green_ink; }
.card-action { min-height: 0; padding: 3px 8px; border: 1px solid @border; border-radius: 6px; background: transparent;
  box-shadow: none; font-size: 11.5px; color: @text; }
.card-action label { color: @text; }
.card-action:hover { background: @hover; }
.card-action:disabled label { color: @faint; }

#capture-reader { background: @bg; }
#capture-reader label { color: @text; }
#capture-reader textview text { color: @text; background: @bg; }
#capture-reader button { color: @text; background: @bg; border: 1px solid @border; box-shadow: none; border-radius: 6px; }
#capture-reader .capture-title { font-size: 20px; font-weight: 600; }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_has_a_light_and_a_dark_color() {
        for shade in [Shade::Light, Shade::Dark] {
            let css = shade.css();
            for (token, name, _, _) in TOKENS {
                assert!(css.contains(&format!("@define-color {name} {};", shade.value(token))));
                assert!(gdk::RGBA::parse(shade.value(token)).is_ok());
            }
        }
        assert_ne!(Shade::Light.value(Token::Bg), Shade::Dark.value(Token::Bg));
    }
}
