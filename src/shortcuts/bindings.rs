use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::Path,
};

use anyhow::Context;

use super::{key, Command, CommandId, Defaults, Key, KeyCombo, Modifiers, Shortcut, COMMANDS};

pub(crate) struct Bindings {
    active: Vec<Vec<KeyCombo>>,
    explicit: Overrides,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct EditableId(CommandId);

impl EditableId {
    pub(super) fn named(name: &str) -> Option<Self> {
        COMMANDS
            .iter()
            .find(|row| row.stable_id == name && row.editable())
            .map(|row| Self(row.id))
    }

    pub(super) fn id(self) -> CommandId {
        self.0
    }

    fn defaults(self) -> &'static [KeyCombo] {
        let Defaults::Exact { combos, .. } = self.0.command().defaults else {
            unreachable!("an editable command has exact defaults");
        };
        combos
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct EditableCombo(KeyCombo);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Refusal {
    Typing,
    TextEditing,
    Fixed,
    ModifierOnly,
    UnknownKey,
    UnsupportedModifiers,
}

impl EditableCombo {
    pub(super) fn new(combo: KeyCombo) -> Result<Self, Refusal> {
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

    fn file_name(self) -> String {
        let combo = self.0;
        let name = combo.key.name().expect("an editable key has a GDK name");
        combo.with_key_name(&name, "+")
    }

    pub(super) fn combo(self) -> KeyCombo {
        self.0
    }
}

type Overrides = BTreeMap<EditableId, Vec<EditableCombo>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Slot {
    Add,
    Replace(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Target {
    pub(super) command: EditableId,
    pub(super) slot: Slot,
}

pub(super) enum Edit {
    Assign {
        target: Target,
        combo: EditableCombo,
        from: Option<EditableId>,
    },
    Remove(EditableId, usize),
    Reset(EditableId),
    ResetAll,
}

impl Bindings {
    pub(super) fn defaults() -> Self {
        Self::compose(COMMANDS, Overrides::new())
    }

    fn compose(commands: &[Command], overrides: Overrides) -> Self {
        let mut assigned: HashSet<_> = overrides.values().flatten().map(|combo| combo.0).collect();
        if let Defaults::Exact { combos, .. } = CommandId::Escape.command().defaults {
            assigned.extend(combos.iter().copied());
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
                        .filter(|&combo| assigned.insert(combo))
                        .collect(),
                }
            })
            .collect();
        Self {
            active,
            explicit: overrides,
        }
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
        self.keys(id).first().copied()
    }

    pub(super) fn keys(&self, id: CommandId) -> &[KeyCombo] {
        &self.active[id as usize]
    }

    pub(super) fn owner(&self, combo: EditableCombo) -> Option<EditableId> {
        COMMANDS
            .iter()
            .find(|row| self.keys(row.id).contains(&combo.0))
            .map(|row| EditableId(row.id))
    }

    pub(super) fn any_changed(&self) -> bool {
        !self.explicit.is_empty()
    }

    pub(super) fn load(path: &Path) -> (Self, Option<String>) {
        match fs::read(path) {
            Ok(bytes) => {
                let (overrides, skipped) = parse_overrides(&bytes);
                let problem = skipped.map(|(first, count)| {
                    if count == 1 {
                        format!("Skipped an invalid shortcut entry on line {first} of shortcuts.")
                    } else {
                        format!("Skipped {count} invalid shortcut entries in shortcuts (first on line {first}).")
                    }
                });
                (Self::compose(COMMANDS, overrides), problem)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Self::defaults(), None),
            Err(error) => (
                Self::defaults(),
                Some(format!("Could not read shortcuts: {error}")),
            ),
        }
    }

    pub(super) fn commit(&mut self, path: &Path, edit: Edit) -> anyhow::Result<()> {
        let explicit = self.edit(edit);
        if explicit != self.explicit {
            crate::atomic_file::write(path, encode_overrides(&explicit))
                .context("Could not save keyboard shortcuts")?;
            *self = Self::compose(COMMANDS, explicit);
        }
        Ok(())
    }

    fn edit(&self, edit: Edit) -> Overrides {
        let mut explicit = self.explicit.clone();
        let effective = |id: EditableId| {
            self.keys(id.0)
                .iter()
                .copied()
                .map(EditableCombo)
                .collect::<Vec<_>>()
        };
        match edit {
            Edit::Assign {
                target,
                combo,
                from,
            } => {
                if let Some(from) = from {
                    let mut keys = effective(from);
                    keys.retain(|&key| key != combo);
                    set_override(&mut explicit, from, keys);
                }
                let mut keys = effective(target.command);
                match target.slot {
                    Slot::Add if !keys.contains(&combo) => keys.push(combo),
                    Slot::Replace(index) if keys[index] != combo => {
                        keys[index] = combo;
                        if let Some(other) =
                            (0..keys.len()).find(|&other| index != other && keys[other] == combo)
                        {
                            keys.remove(other);
                        }
                    }
                    _ => return explicit,
                }
                set_override(&mut explicit, target.command, keys);
            }
            Edit::Remove(id, index) => {
                let mut keys = effective(id);
                keys.remove(index);
                set_override(&mut explicit, id, keys);
            }
            Edit::Reset(id) => {
                let defaults = id.defaults();
                for row in COMMANDS
                    .iter()
                    .filter(|row| row.id != id.0 && row.editable())
                {
                    let other = EditableId(row.id);
                    let mut keys = effective(other);
                    let count = keys.len();
                    keys.retain(|combo| !defaults.contains(&combo.0));
                    if keys.len() != count {
                        set_override(&mut explicit, other, keys);
                    }
                }
                explicit.remove(&id);
            }
            Edit::ResetAll => explicit.clear(),
        }
        explicit
    }
}

fn set_override(explicit: &mut Overrides, id: EditableId, combos: Vec<EditableCombo>) {
    if combos
        .iter()
        .map(|combo| combo.0)
        .eq(id.defaults().iter().copied())
    {
        explicit.remove(&id);
    } else {
        explicit.insert(id, combos);
    }
}

fn encode_overrides(explicit: &Overrides) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for (id, combos) in explicit {
        let keys = combos
            .iter()
            .map(|combo| combo.file_name())
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            output,
            "{} ={space}{keys}",
            id.0.stable_id(),
            space = if keys.is_empty() { "" } else { " " }
        )
        .unwrap();
    }
    output
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
                set_override(&mut overrides, id, combos);
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
        let bindings = Bindings::compose(&catalog, overrides);
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

    #[test]
    fn file_round_trip_keeps_only_explicit_differences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shortcuts");
        fs::write(
            &path,
            "search = Ctrl+k\nreload = Ctrl+e\nfind = Ctrl+U+1F600, Ctrl+comma\n",
        )
        .unwrap();
        let (mut bindings, issue) = Bindings::load(&path);
        assert_eq!(issue, None);
        assert_eq!(bindings.primary(CommandId::Forest), None);
        bindings
            .commit(&path, Edit::Remove(EditableId(CommandId::Bookmark), 0))
            .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "reload = Ctrl+e\nbookmark =\nfind = Ctrl+U+1F600, Ctrl+comma\n"
        );
        let (mut bindings, issue) = Bindings::load(&path);
        assert_eq!(issue, None);
        assert_eq!(
            bindings.resolve(combo("Ctrl+U+1F600")),
            Some(Shortcut::Find)
        );
        assert_eq!(bindings.resolve(combo("Ctrl+comma")), Some(Shortcut::Find));
        bindings
            .commit(&path, Edit::Remove(EditableId(CommandId::Reload), 0))
            .unwrap();
        assert_eq!(bindings.primary(CommandId::Forest), Some(combo("Ctrl+e")));
    }

    #[test]
    fn failed_atomic_write_keeps_dispatch_and_the_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shortcuts");
        fs::write(&path, "reload = F8\n").unwrap();
        fs::create_dir(path.with_extension("tmp")).unwrap();
        let (mut bindings, _) = Bindings::load(&path);
        assert!(bindings.commit(&path, Edit::ResetAll).is_err());
        assert_eq!(bindings.resolve(combo("F8")), Some(Shortcut::Reload));
        assert_eq!(bindings.resolve(combo("F5")), None);
        assert_eq!(fs::read_to_string(&path).unwrap(), "reload = F8\n");
    }

    #[test]
    fn reset_reclaims_defaults_and_leaves_the_other_row_changed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shortcuts");
        fs::write(&path, "forest = Ctrl+r, F5, F8\n").unwrap();
        let (mut bindings, _) = Bindings::load(&path);
        assert_eq!(bindings.keys(CommandId::Reload), &[combo("Ctrl+Shift+r")]);
        bindings
            .commit(&path, Edit::Reset(EditableId(CommandId::Reload)))
            .unwrap();
        assert_eq!(
            bindings.keys(CommandId::Reload),
            &[combo("Ctrl+r"), combo("Ctrl+Shift+r"), combo("F5")]
        );
        assert_eq!(bindings.keys(CommandId::Forest), &[combo("F8")]);
        assert_eq!(fs::read_to_string(&path).unwrap(), "forest = F8\n");
    }

    #[test]
    fn moving_a_default_removes_it_from_the_prior_owner_until_reset() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shortcuts");
        let mut bindings = Bindings::defaults();
        bindings
            .commit(
                &path,
                Edit::Assign {
                    target: Target {
                        command: EditableId(CommandId::Find),
                        slot: Slot::Replace(0),
                    },
                    combo: EditableCombo::parse("F6").unwrap(),
                    from: Some(EditableId(CommandId::ToggleFocus)),
                },
            )
            .unwrap();
        assert_eq!(bindings.resolve(combo("F6")), Some(Shortcut::Find));
        bindings
            .commit(&path, Edit::Remove(EditableId(CommandId::Find), 0))
            .unwrap();
        let (mut bindings, _) = Bindings::load(&path);
        assert_eq!(bindings.resolve(combo("F6")), None);
        bindings.commit(&path, Edit::ResetAll).unwrap();
        assert_eq!(bindings.resolve(combo("F6")), Some(Shortcut::ToggleFocus));
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn recording_an_existing_chord_in_the_same_row_keeps_one_copy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shortcuts");
        let mut bindings = Bindings::defaults();
        let edit = |slot| Edit::Assign {
            target: Target {
                command: EditableId(CommandId::Reload),
                slot,
            },
            combo: EditableCombo::parse("F5").unwrap(),
            from: None,
        };
        bindings.commit(&path, edit(Slot::Add)).unwrap();
        assert!(!path.exists());
        bindings.commit(&path, edit(Slot::Replace(0))).unwrap();
        assert_eq!(
            bindings.keys(CommandId::Reload),
            &[combo("F5"), combo("Ctrl+Shift+r")]
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "reload = F5, Ctrl+Shift+r\n"
        );
    }
}
