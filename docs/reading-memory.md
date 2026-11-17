# Keeping an exploration

The reference scenario starts with a comment that holds three links. All three destinations stay in the tree, only one is opened, then the browser is closed. On resume, a search through the saved text finds the article, the comment it came from and the two pending links.

## A pending link is a visit to open

A visit can be pending or opened. A click that would normally open a background tab creates a pending child, without changing the current page or loading the destination. Activating that child keeps its id. The actual load marks it as opened.

A failure keeps the child pending and keeps the last current page. A redirect belongs to the same child. Two comments can hold the same destination and produce two distinct children, each with its own source.

MVP1 visits stay opened after migration. SQLite keeps the forest, the states and the cursor. The widgets show projections of this data.

## The source belongs to the context of the click

A child can keep the URL of the source page, the link text, an excerpt of the comment and a more precise citation. The source page and the citation are two different things. A citation can point to a particular comment in a page or to another permalink on the same site.

Going back to the source restores the parent visit. Opening a distinct citation does not turn the parent's URL into another page.

WebKit's native signals decide the destination and how it opens. A script limited to the main document supplies context from an isolated JavaScript world. This script receives no visit id and no way to write to the database.

The excerpt is attached to the link only if the destination and the document still match. Missing information stays missing. It must not be replaced by the context of another click.

## A capture is dated text

Captures are local texts tied to a visit and to the document's URL. They keep their date. A new version can be added when the content changes. Identical automatic captures do not need to be duplicated.

Automatic capture applies to opened pages. Delayed, bounded checks pick up content that appears shortly after the load. Pending links are not downloaded to be indexed. An explicit action also saves text selected in a page or a thread.

Each text is limited to 200,000 characters. Extraction leaves out form fields and editable areas, even when they are hidden. A visible password form blocks automatic capture. A visible control turns automatic capture off.

The displayed document has an identity separate from the requested navigation. An asynchronous extraction keeps that identity. A late result must not become the capture of another visit, even if both visits have the same URL.

## Search uses the saved texts

FTS5 indexes the capture text, its title and its source context. Titles and URLs stay searchable when a visit has no capture. Results tell apart several texts saved from the same thread.

Reading a capture uses a native text view and works without reloading the site. Opening the live page or going back to its source is a separate action.

The imported Chrome corpus stays a separate dataset. The feature does not download its old URLs and does not claim to rebuild the posts read in a thread from the history.

## The choices made

Two proposals were compared. The chosen base keeps navigation authority on the native side, captures per visit and FTS5 search. The alternative brought the isolation of the context script and checks on user input.

Merging pending links by URL was rejected, because it would lose distinct sources. A global model of canonical documents and replacing old captures are not needed for the resume scenario.

Precise comment recognition depends on the site's DOM. When fine-grained context is unavailable, the link must still keep its parent page. Captures are texts, not full interactive copies of pages. Thread content that appears long after the load can be saved by explicit selection. Scrolling is not recorded continuously.
