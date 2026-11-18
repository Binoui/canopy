# Trees

Ctrl+E and Forest open Trees. Three lanes show Tabs, Saved for later and Settled today, with their counts. Each lane scrolls independently. Below 620 px of pop-in width, the lanes stay 300 px wide and scroll sideways. Trees has no Find field.

Tabs follow the stored tab order, with pinned tabs first as compact lines. Other tabs show their miniature, title, current page name, page count and waiting count. The current tree is marked. Each tab offers Pin tab or Unpin tab, Save for later and Settle tree. Saved for later shows shelf order, miniature, title, page count and local save time, with Open and Settle tree. Settled today shows every tree in History's Today group, newest first, with its local settle time and Reopen. All of History switches to History.

Click or Enter on a card resumes its last page and closes the forest. Alt+Enter opens it as the last tab without changing the page you are reading. Pin, Save for later, Settle tree, Rename and drops keep the forest open. Menus open with right-click, Menu or Shift+F10. Their actions follow the tree's current place. Delete appears only in History, including Settled today.

Drag a card to another position in Tabs or Saved for later. Drop among pinned tabs to pin it, among other tabs to unpin it, or at their border to keep its pin. Drop on Saved for later to keep a tree off the bar. Drop on Settled today to settle it. Drop a saved or settled tree on Tabs to reopen at that position while keeping the page you are reading. A tab in the bar can travel to a lane, and a card can travel to the bar. A lane mark and insertion line show the destination. Releasing outside the lanes and bar changes nothing. Escape cancels a drag. A press without movement activates the card.

Up and Down focus cards within a lane. Left and Right focus the closest card in the next lane. Alt+Up and Alt+Down move the focused card within its group. Pinned tabs stay in the pinned group. The shelf has one group. Settled today keeps its time order. Buttons and menus provide each cross-lane move, and Alt+Enter provides reopen without navigation. Tab stays inside the forest and Escape closes it.

The bar, overflow list and Ctrl+1 through Ctrl+9 use the same stored order. New and reopened trees append to Tabs. Unpin leaves a tab at the border after pinned tabs. Ctrl+Shift+Tab returns to the previously shown tab. Ctrl+Shift+PageUp and Ctrl+Shift+PageDown move the active tab within its group through the same movement code as Alt+Up and Alt+Down.

## Verify dragging

Run `forest_trees_smoke` under the project's GUI test command. It drives stepped XTest gestures and checks lane marks, moves, outside release, refresh during a drag, tab-bar exchange, keyboard actions and persisted order. `tabs_smoke` checks the tab bar's overflow, pin rule and keyboard order.

For a private real-window drive, use `canopy-verify drag FROM_X FROM_Y TO_X TO_Y`. An optional fifth argument names a screenshot captured before release, for example `canopy-verify drag 200 180 700 320 mid-drag`. The script writes it to that run's evidence folder. Wayland and touch need separate verification.
