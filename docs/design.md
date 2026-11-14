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

## The MVP1 scope

The first milestone checks navigation, forks and resuming after a restart.

The minimap, the graph layout, page extraction, summaries and the agent wait for a later milestone. The live page has no generic trait. Moving to Servo or CEF will mean revisiting the rendering and profile integration.

In-app changes inside web applications remain a separate case. The MVP does not promise to turn every JavaScript state change into a visit.

## In-memory page checkpoints

The per-branch rollout will call the private checkpoint lifecycle in Pages. Its
only current callers are GUI probes. A checkpoint owns serialized WebKit session
bytes, entry records in native list order, and the current position. Each entry
records its visit, document group, native URL, and stored visit URL. The two URLs
allow a pushState entry to alias a visit without changing that visit's stored
address. Checkpoints contain no native objects and never enter SQLite or logs.

Checkpointing cancels the pending ticket through Navigation::failed and discards
continuations. It stops loading before checking the native list. An unbound item,
a missing position, a duplicate binding, or a stale binding makes checkpointing
fail. Release destroys the widget after disconnecting its handlers and clearing
item references. Callback entry also rejects a detached state.

Revival constructs a fresh Navigation and view. Session restoration starts no
load. Before binding any items, it checks the list length, current position,
native URLs, and stored visit URLs. It then rebinds visits and document groups by
position. A mismatch destroys that candidate and uses a fresh view for a cold
restore. A requested visit outside the checkpoint also takes the cold path.
Traversal uses the existing restore intent, Issue state, policy handler, and
commit path. Form and script state do not survive. Per-visit scroll remains in
SQLite, and native back and forward retain the open-tree restriction.

Checkpointing disables native callbacks before stopping the load. Released state
also blocks event emission from a callback that was already running when an
observer released the page. Pages releases its live-page lease before any widget
destruction. The callback guard, native traversal, session restore, and observer
calls run without a Pages, Browser, or navigation RefCell borrow.

If only aliased native URLs remain for the requested visit, revival falls back
to that visit's stored URL. This preserves the existing rule that selecting a
visit reloads its stored address. Checkpoint failure also releases the source;
the caller can cold-restore from SQLite without a checkpoint.

A revived incarnation applies the visit's persisted scroll through the existing
scroll-restoration helper after loading, including when it traversed native
history. Normal traversal in an already loaded view still uses native history's
scroll. The mount probe installs its replacement before releasing the old page through
the shared checkpoint/release helper, so destroy callbacks can reenter Pages.
