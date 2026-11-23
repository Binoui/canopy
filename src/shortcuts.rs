use crate::{
    chrome::{ForestView, PaletteSeed},
    pages::PageScaleAction,
};
use gdk::keys::{constants as key, Key};

/// Window-level keys, handled before the focused web view sees them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shortcut {
    Palette(PaletteSeed),
    NewTree,
    Bookmark,
    SettleTree,
    UndoSettle,
    Forest(ForestView),
    ZoomTree,
    Switch,
    MoveTree { next: bool },
    PreviousTree,
    PageScale(PageScaleAction),
    Reload,
    RememberSelection,
    FillLogin,
    PastePlainText,
    Quit,
    Tree(usize),
    Back,
    Forward,
    PreviousSibling,
    NextSibling,
    ToggleFocus,
    Find,
    FindNext,
    FindPrevious,
    TreesHelp,
    Escape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Modifiers(gdk::ModifierType);

impl Modifiers {
    const NONE: Self = Self(gdk::ModifierType::empty());
    const CONTROL: Self = Self(gdk::ModifierType::CONTROL_MASK);
    const ALT: Self = Self(gdk::ModifierType::MOD1_MASK);
    const SHIFT: Self = Self(gdk::ModifierType::SHIFT_MASK);
    const CONTROL_SHIFT: Self =
        Self(gdk::ModifierType::CONTROL_MASK.union(gdk::ModifierType::SHIFT_MASK));

    fn held(state: gdk::ModifierType) -> Self {
        use gdk::ModifierType as M;
        Self(
            state
                & (M::CONTROL_MASK
                    | M::MOD1_MASK
                    | M::SHIFT_MASK
                    | M::MOD3_MASK
                    | M::MOD4_MASK
                    | M::MOD5_MASK
                    | M::SUPER_MASK
                    | M::HYPER_MASK
                    | M::META_MASK),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeyCombo {
    held: Modifiers,
    key: Key,
}

impl KeyCombo {
    pub(crate) fn from_event(event: &gdk::EventKey) -> Self {
        Self::new(Modifiers::held(event.state()), event.keyval())
    }

    fn new(held: Modifiers, key: Key) -> Self {
        Self {
            held,
            key: key.to_lower(),
        }
    }

    fn tree_digit(self, held: [Modifiers; 2]) -> Option<usize> {
        if !held.contains(&self.held) {
            return None;
        }
        // GDK maps the number row, keypad and Unicode keyvals to the same digit.
        let digit = self.key.to_unicode()?.to_digit(10)?;
        (1..=9).contains(&digit).then(|| digit as usize - 1)
    }
}

#[derive(Clone, Copy)]
enum Defaults {
    Exact {
        action: Shortcut,
        combos: &'static [KeyCombo],
    },
    TreeDigits([Modifiers; 2]),
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Names and groups are catalog metadata")
)]
struct Command {
    id: CommandId,
    stable_id: &'static str,
    name: &'static str,
    group: Group,
    defaults: Defaults,
}

enum Group {
    Navigation,
    Trees,
    Page,
    Browser,
}

#[cfg(test)]
impl Group {
    fn label(&self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::Trees => "Trees",
            Self::Page => "Page",
            Self::Browser => "Browser",
        }
    }
}

macro_rules! commands {
    ($($id:ident, $stable_id:literal, $name:literal, $group:ident, $kind:ident $args:tt;)*) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(crate) enum CommandId { $($id),* }

        const COMMANDS: &[Command] = &[$(Command {
            id: CommandId::$id,
            stable_id: $stable_id,
            name: $name,
            group: Group::$group,
            defaults: commands!(@defaults $kind $args),
        }),*];
    };
    (@defaults exact ($action:expr; $($held:ident + $key:ident),+)) => {
        Defaults::Exact {
            action: $action,
            combos: &[$(KeyCombo { held: Modifiers::$held, key: key::$key }),+],
        }
    };
    (@defaults digits ($first:ident, $second:ident)) => {
        Defaults::TreeDigits([Modifiers::$first, Modifiers::$second])
    };
}

commands! {
    Search, "search", "Search or go to an address", Navigation, exact(Shortcut::Palette(PaletteSeed::Empty); CONTROL+k);
    Address, "edit-address", "Edit the current address", Navigation, exact(Shortcut::Palette(PaletteSeed::CurrentAddress); CONTROL+l);
    Back, "back", "Back to parent page", Navigation, exact(Shortcut::Back; ALT+Left);
    Forward, "forward", "Forward to last opened child", Navigation, exact(Shortcut::Forward; ALT+Right);
    PreviousSibling, "previous-sibling", "Previous sibling page", Navigation, exact(Shortcut::PreviousSibling; ALT+Up);
    NextSibling, "next-sibling", "Next sibling page", Navigation, exact(Shortcut::NextSibling; ALT+Down);
    Switch, "switch-page", "Switch to a waiting or read page", Navigation, exact(Shortcut::Switch; CONTROL+Tab);

    NewTree, "new-tree", "New tree", Trees, exact(Shortcut::NewTree; CONTROL+t);
    Settle, "settle-tree", "Settle tree or close overlay", Trees, exact(Shortcut::SettleTree; CONTROL+w);
    UndoSettle, "undo-settle", "Undo settling a tree or branch", Trees, exact(Shortcut::UndoSettle; CONTROL_SHIFT+t);
    Forest, "forest", "Forest", Trees, exact(Shortcut::Forest(ForestView::Canopy); CONTROL+e);
    ForestHistory, "forest-history", "Forest history", Trees, exact(Shortcut::Forest(ForestView::History); CONTROL+h);
    TreeWorkbench, "tree-workbench", "Current-tree workbench", Trees, exact(Shortcut::ZoomTree; CONTROL_SHIFT+e);
    MoveTreeLeft, "move-tree-left", "Move tree left", Trees, exact(Shortcut::MoveTree { next: false }; CONTROL_SHIFT+Page_Up);
    MoveTreeRight, "move-tree-right", "Move tree right", Trees, exact(Shortcut::MoveTree { next: true }; CONTROL_SHIFT+Page_Down);
    PreviousTree, "previous-tree", "Switch to the previously shown tree", Trees, exact(Shortcut::PreviousTree; CONTROL_SHIFT+Tab, CONTROL_SHIFT+ISO_Left_Tab);
    TreeDigit, "tree-digit", "Switch to a tree", Trees, digits(CONTROL, CONTROL_SHIFT);

    Reload, "reload", "Reload", Page, exact(Shortcut::Reload; CONTROL+r, CONTROL_SHIFT+r, NONE+F5);
    Bookmark, "bookmark", "Toggle bookmark", Page, exact(Shortcut::Bookmark; CONTROL+d);
    Remember, "remember-selection", "Remember selection", Page, exact(Shortcut::RememberSelection; CONTROL+s);
    Find, "find", "Find in page", Page, exact(Shortcut::Find; CONTROL+f);
    FindNext, "find-next", "Find next match", Page, exact(Shortcut::FindNext; CONTROL+g, NONE+F3);
    FindPrevious, "find-previous", "Find previous match", Page, exact(Shortcut::FindPrevious; CONTROL_SHIFT+g, SHIFT+F3);
    Increase, "page-size-increase", "Increase page size", Page, exact(Shortcut::PageScale(PageScaleAction::Increase);
        CONTROL+plus, CONTROL+equal, CONTROL_SHIFT+plus, CONTROL_SHIFT+equal, CONTROL+KP_Equal, CONTROL+KP_Add, CONTROL_SHIFT+KP_Add);
    Decrease, "page-size-decrease", "Decrease page size", Page, exact(Shortcut::PageScale(PageScaleAction::Decrease); CONTROL+minus, CONTROL+KP_Subtract);
    ResetSize, "page-size-reset", "Reset page size", Page, exact(Shortcut::PageScale(PageScaleAction::Reset); CONTROL+_0, CONTROL+KP_0, CONTROL_SHIFT+_0, CONTROL+agrave);
    FillLogin, "fill-login", "Fill saved login", Page, exact(Shortcut::FillLogin; CONTROL_SHIFT+l);
    PastePlain, "paste-plain-text", "Paste as plain text", Page, exact(Shortcut::PastePlainText; CONTROL_SHIFT+v);

    Help, "trees-help", "How trees work", Browser, exact(Shortcut::TreesHelp; NONE+F1);
    ToggleFocus, "toggle-focus", "Move between page and tree strip", Browser, exact(Shortcut::ToggleFocus; NONE+F6);
    Quit, "quit", "Quit", Browser, exact(Shortcut::Quit; CONTROL+q);
    Escape, "escape", "Close or stop", Browser, exact(Shortcut::Escape; NONE+Escape);
}

pub(crate) struct Bindings {
    rows: &'static [Command],
}

impl Bindings {
    pub(crate) fn defaults() -> Self {
        Self { rows: COMMANDS }
    }

    pub(crate) fn resolve(&self, combo: KeyCombo) -> Option<Shortcut> {
        self.rows.iter().find_map(|row| match row.defaults {
            Defaults::Exact { action, combos } => combos.contains(&combo).then_some(action),
            Defaults::TreeDigits(held) => combo.tree_digit(held).map(Shortcut::Tree),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdk::ModifierType as M;

    fn resolve(held: Modifiers, key: Key) -> Option<Shortcut> {
        Bindings::defaults().resolve(KeyCombo::new(held, key))
    }

    #[test]
    fn window_shortcuts_require_exact_modifiers() {
        let bindings = [
            (
                key::equal,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::plus,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::plus,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::equal,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::KP_Equal,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::KP_Add,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::KP_Add,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Increase),
            ),
            (
                key::minus,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Decrease),
            ),
            (
                key::KP_Subtract,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Decrease),
            ),
            (
                key::_0,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
            (
                key::KP_0,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
            (
                key::_0,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
            (
                key::agrave,
                Modifiers::CONTROL,
                Shortcut::PageScale(PageScaleAction::Reset),
            ),
            (
                key::k,
                Modifiers::CONTROL,
                Shortcut::Palette(PaletteSeed::Empty),
            ),
            (
                key::l,
                Modifiers::CONTROL,
                Shortcut::Palette(PaletteSeed::CurrentAddress),
            ),
            (key::t, Modifiers::CONTROL, Shortcut::NewTree),
            (key::t, Modifiers::CONTROL_SHIFT, Shortcut::UndoSettle),
            (key::d, Modifiers::CONTROL, Shortcut::Bookmark),
            (key::w, Modifiers::CONTROL, Shortcut::SettleTree),
            (
                key::e,
                Modifiers::CONTROL,
                Shortcut::Forest(ForestView::Canopy),
            ),
            (
                key::h,
                Modifiers::CONTROL,
                Shortcut::Forest(ForestView::History),
            ),
            (key::e, Modifiers::CONTROL_SHIFT, Shortcut::ZoomTree),
            (
                key::Page_Up,
                Modifiers::CONTROL_SHIFT,
                Shortcut::MoveTree { next: false },
            ),
            (
                key::Page_Down,
                Modifiers::CONTROL_SHIFT,
                Shortcut::MoveTree { next: true },
            ),
            (key::Tab, Modifiers::CONTROL, Shortcut::Switch),
            (key::Tab, Modifiers::CONTROL_SHIFT, Shortcut::PreviousTree),
            (
                key::ISO_Left_Tab,
                Modifiers::CONTROL_SHIFT,
                Shortcut::PreviousTree,
            ),
            (key::r, Modifiers::CONTROL, Shortcut::Reload),
            (key::r, Modifiers::CONTROL_SHIFT, Shortcut::Reload),
            (key::F5, Modifiers::NONE, Shortcut::Reload),
            (key::s, Modifiers::CONTROL, Shortcut::RememberSelection),
            (key::l, Modifiers::CONTROL_SHIFT, Shortcut::FillLogin),
            (key::v, Modifiers::CONTROL_SHIFT, Shortcut::PastePlainText),
            (key::q, Modifiers::CONTROL, Shortcut::Quit),
            (key::Left, Modifiers::ALT, Shortcut::Back),
            (key::Right, Modifiers::ALT, Shortcut::Forward),
            (key::Up, Modifiers::ALT, Shortcut::PreviousSibling),
            (key::Down, Modifiers::ALT, Shortcut::NextSibling),
            (key::F1, Modifiers::NONE, Shortcut::TreesHelp),
            (key::F6, Modifiers::NONE, Shortcut::ToggleFocus),
            (key::f, Modifiers::CONTROL, Shortcut::Find),
            (key::g, Modifiers::CONTROL, Shortcut::FindNext),
            (key::g, Modifiers::CONTROL_SHIFT, Shortcut::FindPrevious),
            (key::F3, Modifiers::NONE, Shortcut::FindNext),
            (key::F3, Modifiers::SHIFT, Shortcut::FindPrevious),
            (key::Escape, Modifiers::NONE, Shortcut::Escape),
        ];
        let exact_defaults = COMMANDS
            .iter()
            .map(|row| match row.defaults {
                Defaults::Exact { combos, .. } => combos.len(),
                Defaults::TreeDigits(_) => 0,
            })
            .sum::<usize>();
        assert_eq!(exact_defaults, bindings.len());

        for (key, required, _) in bindings.iter().copied() {
            for bits in 0..8 {
                let state = if bits & 1 != 0 {
                    M::SHIFT_MASK
                } else {
                    M::empty()
                } | if bits & 2 != 0 {
                    M::CONTROL_MASK
                } else {
                    M::empty()
                } | if bits & 4 != 0 {
                    M::MOD1_MASK
                } else {
                    M::empty()
                };
                let held = Modifiers::held(state);
                let expected = bindings.iter().find_map(|(bound_key, modifiers, action)| {
                    (*bound_key == key && modifiers.0 == state).then_some(*action)
                });
                assert_eq!(resolve(held, key), expected, "{state:?} {key:?}");
                assert_eq!(resolve(held, key.to_upper()), expected);
                assert_eq!(
                    resolve(Modifiers::held(state | M::LOCK_MASK | M::MOD2_MASK), key),
                    expected,
                    "lock modifiers must not affect {state:?} {key:?}"
                );
            }
            for extra in [
                M::MOD3_MASK,
                M::MOD4_MASK,
                M::MOD5_MASK,
                M::SUPER_MASK,
                M::HYPER_MASK,
                M::META_MASK,
            ] {
                assert_eq!(resolve(Modifiers::held(required.0 | extra), key), None);
            }
        }
        assert_eq!(
            resolve(Modifiers::CONTROL, key::K),
            Some(Shortcut::Palette(PaletteSeed::Empty))
        );
    }

    #[test]
    fn digit_tree_shortcuts_accept_shifted_azerty_keyvals() {
        let digits = [
            key::_1,
            key::_2,
            key::_3,
            key::_4,
            key::_5,
            key::_6,
            key::_7,
            key::_8,
            key::_9,
        ];
        let keypad = [
            key::KP_1,
            key::KP_2,
            key::KP_3,
            key::KP_4,
            key::KP_5,
            key::KP_6,
            key::KP_7,
            key::KP_8,
            key::KP_9,
        ];
        assert_eq!(Key::from(0x01000031).to_unicode(), Some('1'));
        let cases = digits
            .into_iter()
            .enumerate()
            .chain(keypad.into_iter().enumerate())
            .chain([(0, Key::from(0x01000031))]);
        for (index, key) in cases {
            for modifiers in [Modifiers::CONTROL, Modifiers::CONTROL_SHIFT] {
                assert_eq!(resolve(modifiers, key), Some(Shortcut::Tree(index)));
                assert_eq!(
                    resolve(
                        Modifiers::held(modifiers.0 | M::LOCK_MASK | M::MOD2_MASK),
                        key
                    ),
                    Some(Shortcut::Tree(index))
                );
            }
            for state in [
                M::empty(),
                M::SHIFT_MASK,
                M::CONTROL_MASK | M::MOD1_MASK,
                M::CONTROL_MASK | M::SHIFT_MASK | M::MOD1_MASK,
            ] {
                assert_eq!(resolve(Modifiers::held(state), key), None);
            }
        }
        for key in [
            key::ampersand,
            key::eacute,
            key::quotedbl,
            key::apostrophe,
            key::parenleft,
            key::egrave,
            key::underscore,
            key::ccedilla,
            key::F5,
            key::F11,
        ] {
            assert_eq!(resolve(Modifiers::CONTROL, key), None);
        }
    }

    #[test]
    fn default_catalog_has_unique_ids_combos_names_and_groups() {
        use std::collections::HashSet;

        let mut ids = HashSet::new();
        let mut stable_ids = HashSet::new();
        let mut exact = HashSet::new();
        let mut digit_family = None;
        for row in COMMANDS {
            assert!(!row.stable_id.is_empty());
            assert!(ids.insert(row.id));
            assert!(stable_ids.insert(row.stable_id));
            assert!(!row.name.trim().is_empty());
            assert!(!row.group.label().trim().is_empty());
            match row.defaults {
                Defaults::Exact { combos, action } => {
                    assert!(!combos.is_empty());
                    for &combo in combos {
                        let combo = KeyCombo::new(combo.held, combo.key);
                        assert!(exact.insert((combo.held.0.bits(), *combo.key)));
                        assert_eq!(Bindings::defaults().resolve(combo), Some(action));
                    }
                }
                Defaults::TreeDigits(held) => assert!(digit_family.replace(held).is_none()),
            }
        }
        let held = digit_family.unwrap();
        assert_ne!(held[0], held[1]);
        for (bits, key) in exact {
            let combo = KeyCombo::new(Modifiers(M::from_bits_retain(bits)), Key::from(key));
            assert!(combo.tree_digit(held).is_none());
        }
    }
}
