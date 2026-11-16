# Canopy Mac prototype

A throwaway app that answers one question: can Canopy's window work on macOS the way it works on
Linux? It is not Canopy. It has no trees and no store. It only has the parts that are tied to the
platform:

- a transparent HTML chrome drawn over the web pages
- several live pages in one window, one shown at a time
- a page that loads in the background before it is ever shown
- saving a page's history, destroying the page, and rebuilding it with its back list and scroll
- memory per live page
- video with sound

## Try it on a Mac

You need Rust (`brew install rust`) and Apple's command line tools (`xcode-select --install`).

```sh
cargo install --git https://github.com/Binoui/canopy --branch proto/mac canopy-mac-proto
canopy-mac-proto
```

What to try:

1. **A, B, C** are three page slots. A green dot means the page is live, amber means it was
   dropped and its history saved, grey means empty. B loads Wikipedia in the background at start.
2. Follow a few links in A, scroll down, type in the field, then press **Save + revive**. The page
   disappears and comes back. Check the address, the scroll position, and that **Back** still works.
3. **Drop** destroys the page and keeps its history. Click its tab to bring it back.
4. **Overlay** covers the window with the chrome. The page must stay visible around the card.
   Click outside the card to close it.
5. **Video** plays a test clip with a tone. Switch to another slot: does the sound go on?
6. **Memory** lists the memory of each live page.
7. Type an address and press Enter to load any site. Resize the window. Play a real video.

## Scripted run

`canopy-mac-proto --self-test out` runs the same questions without a person and writes
`out/report.md` and screenshots. GitHub runs it on a Mac machine for every push to this branch
(workflow `mac-proto`).
