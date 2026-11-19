---
name: verify-canopy
description: Drive a private Canopy window on its own Xvfb display and prove a change the way a user would see it, with screenshots and database reads as evidence. Use when reviewing or verifying a Canopy change in the real app, reproducing a chrome bug, looking at big seeded trees, or running the full check (fmt, clippy, unit tests, every gui-smoke suite) before a merge.
---

# Verify Canopy

Canopy is a GTK and WebKitGTK desktop browser. Its chrome (tabs, tree strip, palette, forest) is an HTML view with no DevTools port, so you prove behavior with real keyboard and pointer input through `xdotool`, screenshots, and read-only queries on the run's own `forest.sqlite3`.

Every command below goes through one script. Run it from the repository root:

```sh
V=.claude/skills/verify-canopy/scripts/canopy-verify
```

Shell variables do not survive between tool calls in most agent harnesses, so either repeat the `V=` assignment in each call or type the full path.

## Launch

```sh
$V launch                 # builds target/debug/canopy, seeds the stress trees
$V launch --seed empty    # first-run state, no trees
$V launch --bin ~/.local/bin/canopy   # the installed build instead of the checkout
```

`launch` creates a run folder `~/.cache/canopy-test-rt/v-<id>/` with a private `XDG_RUNTIME_DIR` (`rt/`) and data folder (`data/`), picks a free X display from `:200` up, starts the fixture site on a free port, and starts Canopy. With `--seed stress` (the default) it starts Canopy once to create the database, quits it with Ctrl+Q, runs `fixtures/seed.py`, and starts it again. Ready means the Canopy window is visible on the run's display, 2 s after it first appears. The run is recorded in `.cache/verify/current`, so later commands find it without arguments. Set `CANOPY_RUN=<run folder>` to address a different run.

The stress seed is 129 pages in 16 Tabs and 3 History trees. The pinned Ask HN and Canopy pull-request tabs come first. Ctrl+3 is Tokio tutorial, Ctrl+4 is the keyboard research tree with 9 waiting pages, and Ctrl+5 is Bonsai with a 46-page trail. Ask HN has 14 waiting pages under its root. Other tabs fill the row so it overflows. Canopy starts on Tokio.


## Doctor

```sh
$V doctor
```

Read-only. It prints one `ok` or `FAIL` line per check: Xvfb owns the display, the fixture site answers, the Canopy process runs the recorded binary on the run's data folder, the instance socket exists, the window is visible, the database reads, and the binary is newer than the last commit. Run it before the first drive and again after any drive that surprised you. A failed doctor means relaunch, not retry: `$V cleanup && $V launch`.

## Drive

Prefer keyboard shortcuts. They are stable handles, and pointer positions move whenever the chrome changes. Shortcuts are listed in `README.md` under Keyboard and in `docs/guide.md`.

```sh
$V key ctrl+k                     # xdotool key names: ctrl+shift+e, alt+Left, Return, Escape, F6
$V type "http://wiki.localhost:PORT/p/Repotting"
$V drag 150 26 350 26            # stepped native tab drag
$V wheel 500 350 up 5            # native wheel notches in the map
$V click 390 328 [left|middle|right]
$V hover 70 68 [seconds]          # rests the pointer, 1.2 s by default
$V url Repotting                  # fixture address for a page title; CANOPY_HOST=wiki picks the host
$V open "$($V url Something)"     # hands an address to the running window like another app would
$V quit && $V start               # Ctrl+Q, then start again on the same data folder
$V log 40                         # last lines of Canopy's own output
```

Each drive command focuses the Canopy window first and waits a moment after acting. Wait longer after loading a page (`sleep 2`) or switching trees (`sleep 3`).

The fixture site answers any `*.localhost` host and any `/p/<Title_with_underscores>` path with a page titled from the path, plus eight links named "A linked page number N". A child page's title is its parent's title plus ` N`.

Pointer positions in the default 1240x800 window, measured on `78dbbd1`. Check against a screenshot before relying on them.

| Target | x, y |
| --- | --- |
| Tab row | y 26. Forest button at 48, search pill at 1100, settings gear at 1218 |
| Strip, first page | 70, 68 |
| Waiting counter, zoomed-tree button | 1172, 68 and 1217, 68 |
| Fixture page link N | 390, 328 + 25.7 × (N − 1), so 328, 354, 379, 405 |

## Evidence

```sh
$V resize 1920 1000                       # resize the private window
$V shot 03-after-fork                       # PNG of the whole display
$V db "SELECT title FROM visits ORDER BY created_at DESC LIMIT 3" 04-rows   # read-only query, saved as text
```

Both write into `.cache/verify/<run id>/`, which git ignores and cleanup keeps. A proof captures the user action and the state after it, and confirms the stored side effect with a `db` read, not only the screen. Number the files in the order you took them. Drive the real path (shortcut, click, `open`). Never write to the database to set up a journey beyond what `seed.py` does.

The forest schema is version 16. Older installed builds refuse it. Tables you will read most: `visits(id, parent_id, url, title, created_at, opened_at)` with `opened_at` NULL for a waiting page, `tree_state(root_id, last_visit_id, name, place, position, pinned, placed_at)`, `session(current_id)`.

## Cleanup

```sh
$V cleanup
```

Kills only the Canopy, fixture site and Xvfb processes this run started (by recorded pid), copies `canopy.log` into the evidence folder, and deletes the run folder. Run it after every run, failed ones included. Evidence stays in `.cache/verify/<run id>/`.

## Full check

```sh
.claude/skills/verify-canopy/scripts/check.sh [CHECKOUT]
```

Runs `cargo fmt --check`, clippy with `gui-smoke`, the unit tests, every `*_smoke` suite in `Cargo.toml` one at a time under `xvfb-run`, and a clean-worktree check, with one line per check. It takes about 15 minutes. Run it once before a merge that touches `src/chrome/`, `src/ui/` or the store. When a suite fails, run the same suite on the base commit before calling it a regression, and rerun a lone timeout once.

## Feature map

`features/README.md` lists the user-facing features with a recipe for each. Use it to choose what to drive for a change, and drive every entry point it lists for the feature you claim to have verified.

## Gotchas

- Never start Xvfb with `-displayfd` or a low display number. On this Hyprland desktop it takes over `:0` and breaks every new X11 app on the real session. `launch` picks from `:200` up for that reason.
- Never put `XDG_RUNTIME_DIR` or the data folder under a long path. WebKit binds sockets inside both, and socket paths stop at 108 characters.
- Never kill by name (`pkill -f Xvfb`). Other sessions run their own Xvfb and Canopy at the same time, and a `pkill -f` pattern can match its own shell.
- `seed.py` writes the tables directly. After a schema change it fails at launch with the SQLite error. Update `fixtures/seed.py` to the new schema.
- Seeded pages have no favicons, so the chrome draws letter tiles. That is real behavior.
- Ctrl+K from a page opens the palette "From" that page, so Enter adds the address as its child. Use Ctrl+T, or Alt+Enter in the palette, to start a new tree.
- `$V db` reads while Canopy runs. Recent writes appear after Canopy commits them, so wait a second after the action.
