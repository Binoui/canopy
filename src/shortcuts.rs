mod bindings;

use std::path::Path;

use crate::{
    chrome::{ForestView, PaletteSeed},
    pages::PageScaleAction,
};
use gdk::keys::{constants as key, Key};
use gdk::ModifierType as M;

use bindings::Bindings;

const TREE_DIGITS: std::ops::RangeInclusive<u32> = 1..=9;

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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Modifiers(M);

impl Modifiers {
    const NONE: Self = Self(M::empty());
    const CONTROL: Self = Self(M::CONTROL_MASK);
    const ALT: Self = Self(M::MOD1_MASK);
    const SHIFT: Self = Self(M::SHIFT_MASK);
    const SUPER: Self = Self(M::SUPER_MASK);
    const CONTROL_SHIFT: Self = Self(M::CONTROL_MASK.union(M::SHIFT_MASK));
    const HOLD: M = M::CONTROL_MASK.union(M::MOD1_MASK).union(M::SUPER_MASK);
    const EDITABLE: M = Self::HOLD.union(M::SHIFT_MASK);
    const NAMES: [(Self, &'static str); 4] = [
        (Self::CONTROL, "Ctrl"),
        (Self::SHIFT, "Shift"),
        (Self::ALT, "Alt"),
        (Self::SUPER, "Super"),
    ];

    fn held(state: M) -> Self {
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

    fn from_keymap(state: M, keymap: Option<&gdk::Keymap>) -> Self {
        let mut held = Self::held(state);
        if let Some(keymap) = keymap.filter(|_| {
            held.0
                .intersects(M::MOD3_MASK | M::MOD4_MASK | M::MOD5_MASK)
        }) {
            let mut super_mask = M::SUPER_MASK;
            if keymap.map_virtual_modifiers(&mut super_mask) && held.0.intersects(super_mask) {
                held.0.remove(super_mask);
                held.0.insert(M::SUPER_MASK);
            }
        }
        held
    }

    pub(crate) fn are_held(self, keymap: &gdk::Keymap) -> bool {
        Self::from_keymap(M::from_bits_truncate(keymap.modifier_state()), Some(keymap))
            .0
            .contains(self.0)
    }

    pub(crate) fn released_by(self, key: Key) -> bool {
        let modifier = match key {
            key::Control_L | key::Control_R => Self::CONTROL,
            key::Alt_L | key::Alt_R => Self::ALT,
            key::Super_L | key::Super_R => Self::SUPER,
            _ => return false,
        };
        self.0.contains(modifier.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct KeyCombo {
    held: Modifiers,
    key: Key,
}

impl KeyCombo {
    pub(crate) fn from_event(event: &gdk::EventKey, keymap: Option<&gdk::Keymap>) -> Self {
        Self::new(
            Modifiers::from_keymap(event.state(), keymap),
            event.keyval(),
        )
    }

    pub(crate) fn hold(self) -> Option<Modifiers> {
        let hold = self.held.0 & Modifiers::HOLD;
        (!hold.is_empty()).then_some(Modifiers(hold))
    }

    pub(crate) fn held_tab_direction(self, hold: Modifiers) -> Option<bool> {
        match self.key {
            key::Tab if self.held == hold => Some(false),
            key::Tab | key::ISO_Left_Tab if self.held.0 == hold.0 | M::SHIFT_MASK => Some(true),
            _ => None,
        }
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
        TREE_DIGITS
            .contains(&digit)
            .then(|| (digit - TREE_DIGITS.start()) as usize)
    }
}

/// `EventKey::is_modifier` is unset for some modifier presses on X11.
pub(crate) fn is_modifier_key(value: Key) -> bool {
    matches!(
        value,
        key::Shift_L
            | key::Shift_R
            | key::Control_L
            | key::Control_R
            | key::Alt_L
            | key::Alt_R
            | key::Meta_L
            | key::Meta_R
            | key::Super_L
            | key::Super_R
            | key::Hyper_L
            | key::Hyper_R
            | key::Mode_switch
            | key::ISO_Level3_Shift
            | key::ISO_Level3_Latch
            | key::ISO_Level3_Lock
            | key::ISO_Level5_Shift
            | key::ISO_Level5_Latch
            | key::ISO_Level5_Lock
            | key::ISO_Group_Latch
            | key::ISO_Group_Lock
            | key::ISO_Lock
            | key::Shift_Lock
            | key::Caps_Lock
            | key::Num_Lock
            | key::Scroll_Lock
    )
}

#[derive(Clone, Copy)]
enum Defaults {
    Exact {
        action: Shortcut,
        combos: &'static [KeyCombo],
    },
    TreeDigits([Modifiers; 2]),
}

#[derive(Clone, Copy)]
struct Command {
    id: CommandId,
    stable_id: &'static str,
    name: &'static str,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Command group for the shortcut editor")
    )]
    group: Group,
    defaults: Defaults,
}

impl Command {
    fn editable(self) -> bool {
        !matches!(self.id, CommandId::TreeDigit | CommandId::Escape)
    }
}

#[derive(Clone, Copy)]
enum Group {
    Navigation,
    Trees,
    Page,
    Browser,
}

impl Group {
    #[cfg(test)]
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
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(usize)]
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

impl Modifiers {
    fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter_map(move |(mask, name)| self.0.contains(mask.0).then_some(name))
    }

    fn label(self) -> String {
        self.names().collect::<Vec<_>>().join(" ")
    }
}

impl KeyCombo {
    fn label(self, separator: &str) -> String {
        self.with_key_name(&self.key_name(), separator)
    }

    fn with_key_name(self, name: &str, separator: &str) -> String {
        let modifiers = self.held.names().collect::<Vec<_>>().join(separator);
        if modifiers.is_empty() {
            name.into()
        } else {
            format!("{modifiers}{separator}{name}")
        }
    }

    fn key_name(self) -> String {
        let key = self.key;
        let name = match key {
            key::Escape => "Esc".into(),
            key::minus => "−".into(),
            key::Return | key::ISO_Enter | key::KP_Enter => "Enter".into(),
            key::Tab | key::ISO_Left_Tab | key::KP_Tab => "Tab".into(),
            key::Page_Up | key::KP_Page_Up => "Page Up".into(),
            key::Page_Down | key::KP_Page_Down => "Page Down".into(),
            key::space | key::KP_Space => "Space".into(),
            key::BackSpace => "Backspace".into(),
            key::Left | key::KP_Left => "←".into(),
            key::Right | key::KP_Right => "→".into(),
            key::Up | key::KP_Up => "↑".into(),
            key::Down | key::KP_Down => "↓".into(),
            _ => match key.to_unicode().filter(|c| !c.is_control()) {
                Some(c) => c.to_uppercase().collect(),
                None => key.name().map_or_else(String::new, |name| {
                    name.strip_prefix("KP_").unwrap_or(&name).replace('_', " ")
                }),
            },
        };
        if (*key::KP_Space..=*key::KP_Equal).contains(&*key) {
            format!("Keypad {name}")
        } else {
            name
        }
    }

    pub(crate) fn switcher_footer(self, hold: Modifiers) -> SwitcherFooter {
        let local = Self::new(Modifiers::NONE, key::Tab);
        let alternate = self.held_tab_direction(hold).is_none().then(|| {
            Self {
                held: Modifiers(self.held.0 & !hold.0),
                key: self.key,
            }
            .label(" ")
        });
        SwitcherFooter {
            next: local.label(" "),
            previous: Self::new(Modifiers::SHIFT, key::Tab).label(" "),
            alternate,
            hold: hold.names().collect::<Vec<_>>().join(" or "),
        }
    }
}

impl CommandId {
    pub(crate) fn stable_id(self) -> &'static str {
        COMMANDS[self as usize].stable_id
    }

    fn command(self) -> &'static Command {
        &COMMANDS[self as usize]
    }

    fn tree_digit_modifiers() -> [Modifiers; 2] {
        let Defaults::TreeDigits(held) = CommandId::TreeDigit.command().defaults else {
            unreachable!("tree digits have a fixed row");
        };
        held
    }
}

pub(crate) struct ShortcutSettings {
    bindings: Bindings,
}

impl ShortcutSettings {
    pub(crate) fn defaults() -> Self {
        Self {
            bindings: Bindings::defaults(),
        }
    }

    pub(crate) fn load(data_dir: &Path) -> (Self, Option<String>) {
        let (bindings, problem) = Bindings::load(&data_dir.join("shortcuts"));
        (Self { bindings }, problem)
    }

    pub(crate) fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    pub(crate) fn presentation(&self) -> Presentation {
        let bindings = &self.bindings;
        let labels = COMMANDS
            .iter()
            .filter_map(|row| {
                let labels = match row.defaults {
                    Defaults::Exact { .. } => bindings.primary(row.id)?.labels(),
                    Defaults::TreeDigits([primary, _]) => Labels {
                        chip: digit_range_label(primary, " "),
                        text: digit_range_label(primary, "+"),
                    },
                };
                Some((row.stable_id, labels))
            })
            .collect();
        let [held, _] = CommandId::tree_digit_modifiers();
        let digit_labels = TREE_DIGITS
            .map(|digit| format!("{}+{digit}", held.label()))
            .collect();

        let switcher = bindings
            .primary(CommandId::Switch)
            .and_then(|combo| combo.hold().map(|hold| combo.switcher_footer(hold)));
        Presentation {
            labels,
            digit_labels,
            switcher,
        }
    }
}

fn digit_range_label(held: Modifiers, separator: &str) -> String {
    format!(
        "{}{separator}{}…{}",
        held.label(),
        TREE_DIGITS.start(),
        TREE_DIGITS.end()
    )
}

pub(crate) struct Presentation {
    pub(crate) labels: std::collections::BTreeMap<&'static str, Labels>,
    pub(crate) digit_labels: Vec<String>,
    pub(crate) switcher: Option<SwitcherFooter>,
}

impl Presentation {
    pub(crate) fn text(&self, id: CommandId) -> &str {
        self.labels
            .get(id.stable_id())
            .map_or("", |labels| &labels.text)
    }
}

pub(crate) struct Labels {
    pub(crate) chip: String,
    pub(crate) text: String,
}

impl KeyCombo {
    fn labels(self) -> Labels {
        Labels {
            chip: self.label(" "),
            text: self.label("+"),
        }
    }
}

pub(crate) struct SwitcherFooter {
    pub(crate) next: String,
    pub(crate) previous: String,
    pub(crate) alternate: Option<String>,
    pub(crate) hold: String,
}

/// The shortcut block printed by `canopy --help`.
pub fn default_help_shortcuts() -> String {
    use std::fmt::Write;

    use CommandId::*;

    let bindings = Bindings::defaults();
    let primary = |id| bindings.primary(id).expect("an exact default combination");
    let label = |id| primary(id).label("+");
    let [digit_modifiers, _] = CommandId::tree_digit_modifiers();
    let (previous, next) = (primary(PreviousSibling), primary(NextSibling));
    let switch_hold = primary(Switch).hold().unwrap().label();
    let switch_description =
        format!("Next waiting page, else the last read one; hold {switch_hold} to pick");
    let rows = [
        (label(Search), "Search your forest or go to an address"),
        (label(Address), "Same, starting from the current address"),
        (label(Forest), "Forest view"),
        (label(TreeWorkbench), "Current-tree workbench"),
        (label(NewTree), "New tree"),
        (label(Settle), "Settle the tree, or close the open overlay"),
        (
            label(UndoSettle),
            "Reopen the most recently settled tree or branch",
        ),
        (digit_range_label(digit_modifiers, "+"), "Switch to a tree"),
        (
            label(PreviousTree),
            "Switch to the previously shown open tree",
        ),
        (label(Switch), &switch_description),
        (label(Help), "How trees work"),
        (
            label(ToggleFocus),
            "Move between the page and the tree strip",
        ),
        (label(Back), "Back to the parent page"),
        (label(Forward), "Forward to the last opened child"),
        (
            format!("{}/{}", previous.label("+"), next.key_name()),
            "Previous or next sibling",
        ),
        (label(Reload), "Reload the page"),
        (label(Bookmark), "Toggle the page bookmark"),
        (label(Remember), "Remember the selection"),
        (label(FillLogin), "Fill the saved login for the page"),
        (
            label(PastePlain),
            "Paste as plain text, even where the page blocks pasting",
        ),
        (label(ForestHistory), ForestHistory.command().name),
        (label(MoveTreeLeft), MoveTreeLeft.command().name),
        (label(MoveTreeRight), MoveTreeRight.command().name),
        (label(Find), Find.command().name),
        (label(FindNext), FindNext.command().name),
        (label(FindPrevious), FindPrevious.command().name),
        (label(Increase), Increase.command().name),
        (label(Decrease), Decrease.command().name),
        (label(ResetSize), ResetSize.command().name),
        (label(Escape), Escape.command().name),
        (label(Quit), "Quit"),
    ];
    let mut help = String::new();
    for (key, description) in rows {
        writeln!(help, "{key:23}{description}").unwrap();
    }
    help
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
    fn shortcut_labels_have_chip_and_running_text_styles() {
        let keys = ShortcutSettings::defaults().presentation();
        let label = |id: CommandId| &keys.labels[id.stable_id()];
        for (id, chip, text) in [
            (CommandId::Find, "Ctrl F", "Ctrl+F"),
            (CommandId::Back, "Alt ←", "Alt+←"),
            (CommandId::Increase, "Ctrl +", "Ctrl++"),
            (CommandId::Decrease, "Ctrl −", "Ctrl+−"),
            (CommandId::Escape, "Esc", "Esc"),
            (
                CommandId::MoveTreeLeft,
                "Ctrl Shift Page Up",
                "Ctrl+Shift+Page Up",
            ),
            (CommandId::PreviousTree, "Ctrl Shift Tab", "Ctrl+Shift+Tab"),
            (CommandId::TreeDigit, "Ctrl 1…9", "Ctrl+1…9"),
        ] {
            assert_eq!(label(id).chip, chip);
            assert_eq!(label(id).text, text);
        }
        assert_eq!(keys.digit_labels.first().unwrap(), "Ctrl+1");
        assert_eq!(keys.digit_labels.last().unwrap(), "Ctrl+9");
        let footer = keys.switcher.unwrap();
        assert_eq!(footer.next, "Tab");
        assert_eq!(footer.previous, "Shift Tab");
        assert_eq!(footer.hold, "Ctrl");
    }

    #[test]
    fn switcher_hold_uses_the_opener_and_leaves_shift_free() {
        assert!(KeyCombo::new(Modifiers::NONE, key::F8).hold().is_none());
        assert!(KeyCombo::new(Modifiers::SHIFT, key::F8).hold().is_none());
        let opener = KeyCombo::new(
            Modifiers(M::MOD1_MASK | M::SHIFT_MASK | M::SUPER_MASK),
            key::j,
        );
        let hold = opener.hold().unwrap();
        assert!(!hold.released_by(key::Shift_L));
        assert!(!hold.released_by(key::Control_L));
        assert!(hold.released_by(key::Alt_R));
        assert!(hold.released_by(key::Super_L));
        assert_eq!(
            KeyCombo::new(hold, key::Tab).held_tab_direction(hold),
            Some(false)
        );
        assert_eq!(
            KeyCombo::new(opener.held, key::ISO_Left_Tab).held_tab_direction(hold),
            Some(true)
        );
        let footer = opener.switcher_footer(hold);
        assert_eq!(footer.alternate.as_deref(), Some("Shift J"));
        assert_eq!(footer.hold, "Alt or Super");
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
