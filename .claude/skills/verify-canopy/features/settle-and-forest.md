# Settle and the forest

Settling a tree takes its tab away and keeps every page, waiting ones included. A toast offers Undo. The forest view lists every tree and keeps settled ones under Settled, where opening a page brings the tab back.

## Sub-features

- `settle-tree` settles the current tree.
- `settle-undo` brings it back from the toast.
- `forest-view` shows open and settled trees with their page counts.

## How to get to it (user POV)

- Ctrl+W, a middle click on a tab, the × on a tab, or right-click a tab then Settle tree.
- Undo in the toast, or Ctrl+Shift+T.
- Ctrl+E or the Forest button at the left of the tab row.

## Driving it with canopy-verify

Preconditions:

- A fresh stress launch and a clean doctor.

- **Settle.** Run `$V key ctrl+2`, `sleep 3`, `$V key ctrl+w`, `sleep 2`, `$V shot settle-toast`. The "best 75%" tab is gone, and a toast reads "Settled 'best 75% mechanical keyboard 2026 at Search' · 9 waiting kept" with Undo.
- **Stored.** Run `$V db "SELECT t.settled_at IS NOT NULL AS settled FROM tree_state t JOIN visits r ON r.id = t.root_id WHERE r.title LIKE 'best 75%'" settle-row`. Returns 1.
- **Forest.** Run `$V key ctrl+e`, `sleep 2`, `$V shot settle-forest`. The header counts one more settled tree than before (4 settled on the stress seed), and the current tree is listed first.
- **Undo.** Right after the settle, run `$V key ctrl+shift+t`, `sleep 2`, `$V shot settle-undo`. The tab returns on the page it was on, and the same query, saved as `settle-undo-row`, returns 0.

## Gotchas

- After settling, Canopy moves to another open tree. Do not assume which one.
- With the palette, new-tree page or forest open, Ctrl+W closes that overlay instead of settling. Press Escape first.
- The toast stays on screen for a while after Undo. Assert the tab and the query, not the toast.
