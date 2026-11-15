# Themes an agent can write

A theme restyles Canopy's chrome: the tabs, the strip, the palette, the cards, the menus, the start page, the forest view and the reader. It never touches the pages themselves. A theme is one JSON file, and an agent writes it with two tools: one reads a guide, the other applies the theme and returns a picture of the result.

The design follows T3 Code's themes, which work the same way. Agents write those well because the contract is small, written down and enforced: two colors give a full palette, named roles override single colors, CSS is an optional extra, and only a short list of selectors is promised to stay stable.

## A theme is one JSON file

```json
{
  "version": 1,
  "name": "Canopy",
  "appearance": "dark",
  "canvas": "#0f0f0f",
  "accent": "#3fcf7d",
  "colors": { "tile-border": "#2d2d2d" },
  "variants": { "light": { "canvas": "#ffffff", "accent": "#2f9e5f" } },
  "css": ""
}
```

- `name` and `appearance` are required. `appearance` is the mode the base colors are for, `dark` or `light`.
- `canvas` and `accent` are hex colors. The full palette comes from them (next section).
- `colors` overrides single roles by name. An unknown role name is an error, not a silent no-op.
- `variants.light` or `variants.dark` holds `canvas`, `accent` and `colors` for the other appearance. Without a variant the other appearance is derived from the same two colors.
- `css` is optional. Its rules are in "Custom CSS is a constrained extra" below.
- `version` is the format version. A file with a version Canopy does not know is refused with a message saying so.

Published themes live in `XDG_DATA_HOME/canopy/themes/<id>.json`. The id is the file name: lowercase letters, digits and dashes. Publishing under an existing id replaces that theme. The built-in themes, `canopy` (today's look) and later others, are compiled in and cannot be overwritten.

## Two colors give the whole palette

The roles are today's tokens in `src/chrome/assets/chrome.css`, renamed so that none names a color:

| Role | Today | Used for |
|---|---|---|
| `bg` | `--bg` | page surfaces, active tab, popovers |
| `chrome` | `--chrome` | tab row, badges |
| `border` | `--border` | hairlines between areas |
| `text` | `--text` | main text |
| `muted` | `--muted` | secondary text, idle icons |
| `faint` | `--faint` | disabled text, hints, hollow marks |
| `hover` | `--hover` | hover wash |
| `tile` | `--tile` | node and shortcut tiles |
| `tile-border` | `--tile-border` | tile outlines |
| `line` | `--line` | tree edges, switch track |
| `accent` | `--green` | the current path, focus, on states |
| `accent-soft` | `--green-soft` | pill ring, tags, selection |
| `accent-ink` | `--green-ink` | text on `accent-soft` |
| `scrim` | `--scrim` | behind the palette and full surfaces |
| `shadow` | `--shadow` | popover and card shadows |
| `danger` | `--danger` | errors, failed loads |

`src/chrome/theme.rs` derives every role from `canvas` and `accent` in OKLCH, where equal steps in lightness look equal. The canvas lightness decides the direction of the steps (lighter surfaces on a dark canvas, darker ones on a light canvas). Neutral roles keep the canvas hue at a low chroma, so a warm canvas gives warm grays. Accent roles keep the accent hue. `colors` is applied after the derivation.

Two tests hold the derivation to the current look:

- Deriving from `#0f0f0f` and `#3fcf7d`, plus the overrides in the built-in `canopy` theme, gives exactly today's dark tokens. The same goes for the light tokens and their variant.
- Every resolved theme passes contrast checks: `text` and `muted` on `bg` and `chrome`, and `accent-ink` on `accent-soft`. A theme that fails is still applied but comes back with warnings naming the roles, so an agent can fix them.

## Tokens and slots are the public surface

A theme can rely on two things: the role variables, and `data-slot` attributes on the parts of the chrome worth styling. Class names and ids stay private and can change in any release.

The first set of slots: `tab-row`, `tab`, `count`, `search-pill`, `strip`, `node`, `pill`, `bud`, `leaves`, `popover`, `menu-row`, `card`, `tag`, `palette`, `palette-input`, `palette-item`, `start`, `shortcut-tile`, `forest`, `forest-node`, `reader`, `toast`. States are attributes on the same elements (`data-state="current|pending|settled"`, `data-on`), so a theme never has to know the classes `.on`, `.pending` or `.settled`.

A test reads every `data-slot` in `index.html` and `chrome.js` and compares the list with the guide. Adding, renaming or removing a slot fails the test until the guide says the same thing. This is what keeps the guide true.

## Custom CSS is a constrained extra

The `css` field goes into a `<style id="theme">` placed after `chrome.css`, so it wins over the chrome's own rules at equal specificity. Its limits:

- At most 50,000 characters.
- Nothing from outside: no `@import`, and `url()` only with `data:` or `#id`.
- It reaches the chrome view only. Pages never load it.

Two layers enforce the "nothing from outside" rule. Every `canopy://` response gets a Content-Security-Policy: `default-src 'none'; script-src canopy:; style-src canopy: 'unsafe-inline'; font-src canopy: data:; img-src canopy: data:`. The policy fails silently, though, so `theme.rs` also scans the CSS before applying it and refuses `@import` and remote `url()` with a message saying which line. An agent learns why its rule did nothing instead of guessing.

A broken rule cannot break the chrome: the role variables carry the look, and CSS only adjusts it.

## Rust owns the active theme

The `session` row gets `theme_id` next to the existing `theme_mode`. `theme_mode` keeps meaning dark, light or system, and picks between a theme's base colors and its variant.

Rust resolves the theme into two complete token sets, one per appearance, plus the CSS. It sends them to the chrome once with a `setTheme` view call, not in every frame, because frames are frequent and themes rarely change. `chrome.js` writes the tokens as custom properties on `:root` and `:root.dark` and replaces the contents of `style#theme`. The existing `applyTheme` keeps toggling `.dark`, so following the desktop's dark mode needs no change.

The Theme section of the settings menu lists the built-in and published themes next to the Dark, Light and System switch. Picking one sends a `theme` intent with an id, which Rust saves and applies.

## The agent gets two tools and a picture

Both tools reach the running Canopy through `instance.sock`, the same way a second launch hands its request over today. `canopy theme read` and `canopy theme set` are command-line forms; a small MCP server wraps the same two calls for agents.

`theme_read` returns:

- the guide: every role with its current resolved value in both appearances, every slot and state, the CSS rules above, and how to undo;
- the list of built-in and published themes, and which one is active;
- the active theme's file, so an agent edits from what is there.

`theme_set` takes an `id` and optionally a theme. With a theme it validates it, saves it under the id and switches to it. Without one it switches to an existing theme. It returns:

- `ok`, or the reasons the theme was refused;
- warnings: contrast failures, CSS that was refused, overrides that equal the derived value;
- two screenshots of the chrome in the new theme, one per appearance.

The screenshots are the part T3 Code lacks: its agents are told they cannot see the result and must ask. Canopy renders a hidden chrome view from the fixture frames in `src/chrome/testdata` (the strip open with a current pill, a pending bud, a card and the palette), then takes a WebKit snapshot of it. The fixtures keep the picture the same from one call to the next, and keep the user's real browsing out of it.

## Undo is a switch

Built-in themes are always there, so `theme_set` with id `canopy` is always a way back. Canopy also remembers the previous theme id, so the settings menu offers "Previous theme" after each change. A published theme that fails to load (unreadable file, unknown version) falls back to `canopy` and the settings menu says which file failed.

## Out of scope for the first version

- Restyling page content (a dark reader mode is a separate feature).
- Different themes per tree or per window.
- Importing themes from a URL. A theme is a local file; sharing means sending the file.
- Fonts other than the bundled Inter. `data:` fonts fit the CSP but not the 50,000-character limit.

## Order of work

1. Rename the tokens to roles, add the `data-slot` and `data-state` attributes, and write the guide with its slot test. The look does not change.
2. Theme files, OKLCH derivation in `theme.rs`, and the round-trip and contrast tests.
3. The apply path: `setTheme`, `style#theme`, the CSP header, `theme_id` in `session`, and the theme list in the settings menu.
4. `theme_read` and `theme_set` over `instance.sock`, the CLI and the MCP wrapper.
5. Fixture rendering and the WebKit snapshot returned by `theme_set`.

Each step merges on its own. Steps 1 and 2 are pure refactors with tests; nothing an agent can call exists before step 4.
