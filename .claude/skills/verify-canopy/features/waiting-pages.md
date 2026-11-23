# Waiting pages

Ctrl+click, a middle click, or a link meant for a new window adds a waiting tab after the current page. A hollow unread mark shows that the page has yet to load. A full dot shows that it has loaded in the background. Waiting descendants of the current page come first, newest first. Other waiting pages follow in their own newest-first group.

## Sub-features

- `waiting-add` queues a link, briefly pulses its strip tab or the overflow button, and announces the queued link.
- `waiting-next` opens the first waiting tab by click, Enter or Ctrl+Tab.
- `waiting-dismiss` dismisses a tab by middle click, its visible cross, Delete, or Dismiss on its attached card or page menu.
- `waiting-overflow` opens the complete queue from **+N** on click, keyboard Enter, or a 120 ms pointer rest.
- `page-switcher` opens the previous read page with Ctrl+Tab when the queue is empty. Keep Ctrl held to open the filmstrip. It shows the current page, up to five waiting pages, and up to eight recently read pages. Pages in memory show a snapshot. Tab and Right step forward, and Shift+Tab and Left step back. Rest the pointer on a thumbnail to select it. Click or release Ctrl to open the selected page. Delete dismisses a selected waiting branch. Escape closes the filmstrip.

## Driving it with canopy-verify

Start with a fresh stress launch and a clean doctor.

- **Queue.** Switch to keyboard research with `$V key ctrl+4`. Inspect the waiting tabs at 1240, 712 and 390 px. Click **+N** when present and compare its rows to the tab order. The first row shows **Ctrl Tab**. Escape returns focus to the overflow button.
- **Next waiting.** Read the first waiting tab and use `$V key ctrl+Tab`. Verify the displayed page matches that tab and the stored visit has a value in `opened_at`. Use F6, Right and Enter to open another tab.
- **Add.** Middle-click a fixture link. Confirm it appears first with a dashed line from the current page or its read count. Read the newest visit in the private database and check that `opened_at` is empty.
- **Actions.** Rest on a waiting tab to see its title, site, source, Open and Dismiss. Right-click or use Shift+F10 for the page menu. Dismiss different tabs with the cross, middle click and Delete. Verify Delete moves focus to the successor or the current page.


- **Filmstrip.** With the fixture page open, use `$V keydown ctrl Tab` then `$V keyup Tab`. Keep Ctrl down. Compare the first waiting thumbnail to the strip and check its large preview. Use `$V keydown Right` and `$V keyup Right` to step. Rest the pointer on another thumbnail and verify its preview. Use `$V keydown Delete` and `$V keyup Delete` on a waiting card. Check the successor and count, then use `$V keyup ctrl` to open it. Repeat at 720 and 1920 px, and at 300 px tall to check that the row is still visible. Check both themes.
