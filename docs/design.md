# A forest of visits

Canopy keeps the context of a browsing session. A visit has a URL, a title and an optional parent. Each root starts a tree.

After A then B, going back to A and following C gives A two children. Visit B stays reachable. Following a URL again can create another visit, because the context has changed.

## SQLite owns the forest and the cursor

The database holds the visits and the foreground cursor. Browser stays the only writer of SQLite. A foreground commit writes the visit, the cursor, the tree's target and the arrival log in one transaction. A hidden commit writes the visit without moving the other three.

Two structures were compared before implementation. The first gave ownership of the state to SQLite. The second also kept a forest in memory. The first was chosen to avoid a second write path and keeping the cursor in sync.

The alternative brought two things to the chosen design. The navigation intent stays explicit until the load actually happens. The tests check the behaviour with WebKit and compare the forest before and after reopening the database.

Schema 12 adds nullable `dismissed_at` and `branch_settled_at` marks to visits. The migration runs in a transaction and preserves version-11 rows. Selection clears dismissal on the selected visit and settlement on every ancestor in the same transaction as the arrival log and cursor. The forest projection excludes dismissed links and settled subtrees from waiting counts. Strip branch counts omit settled subtrees, while the zoomed tree and attached lists retain a branch stub.

Moving a branch changes only its head's parent and the two tree targets, in one transaction. Moving clears the new root's settled mark and preserves marks below it. Its `tree_state` target is the most recently opened page inside the subtree, ordered by arrival log. The old target moves to the most recently opened page left in the original tree. Native history bindings keep their visit IDs. The existing history guard resolves tree membership against SQLite, so Back cannot cross the detached root.

## An intent comes before a visit

Each view owns its Navigation, its pending load and its current Document. Browser owns no global load. A LoadTicket ties one incarnation of a view to an attempt number. Title, capture, context and scroll callbacks carry the Document they came from and check its epoch before writing. A failed transaction keeps the load and the Document until Finished. Finished abandons an attempt whose commit did not succeed. A same-document completion abandons a rejected commit immediately, because WebKit sends no Finished for it. The live page does not retry commits.

A new navigation records its parent before loading. A restore records the id of the visit to reuse. The live page accepts Started for an issued attempt only after WebKit accepts policy for its requested URL or history item. Earlier Started signals cannot adopt a replacement request. The live page then confirms the URL when the main document loads.
An initial Home Started notification with no document or navigation ticket does
not cancel an address candidate; a queued bootstrap notification may arrive after
Home was stopped. Started notifications for real page navigation retain ownership.

The live page distinguishes an issued request from a started load. Only a started load is stopped and drained before its replacement is issued. The queued replacement retains its own ticket. A same-document request needs no Finished signal. A page-started load cannot adopt a request the live page has not issued.

A return to a live form result is held at navigation policy, before normal history
handling accepts an issue or records a restore. A main-document BackForward POST
or FormResubmitted decision asks “Return to a form result?” in the HTML chrome.
This includes page-cache returns: WebKit exposes the same policy action whether
it will reuse a document or send its form again. First submissions, GET history,
and Canopy's GET-based Reload do not ask. Checkpoint entries with POST provenance
still take the existing GET revival path before this guard.

Frame form-result returns remain an unsupported case. A local no-store fixture
with the WebKit page cache disabled measured BackForward POST policy actions for
iframe-only Back and Forward, main-document Back restoring an iframe POST, and
`history.back()` in either the frame or its parent. NavigationAction supplied no
frame name for these unnamed frames; the fixture identifies the target frame.
An iframe `location.reload()` instead reported Reload GET and sent no POST.

Extending the hold to these actions prevents the POST, but does not provide the
same Cancel contract. Measured iframe-only reproduction:

1. Load a parent with an iframe containing a POST form.
2. Submit it to a no-store result, then navigate that iframe to a GET page with
   a text input. Type a value into the input; save the native current history item.
3. Go Back in the WebView and hold the iframe's BackForward POST policy decision.
4. Click Cancel and ignore the decision. The iframe still displays the GET page,
   but WebKit's back-list position has moved from index 2 to 1, and Forward is
   now available. The cursor remains there after three seconds.
5. Traverse to the saved history item to restore the cursor. The iframe is
   recreated and the typed value is lost, even with no additional network GET.

Main-document Back is worse: the parent has already committed by the time its
iframe's POST policy arrives. Ignoring it leaves the parent selected with an
empty iframe instead of preserving the outgoing page. These are measurements of
an experimental guard, which was removed. Frame support must preserve both the
native cursor and the existing documents before that guard can be enabled;
merely grouping policy decisions cannot undo an already committed parent.

The live view retains one policy decision with a window-unique question number.
Continue takes it out of the slot, runs the ordinary policy handler once, and
uses the decision if that handler leaves it to WebKit. Cancel takes it out and
ignores it, discarding the owning ticket, issue, traversal, response bookkeeping
and continuation. It creates no visit or event and never selects the target.
The chrome defaults focus to Cancel; Escape and the scrim cancel too. Replies
carry the question number, so a delayed reply cannot permit another decision.

A replacement page command cancels the question before registering its own
attempt. It resumes after WebKit restores the native cursor
to the document on screen: Back, Forward and visit picks proceed as Cancel followed
by that command. A main-frame navigation also cancels an older question; iframe
navigation and new-window requests leave it open. Selection, settling, release,
window close and web-process death close the question.

When an owner-requested switch reaches a form result in an older entry of a hidden
live view, Pages selects and shows that view's current document before asking.
This includes Back from a branch to its parent's older entry. Cancel leaves that
document selected and usable; Continue traverses to the form result. The switch
candidate is removed before asking, so neither answer strands a pending candidate.
Other traversals keep loading out of sight until commit. Hidden script returns
and never-seen views still refuse form-result decisions silently.

Tab and Shift+Tab stay inside the question, with a focus guard against controls
behind it. Dialog buttons, Escape and the scrim return focus to the page. A chrome
command that cancels keeps its own focus destination, including the search pill
and palette shortcuts. While held, the forest's selected Document continues to
supply the address, title, strip selection and window title. Chrome has no native
loading indicator; WebKit's provisional URI and loading flag do not supply these
visible fields. Only Continue enters the existing recording path.

Server redirects belong to a single navigation. A reload or a restore does not create a visit. A failure keeps the last recorded position and shows an error.

Persistent ids are ULIDs. URLs are not visit ids. Subresources and embedded frames do not become children. A native Reload is a candidate only when its request URL equals the current document URL. It becomes a restore only at main-frame Started, so an iframe reload cannot suspend document writes. A title updates the visit only while the document URL equals its stored URL.

## GTK and WebKit stay on their thread

WebKitGTK builds the page view inside a GTK container. wry still builds the chrome view. The native signals tell apart the start, the commit and the failure of a load.

Pages in src/pages/mod.rs owns the window's Browser, PageContext and the branch registry. Ui owns the window, HTML chrome and cookie inbox. It translates navigation intents into Go commands and reads the forest through Pages::read, which exposes only a shared Browser reference. Pages owns settling and startup selection, including blanking the live page when the last tree is settled.

PageContext in src/pages/native.rs owns the content WebContext, its persistent data and cookie manager, favicon database and download handler. LivePage in src/pages/live.rs builds each page in that context and keeps navigation and native history. Its callbacks enter Pages through a weak state reference. A callback guard retains the owner for the entire callback, including reentrant observer calls, and reaches the sole Browser writer. Ui applies cookie batches through Pages with its existing startup gate and serialized drain. The chrome keeps its separate wry context.

Pages keeps branch-palette and path continuations by LoadTicket. A continuation waits for its matching commit, then for that attempt to finish loading. A same-document commit is ready immediately. Once ready, it runs exactly once on a GTK idle after the native document has been bound, even if the page starts another navigation first. The path's second address attaches to the visit committed by its first attempt, including when that page refreshes itself. A native replacement before readiness, a failure of the waiting attempt, an explicit cancellation, blanking, or settling discards it. A Pages command cancels when it starts a new attempt; Back and reload also clear a path continuation when they do nothing. A no-op Back or reload preserves a pending branch palette. Pages issues the path's second address itself, or reports OpenBranchPalette to Ui through its one PageEvent observer.

Downloads find their source with WebKit's download webview lookup. PageContext asks that view's weakly registered state for the visit at transfer start. A transfer retains the context, which reports through a weak Pages reference to record the saved file. Saving and recording do not depend on the source view surviving while the window owner remains alive. Its event sink retains no page. Favicons retains the database and observes database changes and page icon notifications through weak cache references.

Pages clones a live-page handle or observer before calling it. Browser, navigation and continuation borrows end before WebKit or GTK operations or observer callbacks that can reenter. The native adapter clones a document before binding its JavaScript epoch. Read closures contain database queries only. Each live branch has one view. Dropped branches retain in-memory native history checkpoints; visits outside those histories stay in SQLite.

Back uses an entry still present in WebKit's history only when its visit belongs to the current open tree, otherwise the parent in the forest. Page-script and mouse Back cannot reopen a settled tree or cross into another open tree. Back from the blank start page does nothing. The list's changed signals drop the bindings of removed entries and bind same-document entries to their visit. One exception: when a page script adds an entry for another resource (not only another `#part`) within 4 s of a left click or Enter in the page, as YouTube does when a video is opened, that entry becomes a child visit with the route's title. Each press counts for the first added entry only, so routes a page makes on its own, replaceState, and `#part` links stay on the visit. Each entry also keeps its native document group, because a return can move the list before Started. This group tells a return within the same document from a full load, even at an identical URL. A full commit gets a fresh group unless it is a back or forward traversal. A traversal onto an unbound entry falls back to the forest parent. The native cursor is read at changed or Committed, never at notify::uri. A restore only traverses an entry from another native document. On a pushState route, F5 and a visit-node click load the stored visit URL, so neither leaves an attempt waiting for load signals that a same-document traversal never sends. A cold resume reloads the URL and restores the recorded scroll. WebKit's page cache can keep the DOM and the forms of an entry still present.

## The chrome is an HTML view above the page

The tabs, the tree strip, the palette, the attached lists, the menus, the "New tree" page, the forest view and the reader are drawn in HTML. A second, transparent webview, served by the `canopy://` scheme, sits above the page. Rust keeps the state and every decision: the layouts, the palette results and the intents. The view receives JSON and sends clicks and keys back over IPC.

Chrome frames carry the current tree, open-tab summaries, waiting pages and the
open palette's scope page. The start page requests its saved and suggested miniature data when shown. The
forest requests its independent value after allocation. `ForestPane` owns Closed or Open and either Canopy or History. History owns Find, its selection anchor and loaded reach. A fresh `Index` derives sites and dated history from the in-memory projection, without a cache. Closing discards pane state. Unchanged strip items
and list rows keep their DOM nodes. After a refresh, the strip keeps WebKit's
animation clock active for up to 300 ms, as it does during movement, but skips
all geometry writes when nothing moves. This prevents idle clock startup from
delaying the next visible update. The clock then stops. Strip motion uses transforms,
and favicon revisions apply to each page address, including images in the zoomed tree.

At rest, the view covers only the top 94 px. The tree strip stays 52 px high. `graph.rs` builds `StripItem` values for the root, path pages, named parent, same-host runs, Earlier pages, current page, and read or waiting children. Runs exclude the root and parent. At most three middle items nearest the parent stay visible. Older items merge into Earlier, then more fold only as required by the room reserved by `pins.rs`. Rust supplies the padding, borders, favicon size, and gaps used by both the width estimate and CSS. Parent names can use up to 240 px and shrink before another item folds. Rust sends positions, branch subtree counts, list headings, and at most six named rows per item. Parent names reuse `naming.rs` to remove the site and omit title segments shared with their parent, then remove the prefix shared with the current page when a distinct suffix remains. The current pill keeps its full title. Back and Forward buttons sit before the tree at the strip's left end. `pins.rs` reserves their 66 px only while the tree keeps at least 280 px; narrower, they give way to the tree. The frame names their targets. Back's comes from the selected live page, which makes the same choice `Go::Parent` makes: the visit behind `native_back`'s entry, else the forest parent. Forward's is the child `Browser::forward` returns, the one shown most recently. A click sends the same `Go::Parent` and `Go::Forward` as Alt+Left and Alt+Right.

A 250 ms rest or keyboard focus opens one list below the item, joined by a continuous hover area. Entering the list cancels dismissal. Moving focus outside the item and list or back to the page dismisses it. Its keyed rows and action buttons survive frame updates and refresh deferral between a press and release. Preview requests populate the list head with saved text. Run rows follow path order, Earlier rows reverse it, branch heads are newest first, and waiting children keep queue order. Rows use short names and retain full titles in tooltips and accessible names. The current page lists read children, leaving waiting children to After. Zero counts are omitted. More opens the existing zoomed tree, revealing the earliest hidden page when opened from Earlier. Download leaves and live marks stay on page tiles, and folded items aggregate files. Branch rows retain access to their files.

The view grows to the bottom of the attached list or menu and covers the whole window for the palette, the reader, full-page surfaces, and bottom settle feedback. Below that region, the page gets clicks.

Node menus name their target with its favicon and short name. Settled pages and their descendants omit Settle branch. Repeating the settle command preserves the existing Undo. Settle feedback counts undismissed waiting links kept anywhere inside the branch, including branches put away earlier. The bottom toast extends the chrome view to the window height; its input region includes only Undo alongside the usual chrome area, so page clicks still pass through.

Alt+Right chooses the direct child with the latest arrival in the visit log, regardless of where that arrival came from. Sibling keys exclude dismissed pages and settled branches. Moving a branch clears the settled mark on its new root and preserves marks below it.

The current-tree workbench uses one Rust preorder outline for both Tree tiles and List rows. Older children appear immediately below their parent, oldest first with each subtree indented one level. The newest child follows those side branches at its parent's indent. Tree chains run right in the original 124 by 40 px tiles, and older branches drop below. The same visible outline supplies the List indent and Tree coordinates. Current-path highlighting never changes positions. `Workbench` in `tree_view.rs` holds per-visit fold overrides, a short-name or site query, and the waiting-only flag. `Inner` owns it for the window session, keeping decisions beside the forest projection without persisting browsing presentation in SQLite. A settled branch defaults to folded. Filtering reveals matching pages with their ancestors, temporarily ignoring folds. Rust sends overflow against the window dimensions, short tile titles, fold summaries, and visible rows.

The sheet takes keyboard focus after its final allocation. On close, a visible, focusable keyboard trigger gets focus back; otherwise the page gets focus. A pointer opening returns to the page. Escape clears both filters before closing. Every close clears the filters, while fold overrides last for the window session. The side chosen by the user stays separate from a List forced by overflow. Arrow keys traverse or fold the outline, Enter opens, Delete reuses Dismiss, and Menu or Shift+F10 reuses the node menu. A non-navigation action keeps the sheet open, including settling a branch that moves the current page. Keyed row and tile buttons survive frame refreshes. Filter and toolbar controls also survive refreshes. Node menus focus their first action and share Up, Down, Home, End, Enter, and Escape handling wherever they open.

Tree menus come from `tree_card::acts` for the tree’s stored place. Activation is independent of menu membership. `Place` separates Tabs, Saved for later and History. Schema 16 stores position and tab pin in `tree_state`, with checks rejecting invalid combinations. The schema 15 upgrade stays intact. The schema 16 rebuild preserves `tab_order` within each pin group and removes that column. Tab drags send `place-tree` with a before-tree anchor. Keyboard moves in the bar use `Browser::tree_step` and apply `Move::Tabs` within the pin group. `Browser::place_tree` is the only place writer. Its transaction preserves the first settlement time, appends new or reopened tabs, and computes a landing when a tree leaves Tabs. The tab bar draws pinned tabs as icons first and retains the current tab and overflow access when pins exceed available room.

`Browser::delete_trees` hard-deletes History trees and returns a memory record with a `DeleteId`. `Pages` owns the last record. Undo restores old integer keys when all are free; otherwise that table’s restored rows receive fresh keys in old order. Capture triggers maintain FTS. Foreign-key coverage tests audit the record against the schema. Runtime deletion drops affected queues, native histories, checkpoints and continuations. Download completion skips records for absent visits. Both deletion and Undo rerank frozen suggestions. The pane’s feedback follows successful replies, with no timer.

[Canopy](canopy.md) and [History](forest-history.md) own their independent values. The map folds only drawn pages of open trees, with current and previous roles independent of memory state. One batch query in `Browser::showings` counts new and return events and uses the latest event id, including reloads, for recency. The previous page follows the post-pending frame's current tree, matching the Ctrl+Tab rule.

`forest_pane/canopy` owns the three zoom levels, folds, Flow and Islands geometry, item bounds, labels, menus, targets and directional neighbours. `Layout` selects left-to-right Flow or radial Islands. Islands gives each branch a nested angular share weighted by subtree size and item width. Short chains keep their direction. Asymmetric item rectangles, labels and tags set clearance at each band minimum. Ordinary branches choose a compact ellipse and rotation with clear straight edges. After six items, long chains coil within their share; broad leaf fans use arc rows. Islands centres a fitting current tree in its opening room. Both layouts wrap tree rectangles at the resting zoom; only Flow stacks at Near. `Ui` alone owns the layout preference, reads `canopy-layout` beside the database and atomically replaces that file on a choice. Missing, unreadable or invalid files select Flow. The schema stays unchanged. A changed layout brings the current page into view once. The map path does not build the whole-forest History index. The HTML shell is laid out invisibly before its first request, which carries its room. The map painter retains keyed controls and edges, and owns continuous zoom, translation, drag suppression and in-map hover text. It rejects obsolete numbered threshold replies and preserves a page's fractional box point through refolding, refresh and resize. The existing press guard holds incoming forest paints until release. A capture-phase GTK zoom gesture is retained by the chrome owner and forwards incremental scale and its centre to the same pointer-anchor path as wheel input. Physical touchpad delivery still requires a desktop trial. The strip and zoomed tree sheet retain their own layouts.

Transient views retain their trigger in a focus-return record. Menus, search, saved text and the forest take focus and contain Tab. Closing returns focus to the trigger, with a stable DOM locator when a refresh replaced it. Tab buttons retain identity across frames. Attached strip lists take focus on Down or Enter and return to the item on Escape. Menu and Shift+F10 use the same context-menu handlers as pointer input.

Rust builds accessible names, branch-count descriptions and each visit's waiting, loaded and live state suffix. Chrome exposes tab selection, current pages, branch counts, switch state, theme state and palette selection through ARIA. CSS reads the state attributes for selection and toggles. The palette keeps DOM focus on its combobox and exposes the selected listbox option through aria-activedescendant. Separate visually hidden live regions announce status and errors. The visible toast carries no live role. Keyboard focus uses the text colour to distinguish it from green selection. Tabs and list headers use inset outlines to keep the full ring visible. The tab row uses roving focus with Left, Right, Home and End. The Saved text button follows the current-page item in DOM order. Strip lists retain focus on pointer leave only after keyboard input.

With the desktop reduced-motion preference, strip moves finish immediately, and CSS transitions are disabled.

The zoomed tree sizes its body from the window height sent by Rust, before measuring and centring the current tile. It never uses the temporary native view height. The chrome suppresses WebKit navigation context menus outside editable fields and selected text. GTK tooltip queries are refused while a chrome layer is open, so cached tooltips cannot cover menus even with a stationary pointer. Tooltips work again after the layer closes.

Rust sizes the active tab first from its title width, icon, padding and settle cross, between 110 and 200 px, capped at the room left after the overflow arrow. Other tabs share the remaining room as titles, down to about six characters with their cross overlaid on hover. Tabs become icons one at a time, farthest from the active tree first, then hide behind the arrow. Icon tabs keep a small green waiting count. The active waiting count sits at the icon corner too, preserving title room. Pinned tabs are 34 px icons first in stored order, with titles in their tooltips. A settle click requires the cross to have been visible under the pointer at press time and a click count of one. The cross draws a 16 px hover shape inside its 24 px hit area. The overflow list includes waiting counts, pin marks and a settle button. Rust remembers the room measured by the view and sends a new tab fit in every frame. The view applies that fit in the same presentation call as the active aria-selected state, without another round trip. Ctrl+Shift+Tab uses the visit log in descending event order to find the most recently shown other open tree.


This choice replaces the GTK and cairo chrome. It costs a second WebKit process, measured at about 124 MB of PSS. The HTML chrome can be reused for a future macOS or Windows port. On Linux, GTK remains the invisible host of both webviews, as in Tauri. The page engine and the host window stay tied to WebKitGTK until the port to macOS and Windows.

## Saved logins are filled on request

A LoginProvider answers which accounts are saved for a page, what an account's password is,
and keeps a login. The encrypted vault in src/logins.rs is the first provider. Ui owns the
provider, so a password manager the user already has can stand in for the vault without the
page or the chrome changing.

A helper in the page's isolated world, src/login_fields.js, finds the sign-in fields of the
top frame, including fields inside open shadow roots. It posts an empty message when they
appear or go away. The live page then asks the helper for its state against the document it
knows, and reports the page address only while that document is still on screen and of the
page's own origin. It asks again when a load finishes, and each new document starts with no
fields. Page script cannot post that message or read the helper.

Ui looks the accounts up and numbers each offer it sends to the chrome. The chrome sends
back the offer number with the chosen row, so a choice made on a replaced offer fills
nothing. The password leaves the provider only then. The live page refuses to fill a
document of another origin, and the helper checks the origin again in the same turn as it
sets the fields. Nothing is filled without that choice, which keeps a hidden form from
collecting a password on load.

The same helper hears the user send a login: a form submission, a press on a button or
Enter in a field, while a password field holds a value. It posts the user name and password
at once, because the document may be gone a moment later. The live page accepts them only
for the origin of its committed document. SaveWatch in src/ui/save_watch.rs holds that
login as a candidate. It offers it when the page reports that it shows no password field
any more, drops it when a new document shows one, and drops it after a minute. Ui then
compares it with the provider: an identical saved login brings no offer, a different
password brings an update. The password is written only when the user accepts, and an
unanswered offer is dropped after five minutes. In the vault, a login saved in Canopy has
its own source and stands in for imported copies of the same account.

A provider answers through a callback, at once or later on the GTK thread, and can answer
that it is locked. Ui numbers each lookup and drops an answer that arrives for an earlier
page. LoginHelper in src/login_helper.rs is the provider for a password manager the user
already has. It runs a program called canopy-login-NAME once per question, with JSON on its
input and output, through GIO's asynchronous subprocess calls, so a slow manager never
blocks the window. The helper decides which accounts match a page, and Canopy asks only
about HTTPS and loopback pages. The text a helper answers at unlock stays in memory and
goes back with each request, which lets a helper keep a session without storing it. The
passwords of the last four lookups stay at hand for the fill that follows. One provider is
active at a time: the helper named in the data folder's login-helper file, or the vault.
Canopy links no password manager's code. helpers/canopy-login-bitwarden drives Bitwarden's
own command-line tool.

## The MVP1 scope

The first milestone checks navigation, forks and resuming after a restart.

The forest and current-tree layouts and saved-text extraction are built. Summaries and the agent wait for a later milestone. The live page has no generic trait. Moving to Servo or CEF will mean revisiting the rendering and profile integration.

In-app changes inside web applications remain a separate case. The MVP does not promise to turn every JavaScript state change into a visit.

## Branch registry and switching

Pages owns one selected live branch, hidden live branches ordered by their last
shown stamp, dormant checkpoints, and at most one candidate. The window increments
its own counter at selection. There is no visit-to-branch index and routing never
compares URLs. Routing checks a candidate's pending Restore, the selected document
and history, then hidden live and dormant histories in MRU order. A tree-tab return
uses the most recently shown branch's current document, including hidden redirects;
a specific visit-node selection still requests that historical visit. A view follows
links and forks in place. Queued visits can load in the background under the memory
budget, but become read only on first selection.

Returning to the current document in a hidden live branch selects its existing
Document in Browser's selection transaction, with no load or request. Forms,
JavaScript, scroll and audio survive. A different history entry traverses that
branch's existing native history. Dormant and cold targets use hidden candidates;
only their commit changes the cursor. Cancelled checkpoint revivals keep their
checkpoint. Selected events refresh address, title, loading, Back and login offers;
focus follows the page only if the outgoing page owned it. Selected-document
notifications publish the chrome frame immediately; background changes remain
coalesced. A deferred old cursor frame cannot hide a newly opened chrome editor.

Candidate switches keep the outgoing real page above the incoming page until the
existing snapshot/frame handshake completes, with its one second cap. Returning
to a retained document shows it directly and hides the outgoing view; the live
view already owns rendered content. An initial Xvfb comparison found medians of 17.4 ms for direct live returns and
34.7 ms with the hold, without observed blank pixels in 20 direct returns. After
synchronous chrome bookkeeping, final medians were 17.7 ms live and 239.0 ms dormant. Only real
page views cover pages. After the hold the outgoing page is hidden and retained;
the blank startup view is discarded. Pages::place_view remains the sole owner of
all page allocations, including hidden views, at creation, resize, show and map.

Hidden pages keep running. Their native Navigation records redirects, scripts,
titles, icons and scroll against their own visits. Hidden commits never select a
window visit, and hidden loading, error and login events never reach the chrome.
Tree changes still refresh the forest. Native history is restricted to the page's
own open tree. Hidden script dialogs are dismissed (confirm and beforeunload answer
false; prompt cancels). Permissions and fullscreen are denied; file choosers cancel;
printing is handled without presenting a dialog. New-window links retain the existing
queue-without-selection policy; unhandled WebKit creates have no window factory.
Downloads keep the source view's visit and proceed through PageContext. A dead hidden
web process leaves a checkpoint if possible, otherwise its visits are cold.

The memory budget is one eighth of MemTotal in /proc/meminfo. Set
CANOPY_MEMORY_BUDGET_MB to an integer MiB value before launch to override it (1
forces pressure for desktop checks). Configuration is read once at desktop startup.
Malformed or overflowing CANOPY_MEMORY_BUDGET_MB values print one stderr line naming
the variable and value, then use the default budget. There is no page-count limit. A worker samples PSS plus SwapPss of Canopy and its
own descendants only, following each known process's /proc/PID/task/TID/children
files rather than scanning other processes. Exited processes are skipped; any other
read failure makes the sample incomplete, and no drop follows it.

Sampling starts after a switch or committed navigation settles and every five seconds while hidden pages
exist, and on queued-page admission even with only one live page. Sample starts are at least two seconds apart;
requests during that interval collapse into one trailing sample. GTK uses the fresh complete result to drop the
least recently shown hidden page. It reads is_playing_audio at that decision. The
selected page, candidate, held outgoing page, every audible seen page and any seen page
waiting for WebKit to hand over a download are excluded. A load awaiting its main
response is also protected until policy can distinguish a document from a download. After handover the transfer
survives destruction of its source view.
One budget drop is checkpointed and destroyed at a time across all windows; a
shared permit remains held through the teardown grace. Samples started before a drop are
invalidated. A one second teardown grace follows destruction before sampling again;
protected-only registries wait for the ordinary slow period. Checkpointing uses the
existing scroll capture (500 ms cap), native session bytes and history bindings.
Commands queue during holds or checkpoint capture and drain afterward.

Settling immediately disables the selected page's input and native recording,
mutes its audio, and clears its login offer, including when a landing is still loading. The real page
may remain as the visual cover. All the tree's hidden pages are checkpointed and
destroyed, even if audible. The selected page follows after its landing is shown;
a live landing selects immediately. A retired cover never receives commands.
Addresses, citations, saved pages and Back/Parent wait in the FIFO until the landing
shows; deferred addresses become children of that landing. Reload during landing is
ignored. Visits, trees, new trees and another settle replace the pending landing.
An empty, failed, downloaded or cancelled landing with no replacement falls back to
the start page, clears the cursor and releases the retired cover before draining
commands. Download attribution uses the initiating view's own current visit even
when hidden; a source view never falls back to the window cursor. Settled checkpoints remain available for an
explicit later reopening. A reopening requested during the settled branch's scroll
capture waits for that checkpoint; live landings still select immediately. No
checkpoint is persisted to SQLite.

A checkpoint owns session bytes, native-order entry bindings and current position,
without native objects. Revival validates every position, URL and stored visit URL
before rebinding the newly created item objects. Invalid bindings, absent targets,
fall back to cold GET. Restored POST entries require GET before
native traversal. No checkpoint is written to SQLite. Restart creates only the
cursor's cold page and restores stored scroll.

Revived hidden pages defer scroll restoration until visible. The helper waits for
two equal frame sizes before restoring scroll. WebKit retains scroll for other
history entries; SQLite supplies the checkpoint's current entry. Forms and scripts
are lost when a branch is released. Shared PageContext services survive source-view
destruction. Continuation cancellation checks the owning view's ticket.

Hidden seen pages expose their current Document visit as a live set in the existing
chrome frame. Building it scans hidden handles once (O(hidden pages), no database
queries). Only that current visit gets a filled grey bottom-left dot (green is kept
for pages not read yet, whose top-right dot is larger);
selected pages, native back entries and dormant checkpoints do not. The strip,
tree sheet tree/list and tabs share the mark; a tab uses its tree's last
visit. Preview metadata, node tooltips and tree list details say “Still open”.
Never-seen pages keep their separate top-right waiting dot. Absolute positioning
preserves node geometry. Hold completion, pressure removal and hidden process death
notify Changed; candidate history routing and cancellation also notify when a page
leaves or rejoins the hidden registry. Selection, hidden navigation and settling use
existing refreshes.

### Switch cancellation and page Back

Tree and branch settling share one scoped command. Its membership predicate selects queued work, never-seen views, hidden live views, candidates and the selected cover. Branch settling routes the selected page to its nearest unsettled ancestor and uses the existing checkpoint, retirement and landing path. If startup finds the cursor inside a settled branch whose landing did not commit, it resumes at the nearest unsettled ancestor. The latest settle record keeps the head and the page to reopen until another settle replaces it or Undo consumes it. It reopens the previous cursor when it was inside the target, otherwise the settled tree's last page or the branch head. Ctrl+Shift+T reopens that tree or branch after the six-second toast disappears. Explicitly opening New tree dismisses existing visual feedback so its Undo button cannot intercept start-page controls. Undo clears that head's settle mark and returns through normal visit routing. A return to the retired current page waits for its checkpoint even after Undo clears the mark. It does not recreate released live documents.

Starting a candidate fails all pending foreground work in the old view and invalidates
its asynchronous link decisions. The old committed document remains usable. At
navigation policy, WebKit's user-gesture flag cancels the candidate for a foreground
navigation, including forms and full loads to the current address. Fragment steps
and history state changes keep it. NavigationAction has no main-frame boolean;
Canopy uses its absent target-frame name (or `_top`) as the closest available
signal. Unnamed subframes cannot be distinguished reliably at this point. URI
equality is not a frame signal: the native URI still names the outgoing page.
A page timer, automatic refresh or redirect without a gesture keeps the switch.
Started alone does not establish user intent. Background link commands keep it too.
The new-tree editor stays open across page-driven cursor updates in the outgoing
tree; owner Back, Reload and visit commands uncover the page explicitly. Settling cancels
only a candidate targeting that tree. Settling the last tree creates a fresh home view, so the next branch cannot inherit settled history. Release
keeps valid checkpoints, including those of settled trees.

A restored POST that is current at initial revival is fetched by GET. Its duplicate
native entry is skipped by page Back; the GET drops forward entries, retaining the
back list. A noncurrent restored POST opens by GET in a fresh branch instead of
appending into the revived list. Its Back routes to its forest parent, reviving the
previous branch at that entry. The checkpoint and its forward entries remain dormant;
no form is resubmitted. This preserves the existing current-POST revival behavior.
Back selects the nearest previous page visit, skipping same-page frame and alias
entries. If none exists, it routes to the forest parent. Only when neither exists
may it traverse an earlier same-page state. Mouse Back runs this same command;
mouse Forward runs the tree Forward that Alt+Right uses. Saved captures route by visit identity.
A citation uses its parent's current view only when that parent is the current document;
otherwise it opens a fresh branch with that parent. Failed candidates emit the error
and Loading(false), allowing chrome to refresh.

When a saved capture identifies a more precise citation URL, its visit is routed
first. The existing ticket continuation then opens that citation as a child in the
routed branch, preserving both the capture branch and the previously selected branch.
The URL comparison chooses capture content only; branch ownership still uses visits.

Foreground cancellation returns a request that has not passed policy immediately
to Idle: WebKit sends no Finished when that policy-phase request is stopped.
A policy-accepted or already started load is drained through Finished instead.
Its delayed Started cannot become a new native request or cancel the candidate.
A subsequent user load queues behind that drain with its own ticket. This includes
a policy-accepted load waiting for beforeunload to return.

Pages::place_view owns page allocation and stacking. Every page view, including
hidden candidates and releasing views, uses the content rectangle (window width,
window height minus the fixed 94 px header). Pages stores that rectangle and applies
it at creation, show, map and every resize, without waiting for a container signal.
The widget's size request and Fixed position also equal this rectangle. Thus the
container's own allocation pass agrees with Pages; cover changes and view churn
cannot briefly allocate a different viewport before a corrective handler runs.
The external-policy ScrolledWindow takes the window's allocation independently
of these requests, including when shrinking; page positions do not introduce a
scroll offset.
GTK child visibility unmaps dormant candidates while retaining their visible flag,
so GTK accepts allocations for them; ordinary hide() rejects those allocations.
Expanded chrome covers overlay content rather than shrinking the page viewport.
Chrome stays above every page view, including immediately when a native view maps.
An unchanged forest frame preserves its interactive DOM nodes, so a press and release
can span a refresh without losing the click. renderForest owns that identity rule.
Adding, removing, showing or hiding page views never changes chrome allocation;
its persistent size request still matches the window width and cover height. These
independent allocations let the same GtkFixed preserve both rules.

The Fixed is a CanopyPageStack subclass. GtkFixed draws its private insertion
list even when GDK child windows have been raised into a different order. A
view realized after chrome therefore used to cover an opened forest or palette
while clicks still reached chrome. PageStack uses GtkContainer’s standard draw
propagation and orders its traversal: ordinary pages, the held outgoing page,
then chrome. Chrome explicitly allocates its bounds because wry’s exact-type
GtkFixed check skips subclasses. Pages::place_view supplies those roles and applies the same order
to GDK windows. No widget is reparented or unrealized, and no page image is painted.
The composed root-window pixel case in branch_widget_smoke checks both the opaque
pop-ins and page colour transmitted through their scrims; widget snapshots do
not test this composition.

Nothing but a real page view may cover a page. Canopy never paints an image
over a page or draws into its window. Pages::select_candidate switches bookkeeping,
events, focus and sign-in ownership at commit. Pages::hold_page maps the incoming
view in the same rectangle beneath the outgoing view and below chrome. The
outgoing window remains non-native; PageStack’s traversal draws the real outgoing
widget after the incoming widget and before chrome. The outgoing
window passes pointer input through; the event filter rejects nonselected input.
The commit transfers logical GTK page focus even when the host window is inactive,
without activating that window; global input focus alone cannot identify its focus owner.

The hold ends after an isolated-world script sees a body and two animation frames,
a visible-region snapshot completes (its image is discarded), and one further
animation frame completes. It also ends at 1000 ms, another commit or cancellation,
a queued switch or settle, or unmapping of the incoming view. Pages::end_hold
cancels readiness and removes the cap timer before hiding and retaining the
outgoing page. At most one outgoing view is held. The FIFO drains after the hold;
in-place navigation proceeds when that queue is empty. A short blank after the
cap when a page is busy is accepted. No stale image is retained. Smoke
instrumentation reports Holding, Hidden, Checkpointing and Released independently.
The hold remains bounded by 1000 ms and checkpoint scroll capture by 500 ms;
a memory-policy decision, rather than a switch, now separates those phases.
Pages derives the content rectangle from the host window even when a late Fixed
allocation still reflects the old child requests, preventing a shrink from being
undone by the Fixed's own size-allocate signal.

Response state belongs to a LoadTicket. Only the main response policy decision
that calls download marks that ticket for handover. NativeState::end_response
clears response bookkeeping on replacement or cancellation. No-content responses
end normally and discard empty candidates immediately. Download-started consumes
the matching response metadata and emits completion without a view-wide timer.

Forward cannot leave a POST result opened by GET in a fresh revived branch, because
that fresh native view has no forward history.

### Queued pages loading before first showing

A middle-click queues the visit as before, then Pages admits that session's queued
visit to `Branches::background`, a registry separate from hidden seen pages and
from the switch candidate. `Branches::queued` is a newest-first pending stack;
three loads at most run across windows, from start until first commit. Adopted
pages and committed documents still fetching resources hold no slot. A load with
no document after 20 seconds is silently destroyed without retry.
A weak window registry dispatches the newest eligible pending visit globally. Admission needs
a first complete memory sample. Admission requires that sample plus 256 MiB
for each background start since sampling began, including the proposed start,
fit the budget. Queuing and background commits request a sample, limited to once
per two seconds. Waiting links remain unloaded and are reconsidered newest first
on each sample; admission never drops another page to make room.
The latest over-budget sample blocks admission. Pending work survives switches;
startup does not enroll waiting visits from earlier sessions. Failed and dropped
views are removed without retry.

A never-seen view is muted before loading and uses the same hidden-page refusals
for dialogs, permission, fullscreen, print, file pickers, focus and login offers.
Downloads are refused both at response policy and the context download callback;
204/205 and failures without a document destroy the view silently. Basic-auth
challenges are immediately cancelled. A self-replacing navigation or window.stop()
cancellation preserves an already committed document. Its queued visit is
pinned through script navigation and redirects. Hidden commits update its URL,
title and favicon while preserving NULL opened_at, selection, last visit and the
arrival log. The existing coalesced chrome refresh reads committed background
visits into ChromeFrame::loaded; destruction clears the mark without losing metadata.

Opening a committed background page selects and shows the same view immediately,
records its first opening and arrival, unmutes it and stamps last shown. Opening
before commit transfers the same view to the candidate slot and uses the ordinary
commit hold. Adoption immediately ends background-only refusals: downloads are
saved and recorded, 204/205 follow the cold-open path, and load and certificate
errors are reported. It also unmutes the view; opened_at, current visit, tree last
visit and arrival remain tied to first showing. Cancelling that switch preserves
the request in the background registry. A cold
queued visit retains the existing loading path.

Pressure destroys never-seen views without checkpoints before dropping any seen
page, oldest queued first, ignoring WebKit's audio flag for never-seen pages. The
existing shared drop permit, fresh sample between drops, two-second sampling
limit and teardown grace apply. Settling removes that tree's pending stack entries
and destroys its never-seen views, including an adopted candidate; queueing in a
settled tree enrolls no work. Seen-page checkpoints, holds and revival are unchanged.

Favicon revisions follow the cache's exact hostname, including pages borrowing a
site icon. An arriving icon refreshes retained images on every chrome surface.
Start cards carry miniature icon URLs independently of the current-tree visit
map. Reopening clears the old cards. Rust sends saved trees and bookmarks first;
the view acknowledges two animation frames before asking Rust to rank suggestions.
Forest values carry only the shown view. Canopy lays out the open trees and History lists settled trees. History groups settled trees by local settlement day and starts with twenty rows, extending at either edge on scroll, after a When jump, or to fill a tall window. Its index is transient per call, with no retained History cache. Each row shows its time, miniature, title, page-name trail, sites and page count. Selecting a row unfolds its outline in place, capped at forty pages; choosing a page reopens the tree there. Find matches tree titles, sites and page names and marks matching title and trail text. The When rail jumps by period, and Sites filters by site, with chips on narrow windows. Selected or hovered rows offer Reopen, Save for later and Delete. Delete and Delete day keep Undo available until the next delete or the forest closes. Shared frames omit inactive trees’ page details. Closing clears each view’s state and Delete Undo. The shared parser forwards view-specific messages to the corresponding module, so the two views can evolve independently.

The new-tree page shows a How trees work card until Got it updates session.trees_help. Settings and F1 persistently restore the card and open the start surface. Rust sends its static lines and legend once when the chrome is ready; the view reuses the page marks and miniature drawing. A successful queued-link context callback emits Waiting once. HtmlChrome groups feedback from the same current parent within 2.6 seconds. The After item and waiting counter glow once for 320 ms, with no animation under reduced motion.

HtmlChrome owns a session-only error history (newest ten), its unread mark and the count behind the persistent error toast. Reading the list acknowledges the mark; dismissing a toast leaves history. Errors use a lazily created bottom-centred WebKit view, allocated only to the message and its shadow; a persistent error does not extend the main chrome view beyond its usual cover. Only the error buttons enter its input region. The error keeps its own alert live region, and status and settle feedback keep their positions. Error controls and their positions survive replacements during a press. Rust chooses one Escape target: an open surface or modal, the find bar (closed synchronously), an error with chrome focus, then Stop in Ui. Closing find keeps chrome focus when an error remains so a second Escape can dismiss it. Ctrl+W closes a surface or settles without dismissing errors. The waiting callback refreshes the count before starting the pulse so the first After item participates.
