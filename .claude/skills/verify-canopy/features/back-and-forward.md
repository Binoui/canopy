# Back and forward

Back returns to the parent page. Forward opens the child shown most recently, even after a restart or when that branch was settled. Alt+Left and Alt+Right work from chrome overlays too. Mouse Back and Forward work over the page unless it cancels the mouseup event. The ← and → buttons at the left of the strip run the same commands and name their target in the tooltip.

## Sub-features

- `back-parent` returns to the parent with Alt+Left or mouse button 8.
- `forward-child` returns to the last shown child with Alt+Right or mouse button 9.
- `forward-cold` uses the recorded tree after a restart, without WebKit forward history.
- `forward-uncover` closes chrome overlays when Alt+Right opens the child.
- `nav-buttons` greys out Back on a root and Forward with no shown child; clicking them goes where the keys go.

## How to get to it (user POV)

- Follow a link, then press Alt+Left or mouse Back over the page.
- Press Alt+Right or mouse Forward to return to that child.
- Repeat after quitting and restarting on the parent page.
- Hover the ← and → buttons at the left of the strip to read where they go, then click them.

## Driving it with canopy-verify

Preconditions:

- A fresh launch with `--seed empty` and a clean doctor.
- `$V` is `.claude/skills/verify-canopy/scripts/canopy-verify`.

1. Run `$V open "$($V url Forward_parent)"`, wait for the page, then `$V click 390 328` to follow link 1.
2. Run `$V key alt+Left`, then `$V quit` and `$V start`. The parent reloads into a fresh view.
3. Run `$V hover 390 500`, then send `xdotool click 9` on the run's private display. `$V env` prints `DISPLAY_NUM` and `RUN`. Use `env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY_NUM" GDK_BACKEND=x11 XDG_RUNTIME_DIR="$RUN/rt" xdotool click 9` after sourcing that run's `run.env`. The driver has no side-button click command.
4. Run `$V shot forward-cold`, then `$V db "SELECT v.title FROM session s JOIN visits v ON v.id=s.current_id" forward-cold-current`. Both show `Forward parent 1`.
5. Send `xdotool click 8` on the same private display. Run `$V shot back-parent` and the same current-page query. Both show `Forward parent`.
6. Run `$V key ctrl+e`, then `$V key alt+Right` and `$V shot forward-uncover`. The forest closes and the child page is visible. Save the current-page query again.
7. Run `$V hover 23 68` and `$V shot nav-back`, then `$V click 23 68`. The parent is current; the ← button is greyed out on it and → names the child. `$V click 53 68` returns to the child. Save the current-page query after each click.
8. Run `$V cleanup`.

## Gotchas

- Forward follows the most recently shown child, not a waiting child or an arbitrary branch. It can reopen a settled branch.
- The buttons give way in windows too narrow to leave the tree 280 px; the keys still work there.
- Mouse buttons must be over the page. Chrome overlays receive their own input.
- Pages still receive synthetic mousedown and mouseup events. Cancelling mouseup suppresses navigation. `profile_smoke` checks their coordinates at zoom 1.5; `history_smoke` checks cancellation with a real forward child.
