# Canopy verification map

The maintained list of user-facing features and how to prove each one in the real window. Read this index, then the feature file that matches your change.

## Baseline

- `$V launch` with the default stress seed, then `$V doctor` with every line `ok`. `$V` is `.claude/skills/verify-canopy/scripts/canopy-verify`.
- Start each recipe from a fresh launch unless it says otherwise. Recipes change the forest (new pages, settled trees), so a second recipe on the same run must not assume the seed is untouched.
- Positions are for the default 1240x800 window. Take a screenshot before trusting one.

## Proof

- A screenshot after the action, and a `$V db` read of the stored result, both saved to the evidence folder.
- Record which entry point you drove. Driving the shortcut does not verify the button, and the reverse.
- A path you could not reach is reported with the command you tried and what was missing, never as verified.

## Feature entry contract

Each feature file opens with an H1 and one paragraph of user-visible behavior, then four H2s in this order: `Sub-features`, `How to get to it (user POV)`, `Driving it with canopy-verify` (starting with `Preconditions:`), and `Gotchas`.

## Features

- [Fork a tree](./fork.md) covers starting a tree, following links, going back, branching, and keeping the tree across a restart.
- [Tabs and the strip](./tabs-and-strip.md) covers switching trees, page cards in the strip, keyboard walking, and the zoomed tree.
- [Waiting pages](./waiting-pages.md) covers middle-clicked links that wait in the tree, the counter, and Ctrl+Tab.
- [Settle and the forest](./settle-and-forest.md) covers settling a tree, undo, and finding it again in the forest view.
- [Back and forward](./back-and-forward.md) covers parent and child navigation by keyboard and mouse, cold restores, and closing overlays.
- [Page size](./page-scale.md) covers keyboard and wheel scaling, the reset badge, per-site policy and restart persistence.
