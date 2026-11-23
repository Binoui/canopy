# Window shortcuts

Window shortcuts work from the page and the chrome. The menu, tooltips, help card and command-line help show combinations from the same command list.

## Sub-features

- Window commands match exact modifiers. Caps Lock and Num Lock do not affect the match.
- Page size and tree selection accept keypad and AZERTY variants.
- Chips use spaces. Tooltips and running text usually use plus signs.

## How to get to it (user POV)

Open the gear menu to see the page and tree commands. Press F1 or choose **How trees work** to see the help card. Hold Ctrl and press Tab to see the waiting and read pages and their controls. Run `canopy --help` to see the command-line shortcuts.

## Driving it with canopy-verify

Preconditions:

- A stress launch on a private display and a clean doctor.
- `$V` is `.claude/skills/verify-canopy/scripts/canopy-verify`.

1. Click the gear and check the chips for Find, Reload and Back. Press Escape to close the menu.
2. Press F1 and check the waiting and undo sentences. Press Escape to return to the page.
3. Press Ctrl+4. On the run's private display, hold Ctrl and press Tab. Check the footer, press Tab again, then release Ctrl to open the selected page.
4. Run `canopy --help` with no display and check that the shortcut descriptions line up.
5. Run `$V cleanup`.

## Gotchas

- The site blocking row names the fixture port. Open the menu on the same fixture origin when comparing captures.
- The driver's `key` command releases the modifiers. Use the private display from `$V env` for a held press.
