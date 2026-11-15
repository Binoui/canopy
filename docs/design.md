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

Pages in src/pages/mod.rs owns the window's Browser, PageContext and the branch registry. Ui owns the window, HTML chrome and cookie inbox. It translates navigation intents into Go commands and reads the forest through Pages::read, which exposes only a shared Browser reference. Pages owns settling and startup selection, including blanking the live page when the last tree is settled.

PageContext in src/pages/native.rs owns the content WebContext, its persistent data and cookie manager, favicon database and download handler. LivePage in src/pages/live.rs builds each page in that context and keeps navigation and native history. Its callbacks enter Pages through a weak state reference. A callback guard retains the owner for the entire callback, including reentrant observer calls, and reaches the sole Browser writer. Ui applies cookie batches through Pages with its existing startup gate and serialized drain. The chrome keeps its separate wry context.

Pages keeps branch-palette and path continuations by LoadTicket. A continuation waits for its matching commit, then for that attempt to finish loading. A same-document commit is ready immediately. Once ready, it runs exactly once on a GTK idle after the native document has been bound, even if the page starts another navigation first. The path's second address attaches to the visit committed by its first attempt, including when that page refreshes itself. A native replacement before readiness, a failure of the waiting attempt, an explicit cancellation, blanking, or settling discards it. A Pages command cancels when it starts a new attempt; Back and reload also clear a path continuation when they do nothing. A no-op Back or reload preserves a pending branch palette. Pages issues the path's second address itself, or reports OpenBranchPalette to Ui through its one PageEvent observer.

Downloads find their source with WebKit's download webview lookup. PageContext asks that view's weakly registered state for the visit at transfer start. A transfer retains the context, which reports through a weak Pages reference to record the saved file. Saving and recording do not depend on the source view surviving while the window owner remains alive. Its event sink retains no page. Favicons retains the database and observes database changes and page icon notifications through weak cache references.

Pages clones a live-page handle or observer before calling it. Browser, navigation and continuation borrows end before WebKit or GTK operations or observer callbacks that can reenter. The native adapter clones a document before binding its JavaScript epoch. Read closures contain database queries only. The selected branch has one view. Other branches retain in-memory native history checkpoints; visits outside those histories stay in SQLite.

Back uses an entry still present in WebKit's history only when its visit belongs to the current open tree, otherwise the parent in the forest. Page-script and mouse Back cannot reopen a settled tree or cross into another open tree. Back from the blank start page does nothing. The list's changed signals drop the bindings of removed entries and bind same-document entries to their visit. Each entry also keeps its native document group, because a return can move the list before Started. This group tells a return within the same document from a full load, even at an identical URL. A full commit gets a fresh group unless it is a back or forward traversal. A traversal onto an unbound entry falls back to the forest parent. The native cursor is read at changed or Committed, never at notify::uri. A restore only traverses an entry from another native document. On a pushState route, F5 and a visit-node click load the stored visit URL, so neither leaves an attempt waiting for load signals that a same-document traversal never sends. A cold resume reloads the URL and restores the recorded scroll. WebKit's page cache can keep the DOM and the forms of an entry still present.

## The chrome is an HTML view above the page

The tabs, the tree strip, the palette, the cards, the menus, the "New tree" page, the forest view and the reader are drawn in HTML. A second, transparent webview, served by the `canopy://` scheme, sits above the page. Rust keeps the state and every decision: the layouts, the palette results and the intents. The view receives JSON and sends clicks and keys back over IPC.

At rest, the view covers only the top 94 px. It grows to cover whatever is open (open strip, card, menu) and covers the whole window only for the palette, the reader and full-page surfaces. The page therefore gets clicks everywhere else. During the 220 ms the strip takes to close, the open area keeps the clicks.

This choice replaces the GTK and cairo chrome. It costs a second WebKit process, measured at about 124 MB of PSS. In exchange, a single interface runs on Linux, macOS and Windows. On Linux, GTK remains the invisible host of both webviews, as in Tauri. The page engine and the host window stay tied to WebKitGTK until the port to macOS and Windows.

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

The minimap, the graph layout, page extraction, summaries and the agent wait for a later milestone. The live page has no generic trait. Moving to Servo or CEF will mean revisiting the rendering and profile integration.

In-app changes inside web applications remain a separate case. The MVP does not promise to turn every JavaScript state change into a visit.

## Branch registry and switching

Pages owns the selected live branch, an optional candidate, and a linear MRU list
of dormant branch checkpoints. There is no visit-to-branch index. Each checkpoint
is the continuation of its lane's native history; lane membership is not persisted.
A view follows links and forks in place. WebKit removes abandoned forward entries;
opening those stored visits later starts fresh branches. Roots and first activations
of queued visits also start fresh branches. Queuing never fetches a page.

Routing checks a candidate's pending Restore intent first, including after Started.
It then checks the selected document, actual selected history, and dormant histories
from most recently selected to oldest. Identity is always a visit binding, never URL
equality. A dormant branch revives at the requested entry, including a middle entry.
Otherwise selection cold-loads a new branch. Back uses a native back entry in the
current open tree, or routes the forest parent when no eligible item exists.

Candidates load hidden, without focus. Their successful commit writes the cursor,
tree target and arrival once, in Browser's transaction, and synchronously promotes
the candidate. The old widget is then unmapped with GTK child visibility and the candidate shown. It takes
focus only when the previous page held it, preserving an active chrome editor.
Pages places and raises each view at creation, show and map, before page events.
Candidates are realized while hidden. A failure before commit reports the normal error
and preserves the old selected page and cursor. A retry routes normally. A second
request cancels the candidate by its own ticket and destroys it; selecting the same
pending visit reuses it. A cancelled or failed revival retains its checkpoint.

After promotion, the previous view cancels pending navigation through failed-load
resolution, captures scroll with a 500 ms limit, serializes and is destroyed. The
selected page stays usable. Navigation, reload and settle commands arriving during release enter a FIFO; path
and branch-palette continuations travel with queued commands. They drain when
release completes. Focus, resizing, cookies and downloads continue directly.
A failed checkpoint drops the lane, leaving its visits available cold. Settling a
tree discards its dormant lanes; a release completing after settlement cannot add
them back. Its landing uses the same routing.

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

The next step may retain several live lanes under a measured memory budget. It
must preserve this routing and selection transaction while replacing release at
every switch with budget-driven release. There is no presence marker or reopening
surface in this step.

### Switch cancellation and page Back

Starting a candidate fails all pending foreground work in the old view and invalidates
its asynchronous link decisions. The old committed document remains usable. Main-frame
Started for another document cancels the candidate; same-document changes (pushState,
replaceState and fragments) and subframe policy decisions do not. Settling cancels
only a candidate targeting that tree. Settling the last tree creates a fresh home view, so the next branch cannot inherit settled history. Release
keeps a valid checkpoint when its current entry belongs to an open tree.

A restored POST that is current at initial revival is fetched by GET. Its duplicate
native entry is skipped by page Back; the GET drops forward entries, retaining the
back list. A noncurrent restored POST opens by GET in a fresh branch instead of
appending into the revived list. Its Back routes to its forest parent, reviving the
previous branch at that entry. The checkpoint and its forward entries remain dormant;
no form is resubmitted. This preserves the existing current-POST revival behavior.
Back selects the nearest previous page visit, skipping same-page frame and alias
entries. If none exists, it routes to the forest parent. Only when neither exists
may it traverse an earlier same-page state. Mouse Back runs this same command;
mouse Forward stays native. Saved captures route by visit identity.
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

Pages::select_candidate switches bookkeeping and keyboard focus at commit. It
maps the incoming page under the outgoing native window, whose GDK input is made
pass-through immediately. Only that outgoing picture is held. Two isolated-world
animation frames after a body exists request a rendered snapshot. Pages::present_snapshot
paints this actual document image after the incoming widget's default draw, so an
empty native backing store cannot show through. Only after that draw does
Pages::release_picture hide/checkpoint/release the outgoing view and drain the FIFO.
Subsequent commands use that same queue, so another switch cannot retain a third
page's picture. Failures/cancellation before commit never enter this handoff.
The temporary image scales with the slot while resizing. Native draws are probed
without the image; once the backing store matches it or contains rendered pixels
rather than an empty transparent/grey/white frame, the image handler is removed.
No timer reveals an empty backing store: both 300 ms and 1000 ms cutoffs failed
pixel sampling under load. A renderer that never supplies a snapshot retains the
outgoing picture. This deliberately favours a continuous picture over a deadline.
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
