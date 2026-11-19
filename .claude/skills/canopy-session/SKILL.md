---
name: canopy-session
description: See what the user has open in Canopy, their tree-based web browser - the page on screen, its visible text, every open tree with its pages, and trees saved for later. Use when the user points at something in their browser ("this page", "the article I'm reading", "my open tabs", "what I was looking at", "the docs I have open"), or when knowing their current browsing would answer the question better than a web search.
---

# Canopy session

Canopy is the user's browser. It keeps each tab as a tree of pages: a page opened from another page is its child. `canopy session` asks the running Canopy what it shows. It is read-only, takes about 40 ms, and never starts Canopy.

```sh
canopy session                # text: shown page, open trees with pages indented, saved trees
canopy session --json         # the same as JSON, for jq
canopy session --text         # adds the shown page's visible text (up to 200,000 characters)
canopy session --json --text
```

Use the plain form first. Add `--text` only when you need what the page says, since it can hold mail, messages or anything else the user is reading. Quote from it only what the task needs.

## Reading the text form

```
Shown: Tokio tutorial · Async in depth — http://docs.example/async
  loading                              (only while the page loads)

Open trees (3):
  1. Ask HN · What are you working on? (21 pages, 14 waiting, pinned)
    Ask HN · What are you working on? — https://news.ycombinator.com/item?id=1
      A Rust browser — https://github.com/...  [in memory]
      Tiny SAT solver — https://...  [waiting]
* 2. Tokio tutorial (10 pages)
    ...
        Tokio tutorial · Async in depth — http://docs.example/async  [shown]

Saved for later (1):
    Trip planning (6 pages)
```

- `*` marks the tree on screen, and `[shown]` the page on screen.
- `[waiting]` is a page opened in the background and not read yet. `[waiting, loaded]` means it is already loaded.
- `[in memory]` is a page read earlier that is still loaded, so going back to it is instant.
- Indentation is the tree: each page sits under the page it was opened from. Siblings are oldest first.
- `Shown: nothing` means the New tree tab or the start page is on screen.
- `Opening: ...` means a load was asked for but has not committed yet.

Settled pages (closed branches) and history trees are left out, as the window leaves them out.

## JSON fields

`shown` (`id`, `title`, `url`, `tree`, `loading`, `path` from the root), `pending` (`{"page": ...}` or `{"new_tree": url}` or null), `tabs[]` (`id`, `name`, `pinned`, `shown`, `waiting`, `last_page`, `pages[]` with `id`, `parent`, `depth`, `title`, `url`, `marks`), and `saved[]` (`id`, `name`, `page_count`, `last_page`). `--text` adds `text`, or `text_error` when the page did not answer within 3 s.

```sh
canopy session --json | jq -r '.shown.url'
canopy session --json | jq -r '.tabs[] | select(.shown) | .pages[] | "\(.depth) \(.title)"'
canopy session --json | jq -r '.tabs[].pages[] | select(.marks | index("waiting")) | .url'
```

## When it fails

- `Canopy is not running on ...` — the user has no Canopy window open. Say so. Do not start it.
- `Canopy did not answer. A Canopy started before this version cannot; restart it.` — the window predates `canopy session`. Ask the user to restart Canopy, and do not restart it yourself.
- `command not found` — Canopy is usually installed at `~/.local/bin/canopy`.
- A Canopy using another data folder: `canopy --data-dir PATH session`.
