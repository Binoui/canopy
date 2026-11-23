use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::Path,
};

use super::{key, Command, CommandId, Defaults, Key, KeyCombo, Modifiers, Shortcut, COMMANDS};

pub(crate) struct Bindings {
    active: Vec<Vec<KeyCombo>>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EditableId(CommandId);

impl EditableId {
    fn named(name: &str) -> Option<Self> {
        COMMANDS
            .iter()
            .find(|row| row.stable_id == name && row.editable())
            .map(|row| Self(row.id))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct EditableCombo(KeyCombo);

#[derive(Debug, Eq, PartialEq)]
enum Refusal {
    Typing,
    TextEditing,
    Fixed,
    ModifierOnly,
    UnknownKey,
    UnsupportedModifiers,
}

impl EditableCombo {
    fn new(combo: KeyCombo) -> Result<Self, Refusal> {
        let key = combo.key;
        let held = combo.held;
        if !Modifiers::EDITABLE.contains(held.0) {
            return Err(Refusal::UnsupportedModifiers);
        }
        if key == key::Escape
            || combo
                .tree_digit(CommandId::tree_digit_modifiers())
                .is_some()
        {
            return Err(Refusal::Fixed);
        }
        let name = key.name().ok_or(Refusal::UnknownKey)?;
        if key == key::VoidSymbol || name.starts_with("0x") {
            return Err(Refusal::UnknownKey);
        }
        if super::is_modifier_key(key) {
            return Err(Refusal::ModifierOnly);
        }
        if matches!(
            (held, key),
            (
                Modifiers::CONTROL,
                key::c | key::x | key::v | key::a | key::z | key::y
            ) | (Modifiers::CONTROL_SHIFT, key::z)
        ) {
            return Err(Refusal::TextEditing);
        }
        if !held.0.intersects(Modifiers::CONTROL.0 | Modifiers::ALT.0)
            && (key.to_unicode().is_some_and(|c| !c.is_control())
                || name.starts_with("dead_")
                || matches!(
                    key,
                    key::Multi_key
                        | key::Return
                        | key::ISO_Enter
                        | key::KP_Enter
                        | key::Tab
                        | key::ISO_Left_Tab
                        | key::KP_Tab
                        | key::BackSpace
                        | key::Delete
                        | key::Insert
                        | key::Home
                        | key::End
                        | key::Page_Up
                        | key::Page_Down
                        | key::Left
                        | key::Right
                        | key::Up
                        | key::Down
                        | key::KP_Delete
                        | key::KP_Insert
                        | key::KP_Home
                        | key::KP_End
                        | key::KP_Begin
                        | key::KP_Page_Up
                        | key::KP_Page_Down
                        | key::KP_Left
                        | key::KP_Right
                        | key::KP_Up
                        | key::KP_Down
                ))
        {
            return Err(Refusal::Typing);
        }
        Ok(Self(combo))
    }

    fn parse(mut text: &str) -> Result<Self, Refusal> {
        let mut held = Modifiers::NONE;
        while let Some((prefix, rest)) = text.split_once('+') {
            let Some((modifier, _)) = Modifiers::NAMES
                .into_iter()
                .find(|(_, name)| prefix.trim().eq_ignore_ascii_case(name))
            else {
                break;
            };
            if held.0.contains(modifier.0) {
                return Err(Refusal::UnsupportedModifiers);
            }
            held.0.insert(modifier.0);
            text = rest.trim();
        }
        let key = if let Some(hex) = text.strip_prefix("U+").or_else(|| text.strip_prefix("u+")) {
            let scalar = u32::from_str_radix(hex, 16)
                .ok()
                .and_then(char::from_u32)
                .filter(|&c| c != '\0')
                .ok_or(Refusal::UnknownKey)?;
            Key::from_unicode(scalar)
        } else if !text.contains('\0') {
            Key::from_name(text)
        } else {
            return Err(Refusal::UnknownKey);
        };
        Self::new(KeyCombo::new(held, key))
    }
}

type Overrides = BTreeMap<EditableId, Vec<EditableCombo>>;

impl Bindings {
    pub(super) fn defaults() -> Self {
        Self::compose(COMMANDS, &Overrides::new())
    }

    fn compose(commands: &[Command], overrides: &Overrides) -> Self {
        let mut assigned: HashSet<_> = overrides.values().flatten().copied().collect();
        if let Defaults::Exact { combos, .. } = CommandId::Escape.command().defaults {
            assigned.extend(combos.iter().copied().map(EditableCombo));
        }
        let active = commands
            .iter()
            .map(|row| {
                let Defaults::Exact { combos, .. } = row.defaults else {
                    return Vec::new();
                };
                if !row.editable() {
                    return combos.to_vec();
                }
                match overrides.get(&EditableId(row.id)) {
                    Some(explicit) => explicit.iter().map(|combo| combo.0).collect(),
                    None => combos
                        .iter()
                        .copied()
                        .filter(|&combo| assigned.insert(EditableCombo(combo)))
                        .collect(),
                }
            })
            .collect();
        Self { active }
    }

    pub(crate) fn resolve(&self, combo: KeyCombo) -> Option<Shortcut> {
        COMMANDS.iter().find_map(|row| match row.defaults {
            Defaults::Exact { action, .. } => self.active[row.id as usize]
                .contains(&combo)
                .then_some(action),
            Defaults::TreeDigits(held) => combo.tree_digit(held).map(Shortcut::Tree),
        })
    }

    pub(super) fn primary(&self, id: CommandId) -> Option<KeyCombo> {
        self.active[id as usize].first().copied()
    }

    pub(super) fn load(path: &Path) -> (Self, Option<String>) {
        match fs::read(path) {
            Ok(bytes) => {
                let (overrides, skipped) = parse_overrides(&bytes);
                let problem = skipped.map(|(first, count)| {
                    format!("Skipped {count} invalid shortcut entries in shortcuts (first on line {first}).")
                });
                (Self::compose(COMMANDS, &overrides), problem)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Self::defaults(), None),
            Err(error) => (
                Self::defaults(),
                Some(format!("Could not read shortcuts: {error}")),
            ),
        }
    }
}

fn parse_overrides(bytes: &[u8]) -> (Overrides, Option<(usize, usize)>) {
    let mut overrides = Overrides::new();
    let mut seen = HashSet::new();
    let mut assigned = HashSet::new();
    let mut skipped = None;
    for (index, bytes) in bytes.split(|&b| b == b'\n').enumerate() {
        let line = std::str::from_utf8(bytes).ok().map(str::trim);
        if line.is_some_and(|text| text.is_empty() || text.starts_with('#')) {
            continue;
        }
        if let Some((id, combos)) = line.and_then(parse_line) {
            if !seen.contains(&id.0) && combos.iter().all(|combo| !assigned.contains(combo)) {
                seen.insert(id.0);
                assigned.extend(combos.iter().copied());
                let Defaults::Exact {
                    combos: defaults, ..
                } = id.0.command().defaults
                else {
                    unreachable!("an editable command has exact defaults");
                };
                if !combos
                    .iter()
                    .map(|combo| combo.0)
                    .eq(defaults.iter().copied())
                {
                    overrides.insert(id, combos);
                }
                continue;
            }
        }
        skipped.get_or_insert((index + 1, 0)).1 += 1;
    }
    (overrides, skipped)
}

fn parse_line(line: &str) -> Option<(EditableId, Vec<EditableCombo>)> {
    let (name, keys) = line.split_once('=')?;
    let id = EditableId::named(name.trim())?;
    let mut unique = HashSet::new();
    let mut combos = Vec::new();
    if !keys.trim().is_empty() {
        for text in keys.split(',') {
            let combo = EditableCombo::parse(text.trim()).ok()?;
            if !unique.insert(combo) {
                return None;
            }
            combos.push(combo);
        }
    }
    Some((id, combos))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(text: &str) -> KeyCombo {
        EditableCombo::parse(text).unwrap().0
    }

    #[test]
    fn missing_file_defaults_and_an_unreadable_file_reports_one_problem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shortcuts");
        let (bindings, problem) = Bindings::load(&path);
        assert_eq!(
            bindings.resolve(combo("Ctrl+e")),
            Some(Shortcut::Forest(crate::chrome::ForestView::Canopy))
        );
        assert_eq!(problem, None);
        fs::create_dir(&path).unwrap();
        let (bindings, problem) = Bindings::load(&path);
        assert_eq!(bindings.resolve(combo("F5")), Some(Shortcut::Reload));
        assert!(problem.unwrap().starts_with("Could not read shortcuts"));
    }

    #[test]
    fn invalid_override_lines_do_not_reserve_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shortcuts");
        fs::write(&path, b"# personal keys\n\nunknown = F8\nreload = Ctrl+j\nfind = F7, Ctrl+j\nforest = F8, Ctrl+Hyperspace\nfind = F7, F7\nquit = Ctrl+c\nfind = F7\nfind = F8\nforest = \xff\nescape = F8\ntree-digit = F8\nforest = F8\nfill-login =\n").unwrap();
        let (bindings, problem) = Bindings::load(&path);
        assert_eq!(
            problem.as_deref(),
            Some("Skipped 9 invalid shortcut entries in shortcuts (first on line 3).")
        );
        assert_eq!(bindings.resolve(combo("Ctrl+j")), Some(Shortcut::Reload));
        assert_eq!(bindings.resolve(combo("F7")), Some(Shortcut::Find));
        assert_eq!(
            bindings.resolve(combo("F8")),
            Some(Shortcut::Forest(crate::chrome::ForestView::Canopy))
        );
        assert_eq!(bindings.resolve(combo("Ctrl+Shift+l")), None);
        assert_eq!(bindings.resolve(combo("Ctrl+q")), Some(Shortcut::Quit));
    }

    #[test]
    fn explicit_assignment_wins_against_new_default() {
        let (overrides, problem) = parse_overrides(b"find = Ctrl+j");
        assert_eq!(problem, None);
        let mut catalog = COMMANDS.to_vec();
        catalog[CommandId::Reload as usize].defaults = Defaults::Exact {
            action: Shortcut::Reload,
            combos: &[
                KeyCombo {
                    held: Modifiers::CONTROL,
                    key: key::j,
                },
                KeyCombo {
                    held: Modifiers::NONE,
                    key: key::F5,
                },
            ],
        };
        let bindings = Bindings::compose(&catalog, &overrides);
        assert_eq!(bindings.resolve(combo("Ctrl+j")), Some(Shortcut::Find));
        assert_eq!(bindings.primary(CommandId::Reload), Some(combo("F5")));
    }

    #[test]
    fn refuses_typing_editing_fixed_and_unknown_keys_at_both_inputs() {
        for key in [
            key::a,
            key::_1,
            key::plus,
            key::space,
            key::Return,
            key::Tab,
            key::ISO_Left_Tab,
            key::BackSpace,
            key::Delete,
            key::Insert,
            key::Home,
            key::End,
            key::Page_Up,
            key::Page_Down,
            key::Left,
            key::Right,
            key::Up,
            key::Down,
            key::KP_Enter,
            key::KP_Page_Up,
            key::KP_Insert,
            key::dead_acute,
            key::Multi_key,
        ] {
            for held in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::SUPER] {
                assert_eq!(
                    EditableCombo::new(KeyCombo::new(held, key)),
                    Err(Refusal::Typing)
                );
            }
            assert!(EditableCombo::new(KeyCombo::new(Modifiers::ALT, key)).is_ok());
        }
        for key in [
            "Ctrl+c",
            "Ctrl+x",
            "Ctrl+v",
            "Ctrl+a",
            "Ctrl+z",
            "Ctrl+Shift+z",
            "Ctrl+y",
        ] {
            assert_eq!(EditableCombo::parse(key), Err(Refusal::TextEditing));
        }
        for key in [
            "Ctrl+Escape",
            "Escape",
            "Ctrl+9",
            "Ctrl+Shift+2",
            "Ctrl+KP_1",
        ] {
            assert_eq!(EditableCombo::parse(key), Err(Refusal::Fixed));
        }
        assert_eq!(
            EditableCombo::parse("Alt+Shift_L"),
            Err(Refusal::ModifierOnly)
        );
        assert_eq!(
            EditableCombo::parse("Ctrl+Ctrl+F8"),
            Err(Refusal::UnsupportedModifiers)
        );
        for key in ["Ctrl+Hyperspace", "Ctrl+\0", "Ctrl+U+D800", "Ctrl+U+0"] {
            assert_eq!(EditableCombo::parse(key), Err(Refusal::UnknownKey));
        }
        for key in [
            "F8",
            "Super+F8",
            "Ctrl+Shift+v",
            "Alt+1",
            "Ctrl+0",
            "Ctrl+agrave",
        ] {
            assert!(EditableCombo::parse(key).is_ok(), "{key}");
        }
    }

    #[test]
    fn keys_show_capitals_and_names() {
        for (file, chip) in [
            ("ctrl+J", "Ctrl J"),
            ("Alt+U+00e0", "Alt À"),
            ("Super+F8", "Super F8"),
            ("Ctrl+Page_Up", "Ctrl Page Up"),
            ("Alt+Left", "Alt ←"),
            ("Ctrl+Shift+ISO_Left_Tab", "Ctrl Shift Tab"),
            ("Ctrl+KP_0", "Ctrl Keypad 0"),
        ] {
            assert_eq!(combo(file).labels().chip, chip);
        }
    }
}
