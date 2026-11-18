# History

Ctrl+H opens History. It holds settled trees, grouped by the local day they were settled, newest first. Day names, month names and clock times stay English under other locales. A shelf tree does not appear here.

Each row shows its settle time, a cropped Canopy miniature, its title, four page names after the root in outline order, remaining page count, sites and total pages. A root-only tree says One page.

Click a row to select it and unfold its outline in place. Click it again to fold it. Only one outline is unfolded. Up and Down select without unfolding. Right or Space unfolds and Left folds. The outline keeps true preorder depth, marks the last page and stops at forty pages with a remaining count. Enter, a double click or Reopen resumes the last page and closes the forest. An outline page reopens that page.

S saves the selection for later. Delete deletes it. The next row becomes selected when a tree leaves the results. Selected and hovered rows show Reopen, Save for later and Delete. The row menu adds Rename.

Find matches titles, sites and the full titles of every page, including the root. Rust sends marked text runs. A matching page outside the first four leads the trail. If its short name conceals the match, the full title appears instead. The count says N of M trees. Escape clears both Find and the site filter, then closes the forest on the next press.

The When rail jumps to Today, Yesterday, Earlier this week, Last week or an older month. Weeks start on Monday. A jump loads twenty rows at that destination rather than the intervening history. Reaching either edge extends that window by twenty, without trimming. The view also loads more after rendering or resizing if the window is not filled. Sites lists the nine hosts held by the most settled trees, counting each tree once per host. Click a site to filter; click again to clear it. Under 620 px the rail becomes site chips beneath Find.

Delete removes a tree's visits, saved text, origins, events and download records immediately. Bookmarks and downloaded files remain. Deleted trees stop contributing suggestions. Delete day is offered only with neither Find nor a site filter active. Its operation covers the whole day, including unloaded rows, and the feedback names the count.

The forest shows Deleted with Undo from the successful deletion reply. Undo restores the stored rows and stays available until the next delete or the forest closes. Quitting loses Undo. Deletion needs no later cleanup. A failed restore keeps the record available for retry.

## Fixtures and measurement

`scripts/spread-forest-history.py SOURCE DESTINATION` copies a fixture into a new file and spreads History settle times over 548 days through the run's local date. It accepts both schema 14 and schema 16. It refuses an existing destination and never edits the source. Use the original fixture for comparisons against main, and the spread copy for day groups, jumps and Delete day.

The opt-in `forest_latency_smoke` accepts `FOREST_LATENCY=1 FOREST_HISTORY=1 FOREST_FIND=1 FOREST_FIXTURE=...`. Opening and Find measure wall time immediately before real xdotool input to the second animation frame after the expected History results appear. First open and reopen are separate. Find prints the input injection span separately.

Release measurements on the 2,000-tree fixture keep opening and one Find letter within the spread on main without a retained cache. Each call builds a transient index. Page order and lowercased titles fill lazily within that call. Unchanged miniatures retain their DOM nodes across Find paints, with site icon revisions included in their paint identity. Raw trials and verification reports stay in the builder's `.cache/` evidence.
