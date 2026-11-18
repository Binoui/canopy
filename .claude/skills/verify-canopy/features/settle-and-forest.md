# Settle and the forest

Settling removes a tab and keeps every page, including waiting pages. Undo returns it. Saving and settling are independent.

## Sub-features

- `settle-tree`: Ctrl+W, tab ×, middle click or tree menu; Undo or Ctrl+Shift+T restores it.
- `forest-view`: Ctrl+E or Forest opens Trees, with Open cards in tab order and Saved cards in saved order. Ctrl+H opens History, switches from Trees, or closes History.
- `forest-history`: settled unsaved trees grouped by local last-use day; select a row for a miniature and a capped preorder outline. Reopen/Enter/double-click resumes its last page; an outline click opens that page. Save keeps it settled, moves it to Saved and selects the successor.
- `forest-find`: typing anywhere searches titles and every site in both views. Escape clears text first, then closes; close clears the query.

## Drive with canopy-verify

Use a stress launch and doctor. At 1240px, open Ctrl+E; inspect Open and Saved, click a card, then reopen. Use arrows and Tab. Menu, Shift+F10 and right-click open Rename, Save tree/Remove from saved, and Settle tree for open trees. All mutations keep the forest open, including settling the current tree.

Open Ctrl+H. Select a history row, use Up/Down and inspect the preview; exercise Reopen, Save and an outline page. Find a title and a site; check no matches and the two Escape presses. Resize to at least 1920px and below 700px pop-in width; the preview stacks above the list at narrow width. Capture dark and light at each size, resizing by one pixel and back before capture if WebKit leaves a black band. On large fixtures, Show older trees or keyboard traversal extends the loaded rows by 60.

Check saved/settled state with read-only `tree_state` queries. A saved settled card also restores from the new-tree page after a restart. With no history a hint explains it; no saved trees has a separate saving hint. The forest keeps 22px margins and no width cap. Closing returns focus to its trigger or the page; Ctrl+W closes the forest before settling.
