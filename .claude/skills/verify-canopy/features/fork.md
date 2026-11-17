# Fork a tree

Each typed address or search starts a tree. Following a link adds a child page. Going back to an earlier page and following another link grows a second branch, and both branches stay. The tree and the current page survive a restart.

## Sub-features

- `fork-new-tree` starts a tree from the new-tree page.
- `fork-follow` adds the followed link as a child of the current page.
- `fork-back` returns to the parent page.
- `fork-branch` adds a second child beside the first.
- `fork-restart` keeps the tree and the current page after Ctrl+Q and a new start.

## How to get to it (user POV)

- Ctrl+T, or the `+` after the tabs, then type an address and press Enter.
- Ctrl+K or the "Search or go…" pill, then Alt+Enter, opens an address in a new tree. Plain Enter adds it as a child of the current page.
- Alt+Left, a click on the parent in the strip, or F6 then Left then Enter goes back.

## Driving it with canopy-verify

Preconditions:

- A fresh stress launch and a clean doctor.
- No page titled `Repotting` yet.

- **New tree.** Run `$V key ctrl+t`, `$V type "$($V url Repotting)"` with `CANOPY_HOST=wiki`, `$V key Return`, `sleep 2`. A tab "Repotting" appears at the end of the tab row, and the strip shows one page.
- **Follow a link.** Run `$V click 390 328` (link 1), `sleep 2`. The page heading reads "Repotting 1" and the strip shows Repotting then Repotting 1.
- **Back.** Run `$V key alt+Left`, `sleep 2`. The heading reads "Repotting".
- **Branch.** Run `$V click 390 354` (link 2), `sleep 2`, `$V shot fork-branch`. The heading reads "Repotting 2", and the root in the strip carries a "1" badge for the other branch.
- **Stored shape.** Run `$V db "SELECT v.title, p.title AS parent FROM visits v JOIN visits p ON p.id = v.parent_id WHERE p.url LIKE '%/p/Repotting' ORDER BY v.created_at" fork-rows`. Two rows, "Repotting 1" and "Repotting 2", both with parent "Repotting".
- **Card.** Run `$V hover 70 68`, `$V shot fork-card`. The root's card lists "Other branches from here · 1" with "Repotting 1".
- **Restart.** Run `$V quit`, `$V start`, `$V shot fork-restart`, then `$V db "SELECT v.title FROM session s JOIN visits v ON v.id = s.current_id" fork-current`. The window opens on "Repotting 2" with the same strip, and the query returns "Repotting 2".

## Gotchas

- Plain Enter in the palette opened with Ctrl+K adds the address under the current page, in the current tree. It does not start a tree.
- Fixture child titles are the parent title plus ` N`, so assert the heading text, not the address.
- Restoring a page reloads it. Scroll position and forms are not restored, by design.
