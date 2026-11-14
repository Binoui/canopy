# A forest of visits

Canopy keeps the context of a browsing session. A visit has a URL, a title and an optional parent. Each root starts a tree.

After A then B, going back to A and following C gives A two children. Visit B stays reachable. Following a URL again can create another visit, because the context has changed.

## SQLite owns the forest and the cursor

The database holds the visits and the foreground cursor. Browser stays the only writer of SQLite. A foreground commit writes the visit, the cursor, the tree's target and the arrival log in one transaction. A hidden commit writes the visit without moving the other three.

Two structures were compared before implementation. The first gave ownership of the state to SQLite. The second also kept a forest in memory. The first was chosen to avoid a second write path and keeping the cursor in sync.

The alternative brought two things to the chosen design. The navigation intent stays explicit until the load actually happens. The tests check the behaviour with WebKit and compare the forest before and after reopening the database.

## An intent comes before a visit

Each view owns its Navigation, its pending load and its current Document. Browser owns no global load. A LoadTicket ties one incarnation of a view to an attempt number. Title, capture, context and scroll callbacks carry the Document they came from and check its epoch before writing. A failed transaction keeps the load and the Document until Finished. Finished abandons an attempt whose commit did not succeed. The engine does not retry commits.

A new navigation records its parent before loading. A restore records the id of the visit to reuse. The engine accepts Started for an issued attempt only after WebKit accepts policy for its requested URL or history item. Earlier Started signals cannot adopt a replacement request. The engine then confirms the URL when the main document loads.

The engine distinguishes an issued request from a started load. Only a started load is stopped and drained before its replacement is issued. The queued replacement retains its own ticket. A same-document request needs no Finished signal. A page-started load cannot adopt a request the engine has not issued.

Server redirects belong to a single navigation. A reload or a restore does not create a visit. A failure keeps the last recorded position and shows an error.

Persistent ids are ULIDs. URLs are not visit ids. Subresources and embedded frames do not become children. A native Reload is a candidate only when its request URL equals the current document URL. It becomes a restore only at main-frame Started, so an iframe reload cannot suspend document writes. A title updates the visit only while the document URL equals its stored URL.

## GTK and WebKit stay on their thread

WebKitGTK builds the page view inside a GTK container. wry still builds the chrome view. The native signals tell apart the start, the commit and the failure of a load.

Borrows of the state are released before calls that can trigger another callback. A single webview loads the active page. The other visits stay in SQLite.

Back uses an entry still present in WebKit's history, otherwise the parent in the forest. The list's changed signals drop the bindings of removed entries and bind same-document entries to their visit. Each entry also keeps its native document group, because a return can move the list before Started. This group tells a return within the same document from a full load, even at an identical URL. A full commit gets a fresh group unless it is a back or forward traversal. A traversal onto an unbound entry falls back to the forest parent. The native cursor is read at changed or Committed, never at notify::uri. A restore only traverses an entry from another native document. On a pushState route, F5 and a visit-node click load the stored visit URL, so neither leaves an attempt waiting for load signals that a same-document traversal never sends. A cold resume reloads the URL and restores the recorded scroll. WebKit's page cache can keep the DOM and the forms of an entry still present.

## The chrome is an HTML view above the page

The tabs, the tree strip, the palette, the cards, the menus, the "New tree" page, the forest view and the reader are drawn in HTML. A second, transparent webview, served by the `canopy://` scheme, sits above the page. Rust keeps the state and every decision: the layouts, the palette results and the intents. The view receives JSON and sends clicks and keys back over IPC.

At rest, the view covers only the top 94 px. It grows to cover whatever is open (open strip, card, menu) and covers the whole window only for the palette, the reader and full-page surfaces. The page therefore gets clicks everywhere else. During the 220 ms the strip takes to close, the open area keeps the clicks.

This choice replaces the GTK and cairo chrome. It costs a second WebKit process, measured at about 124 MB of PSS. In exchange, a single interface runs on Linux, macOS and Windows. On Linux, GTK remains the invisible host of both webviews, as in Tauri. The page engine and the host window stay tied to WebKitGTK until the port to macOS and Windows.

## The MVP1 scope

The first milestone checks navigation, forks and resuming after a restart.

The minimap, the graph layout, page extraction, summaries and the agent wait for a later milestone. The engine has no generic trait. Moving to Servo or CEF will mean revisiting the rendering and profile integration.

In-app changes inside web applications remain a separate case. The MVP does not promise to turn every JavaScript state change into a visit.
