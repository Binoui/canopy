# Tabs and the strip

The tab row holds one tab per tree in Tabs, in stored order with pinned icons first. Under it, the strip draws the current tree: the path to the current page, read-page badges on side branches, and waiting tabs after the current page. Resting the pointer on a page shows its card. The zoomed tree shows the whole tree as a tree or as a list.

## Sub-features

- `strip-switch` switches trees and reopens the page last read in each.
- `strip-card` shows a page's card with its site, age and other branches.
- `strip-keyboard` walks the path with F6, Left, Right and Enter. Delete dismisses a waiting tab.
- `strip-zoom` opens the current tree in a larger view.

## How to get to it (user POV)

- Ctrl+1 to Ctrl+9, or a click on a tab. Shift is also accepted.
- Rest the pointer on a page in the strip.
- F6 moves focus between the page and the strip.
- Ctrl+Shift+E or the button at the right end of the strip.

## Driving it with canopy-verify

Preconditions:

- A fresh stress launch and a clean doctor. Canopy opens on tree 1, "Tokio tutorial · Async in depth".

- **Switch.** Run `$V key ctrl+4`, `sleep 3`, `$V shot strip-tree-b`. The keyboard research tab is active, the page is "NuPhy Halo75 V2", and nine waiting pages appear after the current page as named tabs, icons or a **+N** list, depending on width.
- **Switch back.** Run `$V key ctrl+3`, `sleep 3`. The page is "Tokio tutorial · Async in depth" again.
- **Card.** Run `$V hover 70 68`, `$V shot strip-card`. A card for the root page shows below the strip.
- **Keyboard walk.** Run `$V key F6`, `$V key Left`, `$V key Return`, `$V shot strip-f6-card`. Focus is on "Framing" and its card is open with Open focused. Run `$V key Return`, `sleep 2`, then `$V db "SELECT v.title FROM session s JOIN visits v ON v.id = s.current_id" strip-f6`. It returns "Tokio tutorial · Framing".
- **Zoomed tree.** Run `$V key ctrl+shift+e`, `sleep 1.5`, `$V shot strip-zoom`. A sheet under the tabs reads "Tokio tutorial · Getting started, 10 pages · 0 waiting" with Tree and List switches and the whole tree. `$V key Escape` closes it.

## Gotchas

- Enter on a strip page opens its card first, and a second Enter loads the page. `docs/guide.md` says one Enter loads it.
- Not yet driven by this map: tab clicks, the tab overflow arrow, right-click Rename. Drive them by pointer from a screenshot when your change touches them.
- Wait 3 s after a tree switch. The page view swaps after the chrome redraws.
- The strip folds long paths. The exact folding depends on the tree's shape, so read the screenshot rather than counting positions.
