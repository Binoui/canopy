use std::collections::HashSet;

use anyhow::Result;
use serde_json::{json, Value};

use super::{
    bindings::{Bindings, Edit, EditableCombo, EditableId, Refusal, Slot, Target},
    digit_range_label, is_modifier_key, key, Command, Defaults, KeyCombo, Modifiers,
    ShortcutSettings, COMMANDS,
};

#[derive(Default)]
pub(super) struct Editor {
    session: Option<Session>,
    pub(super) revision: u64,
    down: HashSet<u16>,
    swallowed: HashSet<u16>,
    press_cancelled_recording: bool,
}

#[derive(Clone, Copy)]
struct Session {
    id: u64,
    phase: Phase,
}

#[derive(Clone, Copy)]
enum Phase {
    Idle,
    Recording {
        target: Target,
        refusal: Option<Refusal>,
    },
    Conflict {
        target: Target,
        combo: EditableCombo,
        owner: EditableId,
    },
    ConfirmResetAll,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorIntent {
    session: Option<u64>,
    revision: u64,
    action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Open,
    Close,
    Scrim,
    Record(Target),
    Remove(EditableId, usize),
    Reset(EditableId),
    AskResetAll,
    ConfirmResetAll,
    MoveHere,
    Cancel,
}

impl EditorIntent {
    pub(crate) fn parse(message: &Value) -> Option<Self> {
        let index = |field: &str| usize::try_from(message[field].as_u64()?).ok();
        let command = || EditableId::named(message["command"].as_str()?);
        let action = match message["action"].as_str()? {
            "open" => Action::Open,
            "close" => Action::Close,
            "scrim" => Action::Scrim,
            "record" => Action::Record(Target {
                command: command()?,
                slot: if message.get("slot")?.is_null() {
                    Slot::Add
                } else {
                    Slot::Replace(index("slot")?)
                },
            }),
            "remove" => Action::Remove(command()?, index("index")?),
            "reset" => Action::Reset(command()?),
            "ask-reset-all" => Action::AskResetAll,
            "confirm-reset-all" => Action::ConfirmResetAll,
            "move-here" => Action::MoveHere,
            "cancel" => Action::Cancel,
            _ => return None,
        };
        let session = message.get("session")?;
        Some(Self {
            session: if session.is_null() {
                None
            } else {
                Some(session.as_u64()?)
            },
            revision: message["revision"].as_u64()?,
            action,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EditorUpdate {
    Unchanged,
    Paint,
    Open,
    Close { return_focus: bool },
}

pub(crate) enum EditorKey {
    Dispatch,
    Chrome,
    Take(Result<EditorUpdate>),
}

impl ShortcutSettings {
    pub(crate) fn editor_input(&mut self, intent: EditorIntent) -> Result<EditorUpdate> {
        let editor = &mut self.editor;
        if editor.session.map(|session| session.id) != intent.session
            || editor.revision != intent.revision
        {
            return Ok(EditorUpdate::Unchanged);
        }
        let Some(session) = editor.session else {
            return if intent.action == Action::Open {
                editor.revision += 1;
                editor.session = Some(Session {
                    id: editor.revision,
                    phase: Phase::Idle,
                });
                Ok(EditorUpdate::Open)
            } else {
                Ok(EditorUpdate::Unchanged)
            };
        };
        let update = match (session.phase, intent.action) {
            (_, Action::Close) => self.close_editor(true),
            (_, Action::Scrim) if !editor.press_cancelled_recording => self.back_editor(),
            (Phase::Idle, Action::Record(target)) => {
                if let Slot::Replace(index) = target.slot {
                    if !self.has_slot(target.command, index) {
                        return Ok(EditorUpdate::Unchanged);
                    }
                }
                self.set_phase(Phase::Recording {
                    target,
                    refusal: None,
                })
            }
            (Phase::Idle, Action::Remove(id, index)) if self.has_slot(id, index) => {
                self.apply(Edit::Remove(id, index))?
            }
            (Phase::Idle, Action::Reset(id)) => self.apply(Edit::Reset(id))?,
            (Phase::Idle, Action::AskResetAll) if self.bindings.any_changed() => {
                self.set_phase(Phase::ConfirmResetAll)
            }
            (Phase::ConfirmResetAll, Action::ConfirmResetAll) => self.apply(Edit::ResetAll)?,
            (
                Phase::Conflict {
                    target,
                    combo,
                    owner,
                },
                Action::MoveHere,
            ) => self.apply(Edit::Assign {
                target,
                combo,
                from: Some(owner),
            })?,
            (Phase::Conflict { .. } | Phase::ConfirmResetAll, Action::Cancel) => {
                self.set_phase(Phase::Idle)
            }
            _ => EditorUpdate::Unchanged,
        };
        Ok(update)
    }

    pub(crate) fn editor_key(&mut self, event: &gdk::EventKey, combo: KeyCombo) -> EditorKey {
        let hardware = event.hardware_keycode();
        if self.editor.swallowed.contains(&hardware) {
            return EditorKey::Take(Ok(EditorUpdate::Unchanged));
        }
        let Some(session) = self.editor.session else {
            return EditorKey::Dispatch;
        };
        let fresh = self.editor.down.insert(hardware);
        if combo.key == key::Escape && combo.held == Modifiers::NONE {
            self.editor.swallowed.insert(hardware);
            return EditorKey::Take(Ok(self.back_editor()));
        }
        let Phase::Recording { target, .. } = session.phase else {
            return EditorKey::Chrome;
        };
        self.editor.swallowed.insert(hardware);
        // WebKit may return the Enter that activated the chip while its key is still down.
        if !fresh || event.is_modifier() || is_modifier_key(combo.key) {
            return EditorKey::Take(Ok(EditorUpdate::Unchanged));
        }
        EditorKey::Take(self.record(target, combo))
    }

    pub(crate) fn editor_key_release(&mut self, hardware: u16) {
        self.editor.down.remove(&hardware);
        self.editor.swallowed.remove(&hardware);
    }

    pub(crate) fn editor_focus_out(&mut self) -> EditorUpdate {
        self.editor.down.clear();
        self.editor.swallowed.clear();
        self.cancel_recording()
    }

    pub(crate) fn editor_pointer_press(&mut self) -> EditorUpdate {
        let update = self.cancel_recording();
        // The scrim receives this same press in WebKit. It must only back out one level.
        self.editor.press_cancelled_recording = update == EditorUpdate::Paint;
        update
    }

    pub(crate) fn close_editor(&mut self, return_focus: bool) -> EditorUpdate {
        if self.editor.session.take().is_none() {
            return EditorUpdate::Unchanged;
        }
        self.editor.revision += 1;
        self.editor.down.clear();
        self.editor.press_cancelled_recording = false;
        EditorUpdate::Close { return_focus }
    }

    fn has_slot(&self, command: EditableId, index: usize) -> bool {
        index < self.bindings.keys(command.id()).len()
    }

    fn cancel_recording(&mut self) -> EditorUpdate {
        if self
            .editor
            .session
            .is_some_and(|session| matches!(session.phase, Phase::Recording { .. }))
        {
            self.set_phase(Phase::Idle)
        } else {
            EditorUpdate::Unchanged
        }
    }

    fn back_editor(&mut self) -> EditorUpdate {
        match self.editor.session.map(|session| session.phase) {
            Some(Phase::Idle) => self.close_editor(true),
            Some(_) => self.set_phase(Phase::Idle),
            None => EditorUpdate::Unchanged,
        }
    }

    fn set_phase(&mut self, phase: Phase) -> EditorUpdate {
        self.editor.session.as_mut().expect("an open editor").phase = phase;
        self.editor.revision += 1;
        EditorUpdate::Paint
    }

    fn record(&mut self, target: Target, combo: KeyCombo) -> Result<EditorUpdate> {
        let combo = match EditableCombo::new(combo) {
            Ok(combo) => combo,
            Err(refusal) => {
                return Ok(self.set_phase(Phase::Recording {
                    target,
                    refusal: Some(refusal),
                }))
            }
        };
        let owner = self
            .bindings
            .owner(combo)
            .filter(|&id| id != target.command);
        if let Some(owner) = owner {
            Ok(self.set_phase(Phase::Conflict {
                target,
                combo,
                owner,
            }))
        } else {
            self.apply(Edit::Assign {
                target,
                combo,
                from: None,
            })
        }
    }

    fn apply(&mut self, edit: Edit) -> Result<EditorUpdate> {
        self.bindings.commit(&self.path, edit)?;
        Ok(self.set_phase(Phase::Idle))
    }
}

impl Editor {
    pub(super) fn presentation(&self, bindings: &Bindings) -> Option<Value> {
        let session = self.session?;
        let mut groups = Vec::new();
        let mut commands = COMMANDS;
        while let Some(command) = commands.first() {
            let group = command.group.label();
            let end = commands
                .iter()
                .position(|row| row.group.label() != group)
                .unwrap_or(commands.len());
            groups.push(json!({
                "name": group,
                "rows": commands[..end].iter().map(|row| row_value(row, bindings)).collect::<Vec<_>>(),
            }));
            commands = &commands[end..];
        }
        Some(json!({
            "session": session.id,
            "phase": session.phase.value(),
            "groups": groups,
            "anyChanged": bindings.any_changed(),
        }))
    }
}

impl Phase {
    fn value(self) -> Value {
        fn pending(kind: &str, target: Target) -> Value {
            let slot = match target.slot {
                Slot::Add => None,
                Slot::Replace(index) => Some(index),
            };
            json!({ "kind": kind, "command": target.command.id().stable_id(), "slot": slot })
        }
        match self {
            Self::Idle => json!({ "kind": "idle" }),
            Self::Recording { target, refusal } => {
                let mut value = pending("recording", target);
                value["explanation"] = json!(refusal.map(Refusal::message));
                value
            }
            Self::Conflict {
                target,
                combo,
                owner,
            } => {
                let mut value = pending("conflict", target);
                value["key"] = json!(combo.combo().label("+"));
                value["owner"] = json!(owner.id().command().name);
                value
            }
            Self::ConfirmResetAll => json!({ "kind": "confirm-reset-all" }),
        }
    }
}

impl Refusal {
    fn message(self) -> &'static str {
        match self {
            Self::Typing => "Add Ctrl or Alt to keep this key available for typing and navigation.",
            Self::TextEditing => "This combination is reserved for text editing. Try another.",
            Self::Fixed => "This combination belongs to a fixed shortcut. Try another.",
            Self::ModifierOnly => "Hold the modifiers and press a key.",
            Self::UnknownKey => "Canopy could not identify this key. Try another.",
            Self::UnsupportedModifiers => {
                "Use Ctrl, Shift, Alt, or Super. Try another combination."
            }
        }
    }
}

fn row_value(row: &Command, bindings: &Bindings) -> Value {
    let mut terms = vec![row.name.to_owned()];
    let combos = bindings.keys(row.id);
    let (keys, changed) = match row.defaults {
        Defaults::Exact {
            combos: defaults, ..
        } => {
            let keys = combos
                .iter()
                .map(|&combo| {
                    let mut labels = combo.labels();
                    terms.extend([labels.chip.clone(), labels.text.clone()]);
                    if let Some(name) = combo.key.name().filter(|_| {
                        combo.key.to_unicode().is_none_or(char::is_control)
                            || matches!(combo.key, key::plus | key::minus)
                    }) {
                        terms.push(combo.with_key_name(&name.replace('_', " "), " "));
                    }
                    if combo.key == key::ISO_Left_Tab
                        && combos.contains(&KeyCombo::new(combo.held, key::Tab))
                    {
                        labels.chip.push_str(" (ISO)");
                        labels.text.push_str(" (ISO)");
                    }
                    json!({ "chip": labels.chip, "text": labels.text })
                })
                .collect();
            (keys, combos != defaults)
        }
        Defaults::TreeDigits([primary, shifted]) => {
            for modifiers in [primary, shifted] {
                let held = modifiers.label();
                for digit in super::TREE_DIGITS {
                    terms.push(format!("{held} {digit}"));
                }
            }
            let chip = digit_range_label(primary, " ");
            let text = digit_range_label(primary, "+");
            terms.push(chip.clone());
            (vec![json!({ "chip": chip, "text": text })], false)
        }
    };
    json!({
        "id": row.stable_id,
        "name": row.name,
        "keys": keys,
        "terms": terms,
        "editable": row.editable(),
        "changed": changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(settings: &ShortcutSettings, action: Value) -> EditorIntent {
        let mut message = action;
        message["session"] = json!(settings.editor.session.map(|session| session.id));
        message["revision"] = json!(settings.editor.revision);
        EditorIntent::parse(&message).unwrap()
    }

    fn open(settings: &mut ShortcutSettings) {
        assert_eq!(
            settings
                .editor_input(intent(settings, json!({ "action": "open" })))
                .unwrap(),
            EditorUpdate::Open
        );
    }

    #[test]
    fn repeated_remove_and_actions_from_a_closed_session_do_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let (mut settings, _) = ShortcutSettings::load(directory.path());
        open(&mut settings);
        let remove = intent(
            &settings,
            json!({ "action": "remove", "command": "reload", "index": 0 }),
        );
        assert_eq!(
            settings.editor_input(remove.clone()).unwrap(),
            EditorUpdate::Paint
        );
        assert_eq!(
            settings.editor_input(remove.clone()).unwrap(),
            EditorUpdate::Unchanged
        );
        assert_eq!(
            settings.close_editor(true),
            EditorUpdate::Close { return_focus: true }
        );
        open(&mut settings);
        assert_eq!(
            settings.editor_input(remove).unwrap(),
            EditorUpdate::Unchanged
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("shortcuts")).unwrap(),
            "reload = Ctrl+Shift+r, F5\n"
        );
    }

    #[test]
    fn rejects_fixed_rows_and_nonexistent_chips_in_editor_requests() {
        for id in ["escape", "tree-digit"] {
            assert!(EditorIntent::parse(&json!({
                "action": "record", "session": 1, "revision": 1, "command": id, "slot": 0,
            }))
            .is_none());
        }
        let directory = tempfile::tempdir().unwrap();
        let (mut settings, _) = ShortcutSettings::load(directory.path());
        open(&mut settings);
        let record = intent(
            &settings,
            json!({ "action": "record", "command": "reload", "slot": 9 }),
        );
        assert_eq!(
            settings.editor_input(record).unwrap(),
            EditorUpdate::Unchanged
        );
        assert_eq!(
            settings.editor.presentation(&settings.bindings).unwrap()["phase"]["kind"],
            "idle"
        );
    }

    #[test]
    fn a_scrim_press_cancels_a_recording_before_it_can_close_the_editor() {
        let directory = tempfile::tempdir().unwrap();
        let (mut settings, _) = ShortcutSettings::load(directory.path());
        open(&mut settings);
        let record = intent(
            &settings,
            json!({ "action": "record", "command": "reload", "slot": 0 }),
        );
        assert_eq!(settings.editor_input(record).unwrap(), EditorUpdate::Paint);
        assert_eq!(settings.editor_pointer_press(), EditorUpdate::Paint);
        let scrim = intent(&settings, json!({ "action": "scrim" }));
        assert_eq!(
            settings.editor_input(scrim.clone()).unwrap(),
            EditorUpdate::Unchanged
        );
        assert_eq!(settings.editor_pointer_press(), EditorUpdate::Unchanged);
        assert_eq!(
            settings.editor_input(scrim).unwrap(),
            EditorUpdate::Close { return_focus: true }
        );
    }
}
