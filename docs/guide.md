# Using Canopy

This guide covers everything Canopy does today, feature by feature. For installing and a first run, see the [README](../README.md).

A fresh profile shows **How trees work** above Saved for later on the new-tree page. **Got it** hides the card across restarts. **Settings → How trees work** or **F1** opens the new-tree page and shows it again. Its legend explains waiting rings, unread green dots, still-open grey dots, download leaves and other-branch counts.

## Try a fork

1. Press **Ctrl+K**, type an address and press **Enter**.
2. Follow a link to a second page.
3. Go back to the first page by clicking it in the strip.
4. Follow another link. Both visits appear under the first page.
5. Close the browser, then start it again with the same data folder. The tree and the current page are kept.

Returning to a page still open preserves its forms, scripts and scroll position. Released pages reload when reopened. A restart restores the recorded scroll position, but not forms or scripts.

On a page's attached list, **New branch here** opens the palette from that page without loading it, so the next address you type becomes its child. A tree's tab reopens the page you were last reading in that tree.

## Find your way around

The top row has one tab per open tree, in stored tab order, with pinned tabs first. The active tree keeps its title, up to 200 px. Other tabs share the remaining room as short titles, then become icons one at a time, starting farthest from the active tree. Icon tabs show a green waiting count. Trees that still do not fit wait behind the arrow at the end of the row. Their rows show waiting counts, pin marks and a settle button. A tree is named after what you searched for, or after its first page. Right-click a tab to give the tree a name of your own; an empty name goes back to the automatic one. Under the tabs, a 52 px strip starts with Back and Forward buttons and shows the path to the current page. The root keeps its tile, the parent has a short name, and the current page keeps its full title. Consecutive pages on one site fold into a capsule with their count. Between the root and the parent, at most three items nearest the parent remain visible. Older items merge into **Earlier**. More items fold only when room runs out. A count below an item includes all pages on its other branches. A green outline means some are waiting. Grey and green counts after the current page show its read and waiting children.

Rest the pointer on an item for a quarter of a second, or focus it, to open one named list attached directly below it. The strip stays the same height. The list stays open as you move into it. It closes when focus moves outside the item and its list or back to the page. Page lists show the full title, site, time, saved-text preview, and other branch heads, newest first. The current page lists read children; its waiting children appear under After. Rows drop shared title prefixes and site names, with the full title in their tooltip. A capsule lists its pages in order. **Earlier** lists its pages newest first. The count after the current page lists waiting children in the order they were queued. Lists show up to six rows. **More** opens the zoomed tree. From **Earlier**, it brings the earliest hidden page into view. Click a row to open its page. **Open**, **Reload**, and **New branch here** act on the page named in the list head.

The Bookmarks band shows up to sixteen top-level tiles, with folder chips to the right of its heading. Overflow tiles and chips open lists. Right-click a tile or list row to remove its bookmark.

The tab row has one Tab stop. **Left** and **Right** move between visible tabs, and **Home** and **End** move to the ends. **Tab** from the current-page strip item reaches **Saved text** next. **F6** from a header control, strip item or strip-list row returns to the page.

Chrome menus take focus when opened. Arrow keys walk menu actions, Home and End move to the first and last action, and Tab stays inside the menu. Escape returns to the trigger. Canopy, saved text and search also keep Tab inside their overlay and return focus when closed. Focus has a text-coloured outline distinct from selection. On the waiting counter, Down opens the waiting list.

Window shortcuts require the listed modifiers. Caps Lock and Num Lock do not affect them.

- **Ctrl+K** or the **Search or go…** pill opens the palette. Type to search your forest and your saved text, or to go to an address. An address or web search grows a child of the page where you opened the palette. From the new-tree page it starts a new tree. **Enter** opens the selected result. **Alt+Enter** opens it in a new tree. **Escape** closes the palette and returns focus to the control that opened it, or to the page.
- **Ctrl+D** bookmarks the current page or removes its bookmark, including while the new-tree page shows over it.
- **Ctrl+L** opens the same palette with the current address selected.
- **Ctrl+E** or the Canopy button opens the map on the current tree. Wheel to zoom, drag to move, and click a page or folded site to go there. **Back to** returns to the page you just left. **Ctrl+H** opens **History**. The shortcuts switch between the views and close the one already showing. See [Canopy](canopy.md) and [History](forest-history.md).
- **Ctrl+T** or **+** opens a selected **New tree** tab with an empty strip. Type immediately to search in a new tree, or click its search field. **Saved for later** lists the shelf in a three-column band above bookmarks and up to three suggestions from your history. Opening a shelf card resumes its last page and removes it from the shelf. Suggested pages plant a new tree, suggested trees reopen where you left them, and suggested paths load a root and its next page. The reason appears under each title. Hover a chip to **Keep** it as a bookmark or a tree on the shelf, or use the cross to stop suggesting it permanently. Keep on a path bookmarks its first page. Suggestions stay in place until you leave the page. The empty search palette lists the same suggestions in the same order. Chrome history fills spare places while Canopy has little history. **Escape**, **Ctrl+W**, or the New tree tab's **×** returns to your page. The cross appears when there is a page to return to.
- **Menu**, **Shift+F10** or right-click on a tab offers **Pin tab** or **Unpin tab**, **Save for later**, **Rename** and **Settle tree**. Pinned tabs are icons at the front of the bar. Save for later removes the tab and keeps its tree on the shelf. Opening it again appends an unpinned tab. The shelf menu offers **Open**, **Rename** and **Settle tree**. The History menu offers **Reopen**, **Save for later**, **Rename** and **Delete**. Delete removes visits, saved text and download records; bookmarks and files on disk stay. **Undo** restores the last deletion until the forest closes or another delete replaces it.
- **Ctrl+W**, a middle click or a visible tab **×** settles a tree, including a pinned tab. It leaves Tabs and enters History, keeping every page. A tree leaving Tabs loses its pin. With the new-tree page, forest or palette open, Ctrl+W closes that surface first.
- **Ctrl+Shift+T** reopens the most recently settled tree or branch, even after its toast disappears. The toast shows this shortcut beside **Undo**. Opening a new tree dismisses the visible toast; the shortcut still works.
- **Ctrl+1** to **Ctrl+9** switch to a tab. Shift is also accepted for layouts such as AZERTY.
- **F6** moves the keyboard focus between the page and the current page in the strip. **Left** and **Right** walk the strip items, and **Down** or **Enter** enters the item's list. **Up** and **Down** move between its rows. **Escape** returns to the strip item.
- **Alt+Left**, mouse Back or the **←** button at the left of the strip goes back to the parent page. **Alt+Right**, mouse Forward or the **→** button opens the child shown most recently, however you reached it. Each button's tooltip names the page it opens, and it is greyed out when there is none: Back on a tree's first page, Forward before any child has been shown. Back returns to the page you came from when that page is still in this tree, so after **New branch here** on an earlier page it names where you were, not the parent. In a window too narrow to fit them beside the tree, the buttons give way and the keys still work. **Alt+Up** and **Alt+Down** open the previous and next sibling, newest first, skip dismissed pages and settled branches, and stop at either end.
- **Ctrl+Shift+Tab** switches to the previously shown open tree. Repeating it alternates between the two trees. Settled trees are skipped.
- **Ctrl+Tab** opens the next waiting page of the current tree, newest first. The counter at the end of the strip shows how many are waiting. Queuing a link pulses that counter and the After item once, unless reduced motion is enabled, and announces **Waiting · link text** for 2.6 seconds. Several links queued together show **N waiting from this page**.
- **Ctrl+Shift+E**, or the button at the end of the strip, opens the current tree workbench. It opens as a List if the Tree would scroll, otherwise on the side you used last. Both sides keep branches in the same order when you change pages. Older side branches appear directly below their parent, oldest first and indented one level. The newest child follows at its parent's indent. The current path stays highlighted. **Up** and **Down** move focus, **Left** folds a branch, **Right** unfolds it, and **Enter** opens the focused page. Folded branches show page and waiting counts. Settled branches start folded. Type to filter the names shown and their sites, or turn on **Waiting only**. Filters include ancestors so matches keep their context and temporarily reveal folded matches. **Delete** dismisses waiting pages. **Menu** or **Shift+F10** opens the same page menu as right-click. Copying, dismissing, settling, and folding keep the workbench open. **Escape** clears the name filter and **Waiting only** first. Press it again to close the sheet and return focus to a visible keyboard trigger, or to the page. Fold choices last for this session. Closing clears both filters. **Space** on a row does nothing. The Tree keeps chains horizontal and drops older side branches below. The wheel scrolls vertically; **Shift+wheel** and horizontal wheel input scroll horizontally. A List forced by size does not replace your chosen side on smaller trees. **Ctrl+1** to **Ctrl+9** close the sheet and select a tab.
- **Ctrl+R**, **F5**, and **Ctrl+Shift+R** reload the current page. Canopy has no hard reload, so all three perform a plain reload.
- **Escape** closes the open chrome surface first. Next it closes the find bar, then dismisses an error while the chrome has focus, then stops the current load. Each press does one thing. A thin line along the bottom of the strip shows loading progress, including a page loading off-screen before it replaces the one you are reading. The current page's attached list shows **Stop** while loading and **Reload** afterwards. Stopping keeps the last committed page usable.
- **Ctrl+Shift+L** fills the saved login for the page.
- **Ctrl+F** opens a find bar at the right of the tab row. Matches are highlighted as you type, and the bar shows which one is selected, such as **3 of 12**, or **No matches**. Matching ignores case. **Enter** and **Shift+Enter** in the bar, or **Ctrl+G**, **Ctrl+Shift+G**, **F3** and **Shift+F3** anywhere, go to the next or previous match and wrap at either end. A click in the page does not move the count: the next match follows the one counted. If the page changes and the number of matches with it, the count starts again at the first match. **F6** gives the keyboard back to the page and leaves the bar open. **Ctrl+F** again selects the text in the bar. **Escape** closes the bar and clears the highlights. Find belongs to the page it was opened on: switching pages or loading another document closes it. An address change inside the same document, such as a fragment, keeps it open.
- **Ctrl+Q** quits Canopy.

Drag a tab in the bar to reorder it. A drop among pinned tabs pins it, a drop among other tabs unpins it, and a drop on their border keeps its pin. **Ctrl+Shift+PageUp** and **Ctrl+Shift+PageDown** move the active tab within its pinned or unpinned group. The order survives a restart.

## Change page size

Press **Ctrl+=** or **Ctrl+plus** to make the page larger, and **Ctrl+-** to make it smaller. Shifted plus works too. The keypad's Add and Subtract keys do the same. Hold **Ctrl** and turn the wheel over the page to change its size one step at a time, from 25% to 500%.

A percentage button appears in the top row when the size is not 100%. Click it, or press **Ctrl+0** or **Ctrl+keypad 0**, to reset to 100%. On AZERTY, **Ctrl+à** and **Ctrl+Shift+0** also reset.

Canopy remembers the size for each site, including after a restart. Pages on the same scheme, host and port share it across trees and waiting pages. Different paths share a size; a different scheme or port has its own. The tabs, strip and forest keep their usual size.

## Act on a page or branch

Press **Menu** or **Shift+F10** on a focused page in the strip, an attached list, the zoomed tree or the waiting list. A right-click opens the same menu. The keys also open menus on saved cards and bookmarks. Its heading shows the page icon and short name. Up and Down move between menu rows, Home and End select the first and last rows, Enter runs the focused row, and Escape returns to the trigger. The same menu offers **Open**, **Open in a new tree**, **New branch here**, and **Copy address**. The remaining rows appear when they apply.

**Dismiss** marks a waiting page as skipped. **Dismiss N waiting** skips the waiting pages inside a branch. Skipped links stay hollow and faint in the tree. They leave the waiting counts and **Ctrl+Tab**, and their background views stop. Opening a skipped page clears its mark.

**Settle branch** puts away a page and everything below it. The list and zoomed tree show an **N pages put away** stub. Those pages leave the strip counts and waiting queue, and their live views are checkpointed and released. If you are reading inside that branch, Canopy first returns to its nearest unsettled ancestor. Opening any page inside reopens every settled ancestor. Pages already inside a settled branch offer no Settle action.

**Move to its own tree** detaches a branch into a new tab. The moved head becomes open, while settled branches below it stay put away. It appears for a non-root page when its branch contains an opened page. The new tree remembers the most recently opened page in that branch. If you are reading inside it, the new tab becomes active. Live pages follow the move, and Back stays inside the new tree.

Settling a tree or branch shows **Settled ‘name’ · N waiting kept**. The toast appears at the bottom of the window. **Undo** stays available for six seconds and returns to the page you were reading. Waiting links are kept when you settle.

## Resume reading

1. From a comment or an article, open several links with **Ctrl+click**, a **middle click** or a link meant for a new window. The destinations wait in the tree. The source page stays open. Waiting pages may load silently in the background when memory allows. They stay waiting until you open them. A hollow green ring means not loaded; a filled green dot means loaded and waiting. A small grey dot marks a read page still open.
2. Click a waiting page to read it. The visit keeps its parent and the link's context when it is available.
3. Let **Remember pages** save the text of the open page, or select a passage and press **Ctrl+S**. These live in the **Saved text** menu, behind the dot at the end of the current page in the strip. **Remember selection** in that menu does the same as Ctrl+S.
4. Close the browser, then start it again with the same data folder.
5. Search a few words of the saved text with **Ctrl+K**. Open a **Saved text** result to read the passage offline, or go back to the source to find the links that are still waiting.

Results cover titles, URLs, saved text and source context. Saved text carries its date. Select reader text and right-click for **Copy**. Reading saved text and loading the live page are two separate actions. On the current page's attached list, **Back to source** and **Open comment** lead back to where the page was found.

Turn off **Remember pages** in the Saved text menu to stop automatic capture. Remembering a selection stays available. **Read saved text** shows the text saved for the current page. Errors stay at the bottom centre of the window until you close them with **×** or press **Escape** while the chrome has focus. Escape closes an open surface or find bar first; a second press dismisses the error. **Ctrl+W** leaves errors visible. A newer error replaces the message; **· N more** opens the list. **Settings → Recent errors** keeps the last ten errors, newest first, with their age and a **Clear** button. A red dot on Settings means there are errors you have not seen in that list. Opening the list clears the dot. Error history lasts until you quit.

Form fields and editable areas are left out of saved text. Visible password forms block automatic capture.

Recovering the comment or permalink depends on the site's DOM. When that context is missing, the parent page is still kept. The imported Chrome corpus stays separate from live browsing.

For a post that appears later in a dynamic feed, select the passage and press **Ctrl+S**. Automatic capture does not keep saving everything that appears while you scroll.

Without `--data-dir`, Canopy keeps its database and WebKit profile in `$XDG_DATA_HOME/canopy`, or in `~/.local/share/canopy`. Canopy starts dark. The gear at the end of the tab row opens **Settings**, where **Theme** offers **Dark**, **Light** or **System**, and **Remember pages** turns automatic capture on or off. The chrome follows the desktop reduced-motion preference. Right-clicking its background shows no menu. Text fields keep their copy and paste menu.

The theme persists in the `theme_mode` session setting and applies immediately. With **System**, it follows the desktop's dark preference.

## Download files

A link to a file Canopy cannot show, such as an archive or a file the site marks as an attachment, is saved instead of opened. **Download Linked File** and **Save Image As** in the page's context menu save too. Canopy does not ask where to save. Files go to your download folder, the XDG download directory or `~/Downloads`, which Canopy creates when it is missing.

Canopy keeps only the last part of the name the site suggests, so a file always lands directly in that folder. It never replaces a file. A second `report.pdf` is saved as `report (1).pdf`. While a download runs, its data sits in a hidden `.canopy-download-…` folder there, and the file appears under its name once it is complete.

A message under the tab row says when a download starts, when it is saved, with its file name, and when it fails, with the reason. A failed download leaves no partial file behind. A download adds no page to your forest and keeps the page you are reading.

A saved file stays attached to the page it was downloaded from. In the tree strip it shows as a small green leaf on top of that page's node. A page with up to three files shows one leaf for each. A page with more shows one leaf and their count. The current page and its ancestors carry leaves. A folded capsule carries a leaf for files on any page inside it. Side-branch files are available beside their named rows in the attached list.

Click a leaf to list that page's files, with their size and when they were saved. Click a file to open it with your desktop's app for that kind of file. The folder button beside it opens the folder it was saved in.

Canopy records where the file was saved and leaves the file there. If you move or delete it, its leaf turns hollow and the list says **Moved or deleted**. Canopy notices this the next time it redraws the strip or when you try to open the file. A file whose address you type, or that another app hands to Canopy, attaches to the page on screen. A file saved with no page open is not attached to any page.

Canopy has no list of all downloads yet, and it cannot pause or resume one.

## Sign in with a saved login

When a page asks you to sign in and the vault holds a login for that site, a key button appears in the top row with the account name. Click it, or press **Ctrl+Shift+L**, and Canopy types the user name and password into the page. With several accounts for the site, the button opens a list to choose from. You then sign in yourself: Canopy does not submit the form.

Nothing is filled until you ask. A login is offered only on the exact site it was saved for, with the same scheme, host and port, and never to a frame embedded in the page. On a sign-in that asks for the user name first and the password on the next page, use the button on each page.

The keyring may ask you to unlock it the first time a login is filled. Sign-up and change-password forms are not filled. Fields that a site keeps in a closed component or in a frame are not found.

### Save a login

When you sign in or sign up with a login the vault does not hold, the key button reads **Save login?** once the site has let you in. For a saved account with a different password, it reads **Update password?**. Click it and choose **Save** or **Update** to keep the login, or **Not now** to let it go. Nothing is saved until you choose. An offer you do not answer goes away after five minutes.

Canopy waits for the page to stop asking for a password before it offers. If the site sends you back to its sign-in page, or nothing happens for a minute, there is no offer. A site that keeps its sign-in form on screen after letting you in gets no offer either.

A login you save stays in the encrypted vault on this computer, next to the ones brought in from Chrome, and replaces the imported copy of the same account. Importing from Chrome again does not bring the old password back. There is no list of saved logins to edit or delete yet, and no way to tell Canopy never to ask for a site.

### Keep logins in your password manager

Canopy can read and save logins in a password manager you already use, in place of its own vault. It reaches the manager through a helper, a small program called `canopy-login-NAME`. To choose one, run:

```sh
canopy --login-helper NAME
```

The choice takes effect the next time Canopy starts. `canopy --login-helper vault` goes back to the vault on this computer. Canopy uses one or the other, not both: with a helper chosen, the vault's logins are not offered.

When the manager is locked, the key button reads **Unlock** with the manager's name. Click it, or press **Ctrl+Shift+L**, and the manager asks for what it needs. The manager decides which of its logins match a page. Canopy asks it only about pages loaded over HTTPS, or from this computer.

#### Bitwarden

The Bitwarden helper comes with Canopy and uses `bw`, Bitwarden's own command-line tool. Set it up once:

```sh
sudo pacman -S bitwarden-cli   # or your system's package for bw
bw login
install -Dm755 helpers/canopy-login-bitwarden ~/.local/bin/canopy-login-bitwarden
canopy --login-helper bitwarden
```

Your master password is asked for in a small window once each time Canopy runs, the first time you unlock. The helper needs `zenity` or `kdialog` for that window, and Python 3. The session stays in Canopy's memory, so the vault locks again when Canopy quits. Logins you save in Canopy go to your Bitwarden vault and reach your other devices from there. Changes made on other devices arrive at the next unlock.

`bw` is slow to start. The key button appears about four seconds after a sign-in page has loaded, and unlocking takes about six seconds after you type your password. Logins that ask for the master password again are not offered, because `bw` cannot ask for it.

#### Write a helper

A helper is run once per question, with the question as its only argument. It reads a JSON request on its input and writes a JSON answer on its output.

| Question | Request | Answer |
| --- | --- | --- |
| `get` | `url`, `origin`, `state` | `{"logins": [{"username": "…", "password": "…"}]}` |
| `store` | `url`, `origin`, `username`, `password`, `state` | `{}` |
| `unlock` | `state` | `{"state": "…"}` or `{"cancelled": true}` |

Any answer can instead be `{"locked": true}` or `{"error": "why"}`. `state` is the text the last `unlock` answered, or null. Canopy keeps it in memory only and sends it back with every request, so a helper can hold an unlocked session without writing it to disk. `get` has 30 seconds to answer, `store` 60 seconds, and `unlock` five minutes.

## Bring your Chrome history

To bring your Chrome history, bookmarks, cookies, and saved passwords into Canopy, run:

```sh
canopy --import-chrome
```

It reads `~/.config/google-chrome/Default`. Pass another profile folder after the flag to read that one instead, and add `--data-dir PATH` to import into a separate data folder. Chrome can stay open.

`--import-chrome` replaces the history for that profile and adds missing bookmarks to Canopy. Existing bookmarks keep their title, folder, and order. The upgrade copies previously imported bookmarks once. A bookmark removed in Canopy can return on re-import.

**Ctrl+K** then shows **Bookmarks** and **From Chrome** results next to your forest. A bookmark always plants a new tree. Imported history opens like a typed address. Until Canopy has habits of its own, the new-tree page offers your busiest Chrome sites. Imported pages plant no trees and add no tabs.

Cookies reach Canopy while it runs, or at its next start before it reloads your page or opens the addresses it was given. This keeps sites signed in when their sessions still work. Chrome's cookies replace matching Canopy cookies. Session cookies last until Canopy quits. Partitioned and expired cookies are skipped.

Passwords go into an encrypted vault. Its key stays in your system keyring. The import may ask you to unlock the keyring. [Sign in with a saved login](#sign-in-with-a-saved-login) explains how Canopy fills them.

Running the command again refreshes imported pages and updates matching passwords without removing other saved accounts. Canopy loads none of the imported pages. If Chrome is writing a database, or the keyring is locked, resolve the reported error and run the import again. Cookies accepted just before a crash may need another import.

## Open links from other apps

Pass one or more addresses to open each one as a new tree. The last one is shown, and the others load when you open their tree:

```sh
canopy https://example.org https://www.rust-lang.org
```

When Canopy is already open on the same data folder, a second launch hands its addresses to that window, brings it forward and exits. Without addresses it only brings the window forward. A different `--data-dir` is a separate browser with its own window. Canopy opens only HTTP and HTTPS addresses and refuses anything else with an error.

The installed `canopy.desktop` declares Canopy as a handler for `http` and `https` links. To make Canopy your default browser, run this command after installing it:

```sh
xdg-settings set default-web-browser canopy.desktop
```

`xdg-settings get default-web-browser` shows the current choice. Links clicked in other apps then open in Canopy.

The running window holds a lock on `instance.lock` in its data folder and listens on `instance.sock` next to it. The system releases the lock when Canopy quits or crashes, so a crash never blocks the next start. If the window cannot open the addresses, the second launch prints the reason and exits with an error.

## Sandbox and video

Canopy runs page and chrome processes inside WebKit's Linux sandbox, which needs `bubblewrap` and `xdg-dbus-proxy`. If the sandbox cannot start, WebKit stops with a fatal error. Setting `WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1` starts Canopy without it, and Canopy then prints a warning.

NVIDIA's video decoders cannot start inside the sandbox, so on a machine with NVIDIA's driver pages decode video in software. Canopy sets `GST_PLUGIN_FEATURE_RANK` and `WEBKIT_GST_DISABLE_GL_SINK` at startup to keep that video from painting green. A value you set yourself for either variable is kept.
