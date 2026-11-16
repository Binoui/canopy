# Working on Canopy

## Check the project

Check the formatting, the core tests and the compiler diagnostics:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The Bitwarden helper has its own tests, which run it against a stand-in for `bw`:

```sh
python3 helpers/test_canopy_login_bitwarden.py
```


The GUI tests load locally served pages and drive the window with real pointer and keyboard input. They check the HTML chrome through its own DOM. Install Xvfb and xdotool, then run every suite like this. Unsetting `WAYLAND_DISPLAY` keeps the window on Xvfb from a Wayland session:

```sh
for suite in tests/*_smoke.rs; do
  env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s '-screen 0 1440x1000x24' \
    cargo test --locked --features gui-smoke --test "$(basename "$suite" .rs)" || echo "FAILED $suite"
done
```

Each window suite uses a private, short runtime directory under /tmp and removes it on exit: WebKit leaves sandbox folders behind in the session runtime.
An explicitly supplied XDG_RUNTIME_DIR outside /run/user/<uid> is respected, so external test runners can own cleanup.

Run them all before merging a change to the chrome or the store, and compare any failure with the main branch.

`open_url_smoke` starts the `canopy` binary on temporary data folders and launches it again with addresses, so it checks the hand-off between real processes. It needs no session bus.

The download test points `HOME`, `XDG_CONFIG_HOME` and `XDG_DATA_HOME` at a temporary folder, so it never writes to your real download folder and opens files with a stand-in app instead of yours. Set `CANOPY_DOWNLOAD_SHOTS` to a folder to keep screenshots of the leaves and their list.

`newtree_smoke` checks pinned cards, the tab menu, new-tree typing and focus, the empty strip placeholder, and scrolling at 900 by 600. Set `CANOPY_NEWTREE_SHOTS=1` to save dark and light captures in `.cache/shots/`.

## Retake the README's pictures

The pictures and the short film in the README come from the real window, browsing made-up sites that `tests/readme_site` serves on this computer. Nothing is fetched from the web. After a change to the chrome, retake them all with one command. It needs Xvfb, xdotool and ffmpeg:

```sh
scripts/readme-shots.sh
```

It writes `docs/assets/shots/`. `tests/readme_shots.rs` holds the browsing session and the list of pictures. Run without `CANOPY_README_SHOTS`, that test does nothing.

## Where things live

| Path | What it holds |
| --- | --- |
| `src/browser.rs`, `src/browser/` | The forest: visits, trees, bookmarks, history and saved text, in SQLite. The only writer of the database. |
| `src/pages/` | The web views that show pages, one per live branch, and what Canopy reads from them. |
| `src/chrome/` | The chrome, drawn in HTML over the page: tabs, strip, palette, cards, new-tree page, forest view. |
| `src/ui/` | The window, the keyboard shortcuts and the glue between pages and chrome. |
| `src/suggest.rs`, `src/naming.rs` | Suggestions from your history, and automatic tree names. |
| `src/logins.rs`, `src/login_helper.rs`, `helpers/` | The login vault, and the helpers that reach a password manager. |
| `src/import/` | Bringing history, bookmarks, cookies and passwords from Chrome. |
| `src/instance.rs` | One Canopy per data folder, and the hand-off of links from a second launch. |
| `tests/` | The GUI suites, which drive the real window. |
| `docs/` | Design notes, the guide and the README's pictures. |
