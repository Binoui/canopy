# Ads and trackers

Canopy blocks matching requests and hides matching elements by default. The
global switch applies to every open page. The site switch applies to the shown
page's origin in every tree and reloads the shown page once.

## Sub-features

- First-list preparation. Pages load while Canopy prepares its first lists.
- Global switch. Later requests on open pages use the new setting without a reload.
- Site exception. The top page's scheme, host, and port determine its policy.
- Warm restart. The first load uses stored filters, and both switches keep their settings.

## How to get to it (user POV)

Open the gear at the end of the tab row. **Settings** contains **Block ads and
trackers**. With blocking on and a web page shown, **This page** contains **Block
on this site**. Use the pointer or move to a switch with the arrow keys and
press Space or Enter.

## Driving it with canopy-verify

Preconditions: Set `V=.claude/skills/verify-canopy/scripts/canopy-verify`. Use a
loopback server that serves a list with an `[Adblock Plus 2.0]` header, a rule
`/ads/*$script`, and a rule `##.adbox`. Its fixture page should contain an
`.adbox` element and load a script from `/ads/`. Check the server's request log
for the script.

1. Export `CANOPY_BLOCKLISTS` with the loopback list URL. Run
   `$V launch --seed empty`, then `$V doctor`. If the list response is held open,
   inspect the **Preparing lists…** detail in Settings before releasing it.
2. Open the fixture page with `$V open`. Check that the ad element is hidden and
   the script is absent from the server's log.
3. Turn off **Block ads and trackers**. Trigger a new ad script request on the
   fixture page and check that it reaches the server. Check that the main page
   did not reload. Turn blocking on with the keyboard.
4. Turn off **Block on this site**. Check for one page reload, the visible ad
   element, and a script request. Open the same page through another loopback
   hostname or port and confirm that its ad remains blocked.
5. Use `$V quit` and `$V start`, then open Settings. Confirm the saved site
   exception. Switch to Light and use `$V resize` and `$V shot` to check both menu
   rows at 1240 and 720 px. Repeat in Dark. Run `$V cleanup`.

## Gotchas

- Use the same exported list value across restart commands.
- With the global switch off, already hidden elements keep their style until the page loads again.
- A page that redirects itself to an excepted origin can retain hidden elements
  in that document. Reload to clear them.
- `blocking_smoke` checks early requests, background pages, errands, live switching,
  cached Back, local refresh failures, and recovery from unreadable stored filters.
