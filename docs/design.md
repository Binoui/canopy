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

Back uses an entry still present in WebKit's history only when its visit belongs to the current open tree, otherwise the parent in the forest. Page-script and mouse Back cannot reopen a settled tree or cross into another open tree. Back from the blank start page does nothing. The list's changed signals drop the bindings of removed entries and bind same-document entries to their visit. Each entry also keeps its native document group, because a return can move the list before Started. This group tells a return within the same document from a full load, even at an identical URL. A full commit gets a fresh group unless it is a back or forward traversal. A traversal onto an unbound entry falls back to the forest parent. The native cursor is read at changed or Committed, never at notify::uri. A restore only traverses an entry from another native document. On a pushState route, F5 and a visit-node click load the stored visit URL, so neither leaves an attempt waiting for load signals that a same-document traversal never sends. A cold resume reloads the URL and restores the recorded scroll. WebKit's page cache can keep the DOM and the forms of an entry still present.

## The chrome is an HTML view above the page

The tabs, the tree strip, the palette, the cards, the menus, the "New tree" page, the forest view and the reader are drawn in HTML. A second, transparent webview, served by the `canopy://` scheme, sits above the page. Rust keeps the state and every decision: the layouts, the palette results and the intents. The view receives JSON and sends clicks and keys back over IPC.

Rust sizes the active tab first from its title width, icon, padding and settle cross, between 110 and 200 px, capped at the room left after the overflow arrow. Other tabs share the remaining room as titles, down to about six characters with their cross overlaid on hover. Tabs become icons one at a time, farthest from the active tree first, then hide behind the arrow. Icon tabs keep a small green waiting count. The active waiting count sits at the icon corner too, preserving title room. Short titles keep the pin in the tooltip and overflow row; wider titled tabs retain the inline pin, whose width Rust includes. A settle click requires the cross to have been visible under the pointer at press time and a click count of one. The cross draws a 16 px hover shape inside its 24 px hit area. The overflow list includes waiting counts, pins and a settle button. Rust remembers the room measured by the view and sends a new tab fit in every frame. The view applies that fit in the same presentation call as the active class, without another round trip. Ctrl+Shift+Tab uses the visit log in descending event order to find the most recently shown other open tree.

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

Pages owns one selected live branch, hidden live branches ordered by their last
shown stamp, dormant checkpoints, and at most one candidate. The window increments
its own counter at selection. There is no visit-to-branch index and routing never
compares URLs. Routing checks a candidate's pending Restore, the selected document
and history, then hidden live and dormant histories in MRU order. A tree-tab return
uses the most recently shown branch's current document, including hidden redirects;
a specific visit-node selection still requests that historical visit. A view follows
links and forks in place; queued visits still wait until first selection to load.

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
forest, tree sheet tree/list and tabs share the mark; a tab uses its tree's last
visit. Preview metadata, node tooltips and tree list details say “Open in memory”.
Never-seen pages keep their separate top-right waiting dot. Absolute positioning
preserves node geometry. Hold completion, pressure removal and hidden process death
notify Changed; candidate history routing and cancellation also notify when a page
leaves or rejoins the hidden registry. Selection, hidden navigation and settling use
existing refreshes.

### Switch cancellation and page Back

Starting a candidate fails all pending foreground work in the old view and invalidates
its asynchronous link decisions. The old committed document remains usable. Main-frame
Started for another document cancels the candidate; same-document changes (pushState,
replaceState and fragments) and subframe policy decisions do not. Settling cancels
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
