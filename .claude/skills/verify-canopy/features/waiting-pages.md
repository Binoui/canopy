# Waiting pages

Ctrl+click, a middle click, or a link meant for a new window adds the destination to the tree as a waiting page, drawn dashed, without loading it. The counter at the end of the strip and the badge on the tab count them. Ctrl+Tab opens the newest one.

## Sub-features

- `waiting-add` adds a waiting page by middle-clicking a link.
- `waiting-count` shows the count on the strip counter and the tab badge.
- `waiting-next` opens the newest waiting page with Ctrl+Tab.
- `page-switcher`: with nothing waiting, Ctrl+Tab opens the read page before this one; holding Ctrl and pressing Tab again shows `#switcher` (waiting rows in the left `.switcher-col[data-kind="waiting"]`, read rows in the right one; Tab walks left then right; a click opens a row), and releasing Ctrl opens the selected row.

## How to get to it (user POV)

- Middle click or Ctrl+click a link on a page.
- Ctrl+Tab, or the counter at the right end of the strip.

## Driving it with canopy-verify

Preconditions:

- A fresh stress launch and a clean doctor.

- **Counter.** Run `$V key ctrl+2`, `sleep 3`, `$V shot waiting-before`. The strip counter and the tab badge both read 9.
- **Next waiting.** Run `$V db "SELECT opened_at FROM visits WHERE title = 'Best gaming keyboards 2026'"` (empty), then `$V key ctrl+Tab`, `sleep 3`, `$V shot waiting-next`. The page is "Best gaming keyboards 2026", the newest waiting page under the root, and the counter reads 8.
- **Stored.** Run the same `db` query again with a name, `waiting-opened`. `opened_at` now has a value.
- **Add.** Run `$V click 390 328 middle`, `sleep 2`, `$V shot waiting-add`. The heading does not change, a dashed "1" appears after the current page in the strip, and the counter reads 9 again. Confirm with `$V db "SELECT title, opened_at FROM visits ORDER BY created_at DESC LIMIT 1" waiting-added`, which returns "Best gaming keyboards 2026 1" with no `opened_at`.

## Gotchas

- Ctrl+Tab picks the newest waiting page of the current tree, not the closest one in the strip.
- `$V key ctrl+Tab` is a quick press: it opens the first row and never shows the list. To see the list, hold Ctrl with `xdotool keydown ctrl`, press `key Tab` twice, take the shot, then `keyup ctrl` (which opens the selected row). Shift+Tab under a held Ctrl steps back.
- Under Xvfb, GDK does not mark Shift or Ctrl presses as modifiers (`EventKey::is_modifier` is false), so the code matches modifier keyvals itself.
