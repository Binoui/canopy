# Find in page

Ctrl+F opens a find bar at the right of the tab row, inside the 42 px header. Typing highlights the matches in the page and the bar counts them, such as "3 of 8", or says "No matches". Enter and Shift+Enter step through the matches and wrap. Escape closes the bar and clears the highlights. Find belongs to the page it was opened on: switching pages or loading another document closes it.

## Sub-features

- `find-open` opens the bar with the keyboard in it and counts the matches as you type.
- `find-step` moves to the next or previous match and wraps at both ends.
- `find-keys` steps from anywhere with Ctrl+G, Ctrl+Shift+G, F3 and Shift+F3, also after F6 gave the page the keyboard.
- `find-close` closes the bar with Escape or its cross, and clears the highlights.
- `find-context` closes the bar when the page switches or loads another document.

## How to get to it (user POV)

- Ctrl+F from the page or the chrome. Ctrl+F again selects the text in the bar.
- Enter and Shift+Enter in the bar, or the arrows in the bar.
- Ctrl+G, Ctrl+Shift+G, F3 and Shift+F3 anywhere in the window.
- Escape, or the cross at the end of the bar.

## Driving it with canopy-verify

Preconditions:

- A fresh stress launch and a clean doctor. Canopy opens on "Tokio tutorial · Async in depth". The fixture page has 8 links named "A linked page number N", so "linked" matches 8 times, and 60 copies of "Filler paragraph for scrolling".
- `$S` below is a shot that repaints first: resize the window by one pixel and back with `xdotool windowsize`, then `$V shot`. See Gotchas.

- **Open and count.** Run `$V db "SELECT count(*) FROM visits" find-before`, `$V key ctrl+f`, `$V type linked`, `$V key F6`, `$S find-open`. The search pill shrinks to its icon, the bar reads "linked · 1 of 8", and the eight words are yellow in the page.
- **Step and wrap.** Run `$V key ctrl+f`, then `$V key Return` twice and `$V key F6`, `$S find-step`. The bar reads "3 of 8" and the third link's word carries the selection. `$V key shift+F3` three times reads "8 of 8"; `$V key F3` reads "1 of 8".
- **No matches.** Run `$V key ctrl+f`, `$V type zebra`, `$V key F6`, `$S find-none`. The bar reads "No matches" and its arrows are grey.
- **Close.** Run `$V key Escape`, `$S find-closed`. The bar is gone, the pill is back and no word is yellow. `$V db "SELECT count(*) FROM visits" find-after` returns the count from `find-before`: find adds no visit.
- **Switch closes.** Run `$V key ctrl+f`, `$V type linked`, `$V key ctrl+2`, `sleep 3`, `$S find-switch`. Tree 2 is shown and the bar is closed.

## Gotchas

- WebKitGTK 2.54 on Xvfb paints only the part of the chrome that last changed, so a plain `$V shot` can show a black header. Resizing the window by one pixel and back repaints all of it. The bar's blinking caret repaints only itself, so give the page the keyboard with F6 before the shot, or shoot within about 0.1 s of the resize to keep the focus ring.
- Canopy counts the position itself: WebKit reports only the total. If a page adds or removes matches while you step, the count starts again at the first match.
- Matching ignores case.
- Keys sent less than a millisecond apart can overtake each other between the window and the chrome. Leave the default `$V` delays in place.
