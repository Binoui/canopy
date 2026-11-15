# Canopy

Canopy is a prototype Linux browser written in Rust. Your visits form trees. Going back to a page and following another link keeps both branches.

## Run the browser

Install stable Rust, GTK 3, WebKitGTK 4.1 and the SQLite development files.

On Debian or Ubuntu, install the native dependencies with this command:

```sh
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libsqlite3-dev bubblewrap xdg-dbus-proxy
```

On Arch Linux, use this command:

```sh
sudo pacman -S --needed base-devel pkgconf gtk3 webkit2gtk sqlite bubblewrap xdg-dbus-proxy
```

From the project folder, run this command:

```sh
cargo run --locked
```

To keep the data in a separate folder, pass its path:

```sh
cargo run --locked -- --data-dir .data
```

Canopy runs page and chrome processes inside WebKit's Linux sandbox, which needs `bubblewrap` and `xdg-dbus-proxy`. If the sandbox cannot start, WebKit stops with a fatal error. Setting `WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1` starts Canopy without it, and Canopy then prints a warning.

## Install for your launcher

To build an optimized binary into `~/.local/bin` and add Canopy to rofi, wofi or your desktop menu, run these commands:

```sh
cargo install --path . --locked --root ~/.local
install -Dm644 packaging/canopy.desktop ~/.local/share/applications/canopy.desktop
install -Dm644 assets/logo/canopy.svg ~/.local/share/icons/hicolor/scalable/apps/canopy.svg
for size in 16 22 24 32; do
  install -Dm644 assets/logo/canopy-small.svg ~/.local/share/icons/hicolor/${size}x${size}/apps/canopy.svg
done
```

Run the first command again after pulling changes. The binary embeds its font, so it does not need the project folder.

The icon is `assets/logo/canopy.svg`. The simpler `assets/logo/canopy-small.svg` replaces it at 32 pixels and below, where the full drawing blurs.

The `rust-toolchain.toml` file selects stable Rust for this project. It does not change your default version in other projects.

Canopy ships the Inter typeface in `assets/fonts/`, under the SIL Open Font License in `assets/fonts/LICENSE.txt`.

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

## Bring your Chrome history

To bring your Chrome history, bookmarks, cookies, and saved passwords into Canopy, run:

```sh
canopy --import-chrome
```

It reads `~/.config/google-chrome/Default`. Pass another profile folder after the flag to read that one instead, and add `--data-dir PATH` to import into a separate data folder. Chrome can stay open.

**Ctrl+K** then shows **Bookmarks** and **From Chrome** results next to your forest. Opening one loads it like a typed address. Until Canopy has habits of its own, the new-tree page offers your busiest Chrome sites. Imported pages plant no trees and add no tabs.

Cookies reach Canopy while it runs, or at its next start before it reloads your page or opens the addresses it was given. This keeps sites signed in when their sessions still work. Chrome's cookies replace matching Canopy cookies. Session cookies last until Canopy quits. Partitioned and expired cookies are skipped.

Passwords go into an encrypted vault. Its key stays in your system keyring. The import may ask you to unlock the keyring. Canopy does not fill password fields yet.

Running the command again refreshes imported pages and updates matching passwords without removing other saved accounts. Canopy loads none of the imported pages. If Chrome is writing a database, or the keyring is locked, resolve the reported error and run the import again. Cookies accepted just before a crash may need another import.

## Find your way around

The top row has one tab per open tree, in the order you started them. Trees that do not fit wait behind the arrow at the end of the row. A tree is named after what you searched for, or after its first page. Right-click a tab to give the tree a name of your own; an empty name goes back to the automatic one. Under the tabs, a strip shows the current tree: the path to the current page, with small buds for the other branches. Rest the pointer on the strip and it opens over the page with every branch. Rest it on a page in the strip to see a card with its title, site and a preview.

Window shortcuts require the listed modifiers. Caps Lock and Num Lock do not affect them.

- **Ctrl+K** or the **Search or go…** pill opens the palette. Type to search your forest and your saved text, or to go to an address. **Enter** opens the selected result. **Alt+Enter** opens it in a new tree. **Escape** closes the palette and gives the focus back to the page.
- **Ctrl+L** opens the same palette with the current address selected.
- **Ctrl+E** or the forest button opens every tree. Drag or scroll to pan, Ctrl+scroll to zoom, and use **Titles** to switch between pills and icons. Click a page to restore it.
- **Ctrl+T** or **+** opens the new-tree page with frequent pages and usual paths from your history. A usual path loads its first page as a root, then its second page as a child. **Escape** returns to your page.
- **Ctrl+W**, a middle click on a tab, or the **×** on a tab settles a tree. A settled tree leaves the tabs and keeps every page. It waits under **Settled** in the forest view, and opening any of its pages brings its tab back. With the new-tree page, the forest view or the palette open, **Ctrl+W** closes that instead.
- **Ctrl+1** to **Ctrl+9** switch to a tab. Shift is also accepted for layouts such as AZERTY.
- **F6** moves the keyboard focus between the page and the current page in the strip. **Left** and **Right** walk the path, and **Enter** opens the focused page.
- **Alt+Left** goes back to the parent page.
- **Ctrl+R**, **F5**, and **Ctrl+Shift+R** reload the current page. Canopy has no hard reload, so all three perform a plain reload.

## Try a fork

1. Press **Ctrl+K**, type an address and press **Enter**.
2. Follow a link to a second page.
3. Go back to the first page by clicking it in the strip.
4. Follow another link. Both visits appear under the first page.
5. Close the browser, then start it again with the same data folder. The tree and the current page are kept.

Clicking an older visit reloads its URL. Resuming does not restore the scroll position or forms.

On a page's card, **New branch here** opens the palette from that page without loading it, so the next address you type becomes its child. A tree's tab reopens the page you were last reading in that tree.

## Resume reading

1. From a comment or an article, open several links with **Ctrl+click**, a **middle click** or a link meant for a new window. The destinations wait in the tree, drawn dashed. The source page stays open and the destinations are not downloaded.
2. Click a waiting page to read it. The visit keeps its parent and the link's context when it is available.
3. Let **Remember pages** save the text of the open page, or select a passage and press **Ctrl+S**. These live in the **Memory** menu, behind the dot at the end of the current page in the strip. **Remember selection** in that menu does the same as Ctrl+S.
4. Close the browser, then start it again with the same data folder.
5. Search a few words of the saved text with **Ctrl+K**. Open a **Saved text** result to read the passage offline, or go back to the source to find the links that are still waiting.

Results cover titles, URLs, saved text and source context. Saved text carries its date. Reading saved text and loading the live page are two separate actions. On the current page's card, **Back to source** and **Open comment** lead back to where the page was found.

Turn off **Remember pages** in the Memory menu to stop automatic capture. Remembering a selection stays available. **Read saved text** shows the text saved for the current page. The Memory menu also lists recent errors, which otherwise appear briefly under the tab row.

Form fields and editable areas are left out of saved text. Visible password forms block automatic capture.

Recovering the comment or permalink depends on the site's DOM. When that context is missing, the parent page is still kept. The imported Chrome corpus stays separate from live browsing.

For a post that appears later in a dynamic feed, select the passage and press **Ctrl+S**. Automatic capture does not keep saving everything that appears while you scroll.

Without `--data-dir`, Canopy keeps its database and WebKit profile in `$XDG_DATA_HOME/canopy`, or in `~/.local/share/canopy`. Canopy starts dark. The gear at the end of the tab row opens **Settings**, where **Theme** offers **Dark**, **Light** or **System**, and **Remember pages** turns automatic capture on or off. The theme persists in the `theme_mode` session setting and applies immediately. With **System**, it follows the desktop's dark preference.

## Download files

A link to a file Canopy cannot show, such as an archive or a file the site marks as an attachment, is saved instead of opened. **Download Linked File** and **Save Image As** in the page's context menu save too. Canopy does not ask where to save. Files go to your download folder, the XDG download directory or `~/Downloads`, which Canopy creates when it is missing.

Canopy keeps only the last part of the name the site suggests, so a file always lands directly in that folder. It never replaces a file. A second `report.pdf` is saved as `report (1).pdf`. While a download runs, its data sits in a hidden `.canopy-download-…` folder there, and the file appears under its name once it is complete.

A message under the tab row says when a download starts, when it is saved, with its file name, and when it fails, with the reason. A failed download leaves no partial file behind. A download adds no page to your forest and keeps the page you are reading.

A saved file stays attached to the page it was downloaded from. In the tree strip it shows as a small green leaf on top of that page's node. A page with up to three files shows one leaf for each. A page with more shows one leaf and their count. At rest only the current page and its ancestors carry leaves. Hover the strip to see them on side branches too.

Click a leaf to list that page's files, with their size and when they were saved. Click a file to open it with your desktop's app for that kind of file. The folder button beside it opens the folder it was saved in.

Canopy records where the file was saved and leaves the file there. If you move or delete it, its leaf turns hollow and the list says **Moved or deleted**. Canopy notices this the next time it redraws the strip or when you try to open the file. A file whose address you type, or that another app hands to Canopy, attaches to the page on screen. A file saved with no page open is not attached to any page.

Canopy has no list of all downloads yet, and it cannot pause or resume one.

## Check the project

Check the formatting, the core tests and the compiler diagnostics:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The GUI tests load locally served pages and drive the window with real pointer and keyboard input. They check the HTML chrome through its own DOM. Install Xvfb and xdotool, then run them like this. Unsetting `WAYLAND_DISPLAY` keeps the window on Xvfb from a Wayland session:

```sh
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test webkit_smoke
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test reading_smoke
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test download_smoke
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test open_url_smoke
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test shortcut_smoke
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test sandbox_smoke
```

`open_url_smoke` starts the `canopy` binary on temporary data folders and launches it again with addresses, so it checks the hand-off between real processes. It needs no session bus.

The download test points `HOME`, `XDG_CONFIG_HOME` and `XDG_DATA_HOME` at a temporary folder, so it never writes to your real download folder and opens files with a stand-in app instead of yours. Set `CANOPY_DOWNLOAD_SHOTS` to a folder to keep screenshots of the leaves and their list.

The design choices are described in [A forest of visits](docs/design.md) and [The memory of an exploration](docs/reading-memory.md).
