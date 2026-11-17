# Change page size

Page size changes text, images and links together. Canopy remembers it per site across trees, waiting pages and restarts. A percentage button appears at sizes other than 100%; clicking it resets the site.

## Sub-features

- Keyboard increase, decrease and reset, including keypad and shifted plus.
- Ctrl+wheel over the page, one scale step per wheel notch.
- Site policy shared by all views with the same scheme, host and port.
- A percentage button in the header, visible only away from 100%.

## How to get to it (user POV)

- Press Ctrl+=, Ctrl+plus or Ctrl+Shift+plus to increase size, or Ctrl+- to decrease it. Keypad Add, Equal and Subtract work too.
- Press Ctrl+0 or Ctrl+keypad 0 to reset.
- Hold Ctrl while turning the vertical wheel over a page.
- Click the percentage in the header to reset.

## Driving it with canopy-verify

Preconditions:

- A fresh stress launch and a clean doctor.
- The Tokio page is loaded. `$V` is `.claude/skills/verify-canopy/scripts/canopy-verify`.

1. Run `$V key ctrl+equal` three times, then `$V shot scale-150-dark`. The header reads 150%, and the page is larger while chrome stays the same size.
2. Run `$V db "SELECT origin,percent FROM site_page_scale" scale-policy`. The displayed site's row reads 150.
3. Run `$V key ctrl+1`, then `$V key ctrl+2`. The same site retains its size; a different site starts at 100% with no badge.
4. Return with `$V key ctrl+1`. Run `$V quit`, `$V start`, then `$V shot scale-restart`. The page still reads 150%.
5. For the wheel, read the run's `run.env` to get its display and use `DISPLAY=... xdotool mousemove 600 500 keydown ctrl click 5 keyup ctrl`. The badge changes once, to 125%. Repeat with button 4 to return to 150%.
6. Take a screenshot to locate the percentage button, then run `$V click X Y`, `$V shot scale-reset`, and `$V db "SELECT origin,percent FROM site_page_scale" scale-reset-policy`. The badge disappears and the site's row is removed.
7. In Settings, choose Light and repeat the increase and reset captures. Switch back to Dark and repeat. Both themes keep the header legible.

## Gotchas

- Ctrl+wheel on the forest changes its camera, not page size. Point below the strip to change the page.
- WebOrigin includes the scheme and port. Two fixture servers on different ports do not share a policy.
- The screenshot cannot prove that WebKit did not add a second wheel step. `page_scale_smoke` checks the native value immediately and after a delay.
- The scale suite also checks hidden and background views, checkpoint revival and redirects. `profile_smoke` checks buttons 8 and 9 at 150% with prevented page events.
