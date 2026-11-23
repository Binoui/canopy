# Window shortcuts

Window shortcuts work from the page and the chrome. Change them in the Settings menu. Menus, tooltips, and the help card show the current combinations. Command-line help lists defaults.

## Sub-features

- Window commands match exact modifiers. Caps Lock and Num Lock do not affect the match.
- Page size and tree selection accept keypad and AZERTY variants.
- Chips use spaces. Tooltips and running text usually use plus signs.
- The editor supports search, recording, conflicts, and row or full reset. Escape and tree digits stay fixed.
- Changes survive a restart in the `shortcuts` file beside the database. Recent errors reports skipped entries.

## How to get to it (user POV)

Open **Settings → Keyboard shortcuts**. Search by name or key. Select a chip or **Add** to record. Press Delete on a chip to remove it. Tab and the arrow keys move focus. Press Escape or click outside the panel to back out one step.

## Driving it with canopy-verify

Preconditions:

- A stress launch on a private display and a clean doctor.
- `$V` is `.claude/skills/verify-canopy/scripts/canopy-verify`.

1. Open **Keyboard shortcuts** from the gear, by pointer and by keyboard. Search for `ctrl e`, `reload`, an unmatched word, and both fixed rows.
2. Focus the Forest chip and press Enter. Try a plain letter and Ctrl+V. Check the explanation and that recording continues. Record F6 and check the conflict names the current owner. Use Escape, then start recording again and click the search field. Check that both actions cancel recording.
3. Record Ctrl+J for Forest. Close the panel and check the gear's Forest chip and Canopy button tooltip. Press Ctrl+E and then Ctrl+J. Verify that only Ctrl+J opens Forest.
4. Restart with `$V quit` and `$V start`. Resize the new window if needed. Verify Ctrl+J still opens Forest. Try **Move it here**, the row's **Reset**, and **Reset all**.
5. Confirm the editor layout and focus in both themes, at 1240 by 800, about 720 px wide, and in a short window.
6. To check file loading, stop Canopy and edit the run's private `shortcuts` file. Put an unknown key between two valid rows. Restart and check that both valid keys work. Open **Recent errors** and look for one message.
7. Rebind the page switcher to Alt+J and switch to a tree with waiting pages. Hold Alt and press J. Use Tab, Shift+Tab, and the arrow keys, then release Alt. Check that the selected page opens. Run `canopy --help` without a display and check that it lists the defaults and points to Settings.
8. Run `$V cleanup`.

## Gotchas

- The site blocking row names the fixture port. Open the menu on the same fixture origin when comparing captures.
- The driver's `key` command releases the modifiers. Use the private display from `$V env` for a held press.
