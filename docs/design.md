# A forest of visits

Canopy keeps the context of a browsing session. A visit has a URL, a title and an optional parent. Each root starts a tree.

After A then B, going back to A and following C gives A two children. Visit B stays reachable. Following a URL again can create another visit, because the context has changed.

## SQLite owns the forest and the cursor

The database holds the visits and the foreground cursor. Browser stays the only writer of SQLite. A foreground commit writes the visit, the cursor, the tree's target and the arrival log in one transaction. A hidden commit writes the visit without moving the other three.

Two structures were compared before implementation. The first gave ownership of the state to SQLite. The second also kept a forest in memory. The first was chosen to avoid a second write path and keeping the cursor in sync.

The alternative brought two things to the chosen design. The navigation intent stays explicit until the load actually happens. The tests check the behaviour with WebKit and compare the forest before and after reopening the database.

## An intent comes before a visit

Each view owns its Navigation, its pending load and its current Document. Browser owns no global load. A LoadTicket ties one incarnation of a view to an attempt number. Title, capture, context and scroll callbacks carry the Document they came from and check its epoch before writing. A failed transaction keeps the load and the Document until Finished. Finished abandons an attempt whose commit did not succeed. A same-document completion abandons a rejected commit immediately, because WebKit sends no Finished for it. The live page does not retry commits.

A new navigation records its parent before loading. A restore records the id of the visit to reuse. The live page accepts Started for an issued attempt only after WebKit accepts policy for its requested URL or history item. Earlier Started signals cannot adopt a replacement request. The live page then confirms the URL when the main document loads.

The live page distinguishes an issued request from a started load. Only a started load is stopped and drained before its replacement is issued. The queued replacement retains its own ticket. A same-document request needs no Finished signal. A page-started load cannot adopt a request the live page has not issued.

Server redirects belong to a single navigation. A reload or a restore does not create a visit. A failure keeps the last recorded position and shows an error.

Persistent ids are ULIDs. URLs are not visit ids. Subresources and embedded frames do not become children. A native Reload is a candidate only when its request URL equals the current document URL. It becomes a restore only at main-frame Started, so an iframe reload cannot suspend document writes. A title updates the visit only while the document URL equals its stored URL.

## GTK and WebKit stay on their thread

WebKitGTK builds the page view inside a GTK container. wry still builds the chrome view. The native signals tell apart the start, the commit and the failure of a load.

Pages in src/pages/mod.rs owns the window's Browser, PageContext and one LivePage. Ui owns the window, HTML chrome and cookie inbox. It translates navigation intents into Go commands and reads the forest through Pages::read, which exposes only a shared Browser reference. Pages owns settling and startup selection, including blanking the live page when the last tree is settled.

PageContext in src/pages/native.rs owns the content WebContext, its persistent data and cookie manager, favicon database and download handler. LivePage in src/pages/live.rs builds the single page in that context and keeps navigation and native history. Its callbacks enter Pages through a weak state reference. A callback guard retains the owner for the entire callback, including reentrant observer calls, and reaches the sole Browser writer. Ui applies cookie batches through Pages with its existing startup gate and serialized drain. The chrome keeps its separate wry context.

Pages keeps branch-palette and path continuations by LoadTicket. A continuation waits for its matching commit, then for that attempt to finish loading. A same-document commit is ready immediately. Once ready, it runs exactly once on a GTK idle after the native document has been bound, even if the page starts another navigation first. The path's second address attaches to the visit committed by its first attempt, including when that page refreshes itself. A native replacement before readiness, a failure of the waiting attempt, an explicit cancellation, blanking, or settling discards it. A Pages command cancels when it starts a new attempt; Back and reload also clear a path continuation when they do nothing. A no-op Back or reload preserves a pending branch palette. Pages issues the path's second address itself, or reports OpenBranchPalette to Ui through its one PageEvent observer.

Downloads find their source with WebKit's download webview lookup. PageContext asks that view's weakly registered state for the visit at transfer start. A transfer retains the context, which reports through a weak Pages reference to record the saved file. Saving and recording do not depend on the source view surviving while the window owner remains alive. Its event sink retains no page. Favicons retains the database and observes database changes and page icon notifications through weak cache references.

Pages clones a live-page handle or observer before calling it. Browser, navigation and continuation borrows end before WebKit or GTK operations or observer callbacks that can reenter. The native adapter clones a document before binding its JavaScript epoch. Read closures contain database queries only. A single webview loads the active page. The other visits stay in SQLite.

Back uses an entry still present in WebKit's history only when its visit belongs to the current open tree, otherwise the parent in the forest. Page-script and mouse Back cannot reopen a settled tree or cross into another open tree. Back from the blank start page does nothing. The list's changed signals drop the bindings of removed entries and bind same-document entries to their visit. Each entry also keeps its native document group, because a return can move the list before Started. This group tells a return within the same document from a full load, even at an identical URL. A full commit gets a fresh group unless it is a back or forward traversal. A traversal onto an unbound entry falls back to the forest parent. The native cursor is read at changed or Committed, never at notify::uri. A restore only traverses an entry from another native document. On a pushState route, F5 and a visit-node click load the stored visit URL, so neither leaves an attempt waiting for load signals that a same-document traversal never sends. A cold resume reloads the URL and restores the recorded scroll. WebKit's page cache can keep the DOM and the forms of an entry still present.

## The chrome is an HTML view above the page

The tabs, the tree strip, the palette, the cards, the menus, the "New tree" page, the forest view and the reader are drawn in HTML. A second, transparent webview, served by the `canopy://` scheme, sits above the page. Rust keeps the state and every decision: the layouts, the palette results and the intents. The view receives JSON and sends clicks and keys back over IPC.

At rest, the view covers only the top 94 px. It grows to cover whatever is open (open strip, card, menu) and covers the whole window only for the palette, the reader and full-page surfaces. The page therefore gets clicks everywhere else. During the 220 ms the strip takes to close, the open area keeps the clicks.

This choice replaces the GTK and cairo chrome. It costs a second WebKit process, measured at about 124 MB of PSS. In exchange, a single interface runs on Linux, macOS and Windows. On Linux, GTK remains the invisible host of both webviews, as in Tauri. The page engine and the host window stay tied to WebKitGTK until the port to macOS and Windows.

## Saved logins are filled on request

A LoginProvider answers two questions: which accounts are saved for a page's origin, and
what an account's password is. The encrypted vault in src/logins.rs is the first provider.
Ui owns the provider, so a password manager the user already has can stand in for the vault
without the page or the chrome changing.

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

## The MVP1 scope

The first milestone checks navigation, forks and resuming after a restart.

The minimap, the graph layout, page extraction, summaries and the agent wait for a later milestone. The live page has no generic trait. Moving to Servo or CEF will mean revisiting the rendering and profile integration.

In-app changes inside web applications remain a separate case. The MVP does not promise to turn every JavaScript state change into a visit.

## In-memory page checkpoints

The per-branch rollout will call the private checkpoint lifecycle in Pages. Its
only current callers are GUI probes. A checkpoint owns serialized WebKit session
bytes, entry records in native list order, and the current position. Each entry
records its visit, document group, native URL, stored visit URL, and whether its
request used POST. The two URLs allow a pushState entry to alias a visit without
changing that visit's stored address. Checkpoints contain no native objects and
never enter SQLite or logs.

Release takes the selected page out of its slot, cancels its pending navigation
and queued capabilities, and stops loading before asking its isolated script
world for the displayed document's URL and scroll. The pending completion owns the source view.
The answer saves scroll only for the same valid document and URL. A 500 ms GLib
timeout cancels the query and completes release if the web process does not answer.
The main loop must be serviced for either completion to run. A hung main loop has
no wall-clock guarantee. On timeout, the most recent SQLite and WebKit values can
still miss recent scrolling. The query measures scroll when the web process handles
it, so scrolling between the release request and that answer can enter the snapshot.

Pending navigation cancellation uses Navigation::failed. Checkpointing discards
only a continuation owned by that view and checks the native list after the query. An unbound item, missing position, duplicate binding, or
stale binding makes checkpointing fail. Entries without a Canopy visit, including
about:blank and blob: documents, also make checkpointing fail. Release still destroys
the view and permits a cold restore from SQLite. No form or script state survives.

Release disconnects handlers, clears item references, unparents the view, and
destroys it. The lifecycle gate rejects native and capture callbacks during the
scroll query and after release. The query completion alone can save the final
scroll. Weak references and load and document generations also reject late work. Callers hold no selected
slot borrow while calling observers. During the release wait, the slot is empty.
Resize and focus tolerate that state. Navigation and capture requests return an
error. Revival waits for release to complete before mounting a replacement.

Revival creates a fresh Navigation and view without loading HOME. Restoring the
session starts no load. Validation checks list length, current position, native
URLs, and stored visit URLs before rebinding every position. A mismatch or absent
target takes the cold path. Alias-only targets reload their stored URL cold.
The saved current entry wins over another entry showing the same visit and URL.
SQLite scroll applies only when reviving the entry current at checkpoint time.
Revived views follow their container allocation. Their scroll helper waits for
two consecutive rendered frames with the same size before applying SQLite scroll, because
WebKit can finish loading before receiving GTK's final viewport. The wait ends after
30 frames even if the size keeps changing. Normal live-view
restoration keeps its existing timing. Other entries retain WebKit's scroll,
captured when the document was left.
Native back and forward retain the open-tree restriction.

A revival never resends a form. A POST target in a checkpoint takes the cold path
with GET. Other restored POST entries remain ineligible for native traversal until
loaded again in this view. Native Back or Forward to one of those entries is refused
once and replaced by a GET of its stored visit address. The replacement excludes
that native entry, so it cannot select the same refused entry again. A main-frame
request supplies one method value, refreshed on redirects and consumed when its entry is bound. A GET reload
refreshes an existing item's method. New same-document entries inherit the main
document's method, including entries added by iframe navigation. Iframe decisions
retain WebKit's behavior.
Ordinary views retain WebKit's existing form navigation, including cached state and
POST resubmission when the page cache has evicted a result. Pages ignore
window.close() so page script cannot destroy the browser's page view.

Commands that need a selected page check the slot before changing the forest.
Startup can queue a landing without a page. Resume preserves that landing until a
page is available. Policy decisions during release are refused while the final
scroll query checks both the URL and the isolated world's document epoch.

### Next PR

A hidden revived view restores scroll only after it is shown. Its first visible
frames can show the top. Revival refuses while a release is pending, so the old
page cannot yet remain on screen until the revived page commits. Release can take
500 ms, during which commands requiring a selected page fail.
