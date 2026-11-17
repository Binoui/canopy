# Loading and Stop

A thin accent line along the bottom of the strip shows foreground loading progress. The current page's attached list offers Stop while that page or its replacement is loading. Stop leaves the last committed page usable.

## Sub-features

- Progress follows the foreground candidate before it replaces the displayed page.
- Escape closes an open chrome surface first, then stops a load.
- The current page's Reload action becomes Stop without losing its focus or pressed action.
- Background preloads stay quiet. Stopping an adopted preload cancels its network load.
- Reload shortcuts always reload.

## How to get to it (user POV)

Open a slow page and watch the line below the strip. Press Escape to stop it. Rest the pointer on the current page's strip item to open its list, then click Stop. Open the palette during a load and press Escape twice to close the palette and then stop.

## Driving it with canopy-verify

Preconditions: Run `$V launch` and `$V doctor`. Set `V=.claude/skills/verify-canopy/scripts/canopy-verify`.

1. Run `$V shot 01-rest` to capture the idle strip.
2. Read the fixture port from the active run's `run.env`. Run `$V open "http://wiki.localhost:PORT/slow/Slow_page"`. The fixture waits 12 seconds before sending headers.
3. Run `$V shot 02-loading` before the response arrives. The old page remains visible with a progress line along the strip.
4. Run `$V key ctrl+k`, then `$V key Escape`. The palette closes and the progress line stays. Run `$V key Escape` again and `$V shot 03-stopped`. The line disappears and the old page stays usable.
5. Open another slow address. Hover the current page's strip pill with `$V hover X 68`, using the last screenshot to choose X. Capture its Stop action, click it, and capture the idle strip.
6. Run `$V db "SELECT current_id FROM session" 04-cursor` and confirm that Stop kept the cursor from the idle page. Run `$V cleanup`.

## Gotchas

- The new-tree page, palette, menus, attached lists, and reader consume Escape before Stop. Close them first, or click Stop directly in the attached list.
- After a commit, Stop keeps the newly committed document. It does not go back to the page before that commit.
- A stopped transport can finish draining after the line disappears. The GUI suite checks immediate logical clearing and eventual native stop separately.
- Run `loading_smoke` under the private Xvfb setup for pre-Started cancellation, native navigation, adopted background work, pressed-action completion, failure, and download handover.
