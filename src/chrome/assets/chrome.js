// Canopy's chrome view. Rust sends state with `canopy.present` and friends; this script
// paints it and reports input back with `post`. It decides nothing about the forest: layouts,
// palette results and intents all come from Rust.
'use strict';

const HEADER = 42, REST = 52, TRUNK_Y = 26, ACCESSORY = 22;
const MOVE = 300;
/// A page with more files than this shows one leaf and their count.
const LEAVES = 3;
const MONOGRAM = ['#d95c42', '#3b7ad6', '#2e9e5e', '#9c6bde', '#d9962e', '#3b99ad', '#d1547a', '#6b7380'];
const ICONS = {
  read: '<path d="M4 10l8-5 8 5v9H4z" stroke-width="1.7"/><path d="M4 10l8 5.5 8-5.5" stroke-width="1.7"/>',
  mail: '<path d="M4 6h16v12H4z" stroke-width="1.7"/><path d="M4 7l8 6 8-6" stroke-width="1.7"/>',
  bookmark: '<path d="M6 3h12v18l-6-4-6 4z" stroke-width="1.7"/>',
  expand: '<path d="M9 4H4v5M15 4h5v5M4 15v5h5M20 15v5h-5" stroke-width="1.7"/>',
  collapse: '<path d="M4 9h5V4M20 9h-5V4M9 20v-5H4M15 20v-5h5" stroke-width="1.7"/>',
  sound: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z" fill="currentColor" stroke-width="1.6"/><path d="M15.5 9a4.2 4.2 0 0 1 0 6M18.3 6.5a8 8 0 0 1 0 11" stroke-width="1.9"/>',
  pinned: '<path d="M9 3h6l-1 7 4 4v2H6v-2l4-4zM12 16v6" stroke-width="1.6"/>',
  plus: '<path d="M12 5v14M5 12h14" stroke-width="1.9"/>',
  search: '<circle cx="11" cy="11" r="6.5" stroke-width="1.9"/><path d="M20 20l-4.2-4.2" stroke-width="1.9"/>',
  spark: '<path d="M11 2.5l1.9 5.6 5.6 1.9-5.6 1.9L11 17.5l-1.9-5.6L3.5 10l5.6-1.9z" fill="currentColor" stroke="none"/><path d="M18.5 14l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9z" fill="currentColor" stroke="none" opacity=".7"/>',
  forest: '<circle cx="4.5" cy="12" r="2" stroke-width="1.8"/><circle cx="19.5" cy="5.5" r="2" stroke-width="1.8"/><circle cx="19.5" cy="12" r="2" stroke-width="1.8"/><circle cx="19.5" cy="18.5" r="2" stroke-width="1.8"/><path d="M6.5 12h11M10 12c2.5 0 3-6.5 7.5-6.5M10 12c2.5 0 3 6.5 7.5 6.5" stroke-width="1.8"/>',
  chevron: '<path d="M7 10l5 5 5-5" stroke-width="2"/>',
  back: '<path d="M19 12H5M11 6l-6 6 6 6" stroke-width="1.9"/>',
  forward: '<path d="M5 12h14M13 6l6 6-6 6" stroke-width="1.9"/>',
  globe: '<circle cx="12" cy="12" r="8.5" stroke-width="1.7"/><path d="M3.5 12h17M12 3.5c2.6 2.6 2.6 14.4 0 17M12 3.5c-2.6 2.6-2.6 14.4 0 17" stroke-width="1.7"/>',
  close: '<path d="M6 6l12 12M18 6L6 18" stroke-width="1.9"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" stroke-width="1.7"/><path d="M14 3v5h5" stroke-width="1.7"/>',
  key: '<circle cx="7.5" cy="15.5" r="5.5" stroke-width="1.7"/><path d="M21 2l-9.6 9.6M15.5 7.5l3 3L22 7l-3-3" stroke-width="1.7"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" stroke-width="1.7"/>',
  settings: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" stroke-width="1.6"/><circle cx="12" cy="12" r="3" stroke-width="1.6"/>',
};

const $ = (id) => document.getElementById(id);
const post = (message) => window.ipc.postMessage(JSON.stringify(message));
/// The old GTK buttons did not take focus from the page.
const NAVIGATES = new Set(['reload', 'back', 'forward', 'stop', 'restore', 'settle-current', 'open-suggestion', 'open-bookmark', 'open-mail', 'open-inbox', 'open-citation', 'open-saved', 'branch-queued', 'fill-login', 'save-login']);
const intent = (kind, extra = {}) => {
  post({ t: 'intent', kind, ...extra });
  if (NAVIGATES.has(kind)) post({ t: 'focus', target: 'page' });
};
const esc = (text) => String(text ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const icon = (name) => `<svg class="icon" viewBox="0 0 24 24">${ICONS[name]}</svg>`;
const ease = (t) => 1 - Math.pow(1 - Math.min(Math.max(t, 0), 1), 3);
const mix = (a, b, t) => a + (b - a) * t;
const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);

const S = {
  frame: null,
  keyboard: null,
  pageScale: null,
  loading: null,
  logins: null,
  passwordSettings: null,
  save: null,
  locked: null,
  strip: null,
  revisions: new Map(),
  systemDark: null,
  surface: 'reading',
  overlay: null,
  formReturn: false,
  treeSide: 'tree',
  treePreference: 'tree',
  layer: 'none',
  hovered: null,
  list: null,
  popover: null,
  switcher: null,
  mailTab: 'all',
  mailIconRevision: 0,
  openMailSenders: new Set(),
  palette: null,
  reader: null,
  find: null,
  errors: { recent: [], toast: null, unread: false },
  forest: null,
  forestWants: 'canopy',
  reported: '',
};

const shortcut = (id, style = 'text') => S.keyboard?.labels[id]?.[style] ?? '';
const keyTitle = (text, key) => key ? `${text} · ${key}` : text;
const shortcutTitle = (text, id) => keyTitle(text, shortcut(id));
const kbd = (key) => key ? `<kbd>${esc(key)}</kbd>` : '';
const shortcutChip = (id, style = 'chip') => kbd(shortcut(id, style));
const stripTip = (tip) => tip ? shortcutTitle(tip.text, tip.command) : null;
const undoShortcut = () => {
  const label = shortcut('undo-settle');
  return label ? ` (${label})` : '';
};

let inputOrigin = 'keyboard';
document.addEventListener('mousedown', () => {
  inputOrigin = 'pointer';
  if (S.list) S.list.origin = 'pointer';
}, true);
document.addEventListener('keydown', () => {
  inputOrigin = 'keyboard';
  if (S.list) S.list.origin = 'keyboard';
}, true);
const focusReturns = new Map();
function rememberFocus(kind, trigger = document.hasFocus() ? document.activeElement : document.body) {
  focusReturns.set(kind, { trigger, id: trigger?.id, data: trigger?.dataset.id, start: trigger?.dataset.start });
}
function returnFocus(kind) {
  const saved = focusReturns.get(kind);
  focusReturns.delete(kind);
  const trigger = saved?.trigger?.isConnected ? saved.trigger
    : saved?.id ? $(saved.id)
    : saved?.data ? [...document.querySelectorAll('[data-id]')].find((el) => el.dataset.id === saved.data && el.dataset.start === saved.start && el.getClientRects().length) : null;
  if (canFocus(trigger)) {
    post({ t: 'focus', target: 'chrome' });
    trigger.focus();
  } else if (pageShows()) post({ t: 'focus', target: 'page' });
  else if (S.surface === 'start') focusStart();
}
// WebKitGTK reports the reverse tab key as Unidentified on X11.
const tabKey = (event) => ['Tab', 'ISO_Left_Tab'].includes(event.key) || event.code === 'Tab' || event.keyCode === 9;
const controlsIn = (root) => [...root.querySelectorAll('button:not(:disabled), input, [tabindex="0"]')].filter((el) => el.tabIndex >= 0 && el.getClientRects().length);

const visit = (id) => S.frame?.visits[id] ?? S.forest?.canopy?.visits?.[id];
const current = () => S.frame?.current ?? null;
/// The files downloaded from a page of the current tree, oldest first.
const filesOf = (id) => S.frame?.files?.[id] ?? [];

// ---------- favicons ----------

function iconSite(url) {
  try { return new URL(url).hostname; } catch { return url; }
}

function hostOf(url) {
  try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return '?'; }
}

function monogramColor(name) {
  let hash = 2166136261;
  for (let i = 0; i < name.length; i++) hash = Math.imul(hash ^ (name.charCodeAt(i) & 255), 16777619) >>> 0;
  return MONOGRAM[hash % MONOGRAM.length];
}

/// A page icon from Rust's cache, over the host's monogram, which shows until (or unless)
/// the icon loads.
function fav(url, opened, size) {
  const host = hostOf(url);
  const color = monogramColor(host);
  const letter = esc((host[0] || '?').toUpperCase());
  const src = `icon?u=${encodeURIComponent(url)}&o=${opened ? 1 : 0}&r=${S.revisions.get(iconSite(url)) ?? 0}`;
  return `<span class="fav" style="width:${size}px;height:${size}px;background:${color};border-radius:${Math.min(3, size / 4)}px">`
    + `<b aria-hidden="true" style="font-size:${Math.round(size * 0.62)}px">${letter}</b>`
    + `<img src="${src}" alt="" onload="this.style.visibility='';this.parentNode.style.background='none';this.previousSibling.style.visibility='hidden'" onerror="this.style.visibility='hidden'"></span>`;
}
const waitState = (v) => v.queued ? (v.loaded ? 'loaded' : 'unloaded') : null;
const markClass = (v) => v.dismissed ? ' dismissed' : waitState(v) === 'loaded' ? ' pending loaded' : waitState(v) ? ' pending' : '';
const visitFav = (v, size) => fav(v.url, waitState(v) !== 'unloaded', size);
/// A page playing sound wears a speaker on its icon wherever the icon is drawn.
const playingMark = (on) => on ? `<i class="playing-mark" aria-hidden="true">${icon('sound')}</i>` : '';
const playingFav = (html, on) => on ? `<span class="mk">${html}${playingMark(true)}</span>` : html;
const markedFav = (v, size, live = v.live) => `<span class="mk">${visitFav(v, size)}`
  + (live ? '<i class="live-dot"></i>' : '') + playingMark(v.playing)
  + (waitState(v) ? `<i class="pending-dot${waitState(v) === 'loaded' ? ' loaded' : ''}"></i>` : '') + '</span>';

// ---------- timers and layers, as the GTK chrome's presentation state machine ----------

const timers = {};
function schedule(name, ms, run) {
  clearTimeout(timers[name]);
  timers[name] = setTimeout(() => { delete timers[name]; run(); }, ms);
}
function cancel(name) {
  clearTimeout(timers[name]);
  delete timers[name];
}

function enterNode(key) {
  if (nativeDrag || S.layer !== 'none' || S.overlay) return;
  S.hovered = key;
  cancel('hideList');
  schedule('showList', 250, () => { if (S.hovered === key) showList(key, 'pointer'); });
}
function listHasKeyboardFocus() {
  return S.list?.origin === 'keyboard' && ($('strip-list').contains(document.activeElement)
    || strip.elements.get(S.list.key) === document.activeElement);
}
function leaveNode(event) {
  if (nativeDrag) return;
  S.hovered = null;
  cancel('showList');
  if (event.relatedTarget?.closest('#strip-list')) return;
  if (S.list) schedule('hideList', 220, () => { if (!listHasKeyboardFocus()) hideList(); });
}
function setLayer(layer) {
  S.layer = layer;
  if (layer !== 'none' && !S.popover?.anchor.closest('#strip-list')) hideList();
  syncLayer();
}

/// Tells Rust which layer the chrome can close and how far down its view must reach.
/// Below the view, the page gets the pointer.
function syncLayer() {
  const surfaces = { formReturn: !!S.formReturn, reader: !!S.reader, palette: !!S.palette, popover: !!S.popover, list: !!S.list,
    treeFilters: S.overlay === 'tree' && !!(S.frame.treeView.query || S.frame.treeView.waitingOnly),
    overlay: !!S.overlay, start: S.surface !== 'reading' && !!current() };
  let height = HEADER + REST;
  if (S.formReturn || S.palette || S.reader || S.overlay || S.switcher || S.surface !== 'reading') {
    height = null;
  } else {
    for (const el of [S.list && $('strip-list'), S.popover && $('popover')]) {
      if (!el) continue;
      const bottom = el.hidden ? HEADER + REST + 420 : el.getBoundingClientRect().bottom;
      height = Math.max(height, bottom + 30);
    }
    height = Math.ceil(height);
  }
  const toastRects = $('toast').hidden ? [] : [...document.querySelectorAll('#toast:not([hidden]) button')].map((button) => {
    const r = button.getBoundingClientRect();
    return [Math.floor(r.x), Math.floor(r.y), Math.ceil(r.width), Math.ceil(r.height)];
  });
  const report = `${height}/${JSON.stringify(surfaces)}/${JSON.stringify(toastRects)}`;
  if (report !== S.reported) {
    S.reported = report;
    post({ t: 'layer', height, surfaces, toastRects });
  }
}

// ---------- theme ----------

function applyTheme() {
  const mode = S.frame?.theme ?? 'dark';
  const system = S.systemDark ?? matchMedia('(prefers-color-scheme: dark)').matches;
  document.documentElement.classList.toggle('dark', mode === 'dark' || (mode === 'system' && system));
}

// ---------- tab row ----------

function patchChildren(container, html) {
  const template = document.createElement('template');
  template.innerHTML = html;
  patchInto(container, template.content);
}
const keyOf = el => el.dataset.key || el.dataset.id;
const allKeyed = el => el.childNodes.length > 0 && [...el.childNodes].every(n => n.nodeType === Node.ELEMENT_NODE ? !!keyOf(n) : !n.textContent.trim());
function patchInto(container, fresh) {
  const previous = [...container.children], used = new Set();
  const kept = new Map(previous.filter(el => keyOf(el)).map(el => [keyOf(el), el]));
  let cursor = container.firstElementChild;
  for (const next of [...fresh.children]) {
    const key = keyOf(next), el = (key && kept.get(key)) || next;
    if (key) kept.delete(key);
    used.add(el);
    if (el !== next) {
      for (const attr of next.attributes) if (el.getAttribute(attr.name) !== attr.value) el.setAttribute(attr.name, attr.value);
      for (const attr of [...el.attributes]) if (!next.hasAttribute(attr.name)) el.removeAttribute(attr.name);
      if (allKeyed(el) && allKeyed(next)) patchInto(el, next);
      else if (el.innerHTML !== next.innerHTML) el.innerHTML = next.innerHTML;
    }
    if (el !== cursor) container.insertBefore(el, cursor);
    cursor = el.nextElementSibling;
  }
  for (const el of previous) if (!used.has(el)) el.remove();
}
function treeAct(tree, act) {
  if (S.overlay === 'forest') { post({t:'forest',do:'act',tree,act}); return; }
  const navigates = act === 'open';
  const lands = ['save','settle','delete'].includes(act) && S.overlay !== 'forest' && S.surface !== 'start';
  if (navigates) uncover();
  post({t:'tree', tree, act});
  if (navigates || lands) post({t:'focus', target:'page'});
}

function renderTabs() {
  const start = S.surface === 'start';
  $('new-tree-tab').hidden = !start;
  $('new-tree-tab').setAttribute('aria-selected', String(start));
  // A new tree that is still loading shows its site on the tab it will replace.
  const opening = S.frame?.newTree?.title;
  $('new-tree-tab').querySelector('.tab-title').textContent = opening ?? 'New tree';
  $('new-tree-tab').setAttribute('aria-label', opening ? `${opening}, loading` : 'New tree');
  $('new-tree-close').hidden = !current();
  const row = $('tab-row');
  const focusedTab = document.hasFocus() && row.contains(document.activeElement) ? document.activeElement.closest('.tab')?.dataset.id : null;
  const trees = S.frame?.tabs ?? [];
  const kept = new Map([...row.children].map((tab) => [tab.dataset.id, tab]));
  const nodes = trees.map((tree, index) => {
    const active = tree.active && !start;
    const sub = active ? tree.subtitle : null;
    const root = visit(tree.id);
    const pages = `${tree.pages} page${tree.pages === 1 ? '' : 's'}`;
    const pending = tree.pending ? `, ${tree.pending} waiting` : '';
    const pinned = tree.pinned ? ', pinned' : '';
    const playing = tree.playing ? ', playing sound' : '';
    const key = S.keyboard?.digitLabels[index];
    const html = `<button class="tab${tree.pinned ? ' pinned' : ''}${tree.playing ? ' playing' : ''}" draggable="true" role="tab" tabindex="${active ? 0 : -1}" aria-selected="${active}" data-id="${tree.id}" title="${esc(keyTitle(`${tree.title}${sub ? ` · ${sub}` : ''} · ${pages}${pending}${pinned}${playing}`, key))}" aria-label="${esc((sub ? `${tree.name}, on ${sub}` : tree.name) + pending + playing)}">`
      + (root ? `<span class="mk">${visitFav(root, 14)}${playingMark(tree.playing)}</span>` : '')
      + `<span class="tab-title${sub ? ' two-line' : ''}"><span class="tab-name">${esc(tree.title)}</span>${sub ? `<span class="tab-sub">${esc(sub)}</span>` : ''}</span>`
      + (tree.pinned ? `<span class="tab-pinned-mark" aria-hidden="true">${icon('pinned')}</span>` : '')
      + (tree.pending ? '<i class="unread-dot" aria-hidden="true"></i>' : '')
      + `<span class="tab-settle" data-settle="${tree.id}" title="${esc(active ? shortcutTitle('Settle', 'settle-tree') : 'Settle')}" aria-hidden="true">${icon('close')}</span>`
      + '</button>';
    const template = document.createElement('template');
    template.innerHTML = html;
    const fresh = template.content.firstChild;
    const tab = kept.get(tree.id) ?? fresh;
    if (tab !== fresh) {
      tab.classList.toggle('pinned', tree.pinned);
      tab.classList.toggle('playing', !!tree.playing);
      tab.setAttribute('aria-selected', String(active));
      setTooltip(tab, fresh.title);
      tab.setAttribute('aria-label', fresh.getAttribute('aria-label'));
      const settle = tab.querySelector('[data-settle]');
      const freshSettle = fresh.querySelector('[data-settle]');
      settle.title = freshSettle.title;
      freshSettle.replaceWith(settle);
      tab.replaceChildren(...fresh.childNodes);
    }
    return tab;
  });
  [...row.children].filter((tab) => !nodes.includes(tab)).forEach((tab) => tab.remove());
  nodes.forEach((tab, index) => { if (row.children[index] !== tab) row.insertBefore(tab, row.children[index] ?? null); });
  if (S.frame?.tabFit) window.canopy.tabsFit(S.frame.tabFit);
  fitTabs();
  if (focusedTab) nodes.find(tab => tab.dataset.id === focusedTab && !tab.hidden)?.focus({preventScroll:true});
  const focused = row.contains(document.activeElement) && !document.activeElement.hidden ? document.activeElement : row.querySelector('[aria-selected="true"]') ?? row.querySelector('.tab:not([hidden])');
  for (const tab of row.children) tab.tabIndex = tab === focused ? 0 : -1;
}

function fitTabs() {
  const header = $('tabs');
  const fixed = [...header.children].filter((el) => !['tab-row', 'tab-overflow'].includes(el.id) && !el.classList.contains('spacer'))
    .reduce((sum, el) => sum + (el.hidden ? 0 : el.getBoundingClientRect().width + parseFloat(getComputedStyle(el).marginLeft || 0) + parseFloat(getComputedStyle(el).marginRight || 0) + 2), 0);
  post({ t: 'tab-room', room: Math.max(0, header.clientWidth - 14 - fixed - 4) });
}

// ---------- strip ----------

const strip = {
  shown: { height: REST, pan: 0, nodes: new Map(), edges: new Map() },
  from: null,
  target: null,
  anim: null,
  awakeUntil: 0,
  elements: new Map(),
  badges: new Map(),
  leaves: new Map(),
};

const targetLayout = () => S.strip ?? null;

const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
// WebKit can leave CSS media styles stale after a desktop preference change.
const applyMotionPreference = () => document.documentElement.classList.toggle('reduced-motion', reducedMotion.matches);
reducedMotion.addEventListener('change', applyMotionPreference);
applyMotionPreference();

function animateStrip(duration, growingPage = null) {
  if (reducedMotion.matches) duration = 0;
  const layout = targetLayout();
  if (!layout) return;
  strip.target = layout;
  const changed = buildStrip(layout);
  applyRefocus(layout);
  retireStripNodes(layout);
  const geometry = JSON.stringify([layout.nodes.map(({id, x, y, w, s}) => [id, x, y, w, s]), layout.edges, layout.pan]);
  // Keep WebKit's rendering clock warm without repainting unchanged geometry.
  // Letting it sleep delays the next visible update.
  strip.awakeUntil = performance.now() + duration;
  if (strip.geometry === geometry) {
    if (changed && !strip.anim) drawStrip();
    if (duration && !strip.raf) strip.raf = requestAnimationFrame(tick);
    return;
  }
  strip.from = strip.shown;
  strip.geometry = geometry;
  cancelAnimationFrame(strip.raf);
  strip.raf = null;
  if (!duration) {
    strip.anim = null;
    step(performance.now(), 1);
    pruneStrip();
    syncLayer();
    return;
  }
  strip.anim = { start: performance.now(), duration, growingPage };
  if (growingPage) step(strip.anim.start, 0);
  syncLayer();
  strip.raf = requestAnimationFrame(tick);
}

function tick(now) {
  strip.raf = null;
  if (strip.anim) {
    const t = reducedMotion.matches ? 1 : (now - strip.anim.start) / strip.anim.duration;
    step(now, t);
    if (t >= 1) {
      strip.anim = null;
      pruneStrip();
      syncLayer();
    }
  }
  if (strip.anim || now < strip.awakeUntil) strip.raf = requestAnimationFrame(tick);
}

function applyRefocus(layout) {
  const transfer = S.refocus;
  if (!transfer) return;
  if (document.activeElement !== strip.elements.get(transfer.id)) {
    S.refocus = null;
    return;
  }
  if (layout.nodes.some(node => node.kind === 'tab' && node.id === transfer.id)) return;
  S.refocus = null;
  const tabs = layout.nodes.filter(node => node.kind === 'tab');
  const next = tabs[Math.min(transfer.index, tabs.length - 1)]
    ?? layout.nodes.find(node => node.kind === 'overflow')
    ?? layout.nodes.find(node => node.kind === 'pill');
  strip.elements.get(next?.id)?.focus();
}

function retireStripNodes(layout) {
  const ids = new Set(layout.trunk);
  const overflow = strip.elements.get('overflow');
  if (!ids.has('overflow')) {
    cancel('waitingOpen');
    let queue = S.popover;
    while (queue && queue.kind !== 'waiting') queue = queue.parent;
    if (queue) {
      const focused = $('popover').contains(document.activeElement);
      overflow?.setAttribute('aria-expanded', 'false');
      closePopover(false);
      if (focused) strip.elements.get(current())?.focus();
    }
  }
  for (const [id, el] of strip.elements) {
    const leaving = !ids.has(id);
    el.inert = leaving;
    if (leaving) el.classList.remove('waiting-pulse');
    const leaves = strip.leaves.get(id);
    if (leaves) leaves.inert = leaving;
  }
}

function pruneStrip() {
  for (const [id, el] of strip.elements) {
    if (!strip.target.nodes.some((n) => n.id === id)) {
      el.remove();
      strip.elements.delete(id);
      strip.badges.get(id)?.remove();
      strip.badges.delete(id);
      strip.leaves.get(id)?.remove();
      strip.leaves.delete(id);
    }
  }
}

const NODE_KINDS = {
  root: { page: true, click: 'open', enter: 'list', list: 'attached' },
  page: { page: true, click: 'open', enter: 'list', list: 'attached' },
  parent: { page: true, click: 'open', enter: 'list', list: 'attached' },
  pill: { page: true, click: 'open', enter: 'list', list: 'attached' },
  run: { page: false, click: 'list', enter: 'list', list: 'attached', entrance: true },
  earlier: { page: false, click: 'list', enter: 'list', list: 'attached', entrance: true },
  after: { page: false, click: 'list', enter: 'list', list: 'attached', entrance: true },
  tab: { page: true, click: 'open', enter: 'open', list: 'attached', dismiss: true },
  overflow: { page: false, click: 'queue', enter: 'queue', list: 'queue' },
};

function kindOf(node) {
  if (!Object.hasOwn(NODE_KINDS, node.kind)) throw new Error(`Unknown strip kind: ${node.kind}`);
  return NODE_KINDS[node.kind];
}

const nodeAt = el => strip.target?.nodes.find(n => n.id === el.dataset.id);

const NODE_ACTIONS = {
  open: node => activate(node.page),
  queue: (node, el) => openQueue(el),
  list: (node, el, keyboard) => {
    showList(node.id);
    if (keyboard) $('strip-list').querySelector('button')?.focus();
  },
};

function wireNode(el) {
  el.addEventListener('mouseenter', () => {
    const node = nodeAt(el);
    if (!node) return;
    if (kindOf(node).list === 'queue') enterQueue(el);
    else enterNode(node.id);
  });
  el.addEventListener('mouseleave', event => {
    const node = nodeAt(el);
    if (node && kindOf(node).list === 'queue') leaveQueue();
    else leaveNode(event);
  });
  el.addEventListener('focus', () => {
    const node = nodeAt(el);
    if (node && kindOf(node).list === 'attached') showList(node.id);
  });
  el.addEventListener('click', event => {
    const node = nodeAt(el);
    const pressed = crossPress;
    crossPress = null;
    if (!node) return;
    const cross = event.target.closest('[data-act="dismiss"]');
    if (cross) {
      if (cross === pressed && event.detail <= 1 && kindOf(node).dismiss) intent('dismiss', { id: node.page });
      return;
    }
    NODE_ACTIONS[kindOf(node).click](node, el, false);
  });
  el.addEventListener('auxclick', event => {
    const node = nodeAt(el);
    if (event.button === 1 && node && kindOf(node).dismiss) {
      event.preventDefault();
      intent('dismiss', { id: node.page });
    }
  });
  el.addEventListener('keydown', walkTrunk);
}

/// One element per node, kept across frames so hovers and transitions survive a re-layout.
function buildStrip(layout) {
  let changed = false;
  $('strip').style.height = `${layout.height}px`;
  $('strip-clip').style.height = `${layout.height}px`;
  $('strip-bg').style.height = `${layout.height}px`;
  for (const [key, value] of Object.entries(layout.metrics ?? {})) {
    const name = key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
    document.documentElement.style.setProperty(`--${key.endsWith('Gap') ? name : `item-${name}`}`, `${value}px`);
  }
  const nodes = $('strip-nodes');
  const canvas = $('strip-canvas');
  for (const n of layout.nodes) {
    const v = visit(n.page);
    if (!v) continue;
    let el = strip.elements.get(n.id);
    if (el) el.inert = false;
    const paint = JSON.stringify([n, v, n.ids.map(filesOf), S.list?.key === n.id]);
    if (el?.dataset.paint === paint) continue;
    changed = true;
    if (!el) {
      el = document.createElement('button');
      el.className = 'node';
      el.dataset.id = n.id;
      wireNode(el);
      nodes.appendChild(el);
      strip.elements.set(n.id, el);
    }
    el.dataset.paint = paint;
    el.style.width = `${n.w}px`;
    el.style.height = `${n.s}px`;
    el.style.transformOrigin = '0 0';
    if (kindOf(n).page) el.dataset.page = n.page;
    else delete el.dataset.page;
    el.draggable = !!el.dataset.page;
    const queue = kindOf(n).list === 'queue';
    el.className = `node ${n.kind}` + (n.divider ? ' divided' : '') + (n.named ? ' named' : '') + (n.playing ? ' playing' : '') + markClass(v) + ((n.kind === 'tab' || queue) && el.classList.contains('waiting-pulse') ? ' waiting-pulse' : '');
    el.setAttribute('aria-label', n.name);
    el.setAttribute('aria-current', n.kind === 'pill' ? 'page' : 'false');
    el.setAttribute('aria-controls', queue ? 'popover' : 'strip-list');
    if (queue) el.setAttribute('aria-haspopup', 'dialog');
    else el.removeAttribute('aria-haspopup');
    const expanded = queue
      ? S.popover?.anchor === el || S.popover?.parent?.anchor === el
      : S.list?.key === n.id;
    el.setAttribute('aria-expanded', String(expanded));
    setTooltip(el, stripTip(n.tip));
    const html = n.kind === 'tab'
      ? markedFav(v, 16) + (n.named ? `<span class="item-label">${esc(n.label)}</span><span class="x" data-act="dismiss" aria-hidden="true">${icon('close')}</span>` : '')
        + (v.live ? `<i class="live-dot${filesOf(n.page).length ? ' above-leaves' : ''}"></i>` : '')
      : n.kind === 'overflow' ? esc(n.label)
      : n.kind === 'after' ? `<span class="read-count">${n.read}</span>`
      : n.kind === 'earlier' ? `<span>${esc(n.label)}</span>` + playingMark(n.playing)
      : n.kind === 'pill' ? playingFav(visitFav(v, 16), n.playing) + `<span class="pill-text"><span class="pill-title">${esc(v.label)}</span><span class="pill-site">${esc(v.site)}</span></span>` + (v.live ? '<i class="live-dot"></i>' : '')
      : playingFav(visitFav(v, n.kind === 'root' || n.kind === 'page' ? 16 : 14), n.playing) + (n.kind === 'parent' || n.kind === 'run' || n.named ? `<span class="item-label">${esc(n.label)}</span>` : '')
        + (v.live && n.kind !== 'run' ? `<i class="live-dot${filesOf(n.page).length ? ' above-leaves' : ''}"></i>` : '');
    if (el.dataset.html !== html) {
      el.innerHTML = html;
      el.dataset.html = html;
    }
    let badge = strip.badges.get(n.id);
    if (n.badge != null) {
      if (!badge) {
        badge = document.createElement('i');
        canvas.appendChild(badge);
        strip.badges.set(n.id, badge);
      }
      badge.className = 'branch-count';
      badge.textContent = n.badge;
    } else if (badge) {
      badge.remove(); strip.badges.delete(n.id);
    }
    const files = n.ids.flatMap(filesOf);
    let leaves = strip.leaves.get(n.id);
    if (files.length) {
      if (!leaves) {
        leaves = document.createElement('button');
        leaves.dataset.id = n.id;
        leaves.addEventListener('click', (event) => {
          event.stopPropagation();
          const id = leaves.dataset.id;
          if (S.popover?.kind === 'files' && S.popover.anchor === leaves) closePopover();
          else openPopover('files', leaves, JSON.parse(leaves.dataset.ids), strip.elements.get(id));
        });
        canvas.appendChild(leaves);
        strip.leaves.set(n.id, leaves);
      }
      leaves.dataset.ids = JSON.stringify(n.ids);
      leaves.className = `leaves${n.kind === 'pill' ? ' on-pill' : ''}`;
      const few = files.length <= LEAVES;
      const html = few
        ? files.map((f) => `<i class="leaf${f.missing ? ' missing' : ''}"></i>`).join('')
        : `<i class="leaf${files.every((f) => f.missing) ? ' missing' : ''}"></i><b>${files.length}</b>`;
      if (leaves.dataset.html !== html) {
        leaves.innerHTML = html;
        leaves.dataset.html = html;
      }
      const label = files.length === 1 ? `Downloaded here: ${files[0].name}` : `${files.length} files downloaded here`;
      setTooltip(leaves, label);
      leaves.setAttribute('aria-haspopup', 'menu');
      leaves.setAttribute('aria-expanded', String(S.popover?.anchor === leaves));
      leaves.setAttribute('aria-label', `${label}, from ${v.label}`);
    } else if (leaves) {
      leaves.remove();
      strip.leaves.delete(n.id);
    }
  }
  const dot = $('memory-dot');
  const pill = strip.elements.get(layout.nodes.find((node) => node.kind === 'pill')?.id);
  if (pill && pill.nextElementSibling !== dot) pill.after(dot);
  dot.classList.toggle('on', !!S.frame?.captureEnabled);
  setTooltip(dot, S.frame?.captureEnabled ? 'Saved text · saving pages as you read' : 'Saved text · not saving');
  return changed;
}

function step(now, t) {
  const k = ease(t);
  const from = strip.from, to = strip.target;
  const nodes = new Map();
  const targets = new Map(to.nodes.map((n) => [n.id, n]));
  for (const n of to.nodes) {
    if (n.id === strip.anim?.growingPage) {
      nodes.set(n.id, { x: n.x, y: n.y, w: n.w, s: n.s, o: ease(2 * t - 1) });
      continue;
    }
    const a = from.nodes.get(n.id) || { x: n.x, y: n.y, w: 0, s: 0, o: 0 };
    nodes.set(n.id, { x: mix(a.x, n.x, k), y: mix(a.y, n.y, k), w: mix(a.w, n.w, k), s: mix(a.s, n.s, k), o: mix(a.o, 1, k) });
  }
  for (const [id, a] of from.nodes) {
    if (!targets.has(id) && strip.elements.has(id)) nodes.set(id, { ...a, o: mix(a.o, 0, k) });
  }
  const edges = new Map();
  for (const e of to.edges) {
    const key = `${e.from}>${e.to}`;
    const a = from.edges.get(key)?.c || e.c;
    const c = e.c.map((p, i) => [mix(a[i][0], p[0], k), mix(a[i][1], p[1], k)]);
    if (e.to === strip.anim?.growingPage) {
      // The strip's trunk is horizontal. Grow from the moving parent to the child's left edge.
      const parent = nodes.get(e.from);
      const start = parent.x + parent.w / 2;
      const gap = e.c[3][0] - e.c[0][0];
      const child = nodes.get(e.to);
      child.x = start + gap + child.w / 2;
      const end = start + gap * k;
      c[0][0] = c[1][0] = start;
      c[2][0] = c[3][0] = end;
    }
    edges.set(key, { kind: e.kind, to: e.to, c });
  }
  const pan = mix(from.pan, to.pan, k);
  strip.shown = { height: mix(from.height, to.height, k), pan, nodes, edges, width: to.width };
  drawStrip();
}

function drawStrip() {
  const shown = strip.shown;
  $('strip-canvas').style.transform = `translateX(${shown.pan}px)`;
  for (const [id, r] of shown.nodes) {
    const el = strip.elements.get(id);
    if (!el) continue;
    const target = strip.target.nodes.find((n) => n.id === id);
    const width = target?.w ?? r.w;
    const height = target?.s ?? r.s;
    el.style.transform = `translate(${r.x - r.w / 2}px,${r.y - r.s / 2}px) scale(${width ? r.w / width : 0},${height ? r.s / height : 0})`;
    el.style.opacity = r.o;
    el.style.visibility = r.w < 1 ? 'hidden' : '';
    const badge = strip.badges.get(id);
    if (badge) {
      badge.style.transform = `translate(${r.x + r.w / 2 + 7}px,${r.y + r.s / 2 - 8}px) translateX(-100%)`;
      badge.style.opacity = r.o;
    }
    const leaves = strip.leaves.get(id);
    if (leaves) {
      // Leaves stand on the node's top edge, clear of its rounded corner.
      leaves.style.transform = `translate(${r.x - r.w / 2 + (leaves.classList.contains('on-pill') ? 5 : 0)}px,${r.y - r.s / 2 + 1}px) translateY(-100%)`;
      leaves.style.opacity = r.o;
      leaves.style.visibility = r.w < 1 ? 'hidden' : '';
    }
  }
  const pill = strip.target?.nodes.find((n) => n.kind === 'pill');
  const r = pill && shown.nodes.get(pill.id);
  const dot = $('memory-dot');
  const glow = $('glow');
  dot.hidden = glow.hidden = !r;
  if (r) {
    dot.style.opacity = glow.style.opacity = r.o;
    dot.style.transform = `translate(${r.x + r.w / 2 - ACCESSORY}px,${r.y - 9}px)`;
    const reach = 40;
    const x = r.x - r.w / 2 + 16;
    glow.style.transform = `translate(${x - reach}px,${TRUNK_Y - reach}px)`;
  }
  const opacity = new Map([...shown.nodes].map(([id, n]) => [id, n.o]));
  const svg = $('strip-edges');
  const kept = new Set();
  for (const [key, e] of shown.edges) {
    kept.add(key);
    let path = [...svg.children].find((p) => p.dataset.key === key);
    if (!path) { path = document.createElementNS('http://www.w3.org/2000/svg', 'path'); path.dataset.key = key; svg.appendChild(path); }
    const [p0, p1, p2, p3] = e.c;
    path.setAttribute('class', e.kind);
    path.style.opacity = e.kind === 'path' ? 1 : (opacity.get(e.to) ?? 1);
    path.setAttribute('d', `M${p0} C${p1} ${p2} ${p3}`);
  }
  for (const path of [...svg.children]) if (!kept.has(path.dataset.key)) path.remove();
  positionList();
  if (S.popover?.kind === 'waiting') positionPopover(S.popover.anchor, S.popover.anchor);
}

function activate(id) {
  cancel('waitingOpen');
  cancel('waitingClose');
  uncover();
  if (id === current()) post({ t: 'focus', target: 'page' });
  else intent('restore', { id });
}

/// Left and Right walk the trunk while a node has the keyboard.
function walkTrunk(event) {
  const el = event.currentTarget;
  const node = nodeAt(el);
  if (!node) return;
  const kind = kindOf(node);
  if (event.key === 'ArrowDown' || event.key === 'Enter') {
    event.preventDefault();
    NODE_ACTIONS[event.key === 'Enter' ? kind.enter : kind.list === 'queue' ? 'queue' : 'list'](node, el, true);
    return;
  }
  if (event.key === 'Delete' && kind.dismiss) {
    event.preventDefault();
    if (event.repeat) return;
    const tabs = strip.target.nodes.filter(item => item.kind === 'tab');
    S.refocus = { id: node.id, index: tabs.indexOf(node) };
    intent('dismiss', { id: node.page });
    return;
  }
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
  const trunk = strip.target?.trunk ?? [];
  const index = trunk.indexOf(event.currentTarget.dataset.id);
  if (index < 0) return;
  const next = clamp(index + (event.key === 'ArrowLeft' ? -1 : 1), 0, trunk.length - 1);
  strip.elements.get(trunk[next])?.focus();
  event.preventDefault();
}

// ---------- attached strip list ----------

let previewSeq = 0;
const listItem = () => strip.target?.nodes.find((n) => n.id === S.list?.key);

function showList(key, origin = inputOrigin) {
  if (nativeDrag) return;
  if (S.layer !== 'none' || S.overlay || S.surface !== 'reading') return;
  cancel('hideList');
  cancel('showList');
  const item = strip.target?.nodes.find((n) => n.id === key);
  if (!item || kindOf(item).list !== 'attached') return;
  for (const [id, el] of strip.elements) if (el.getAttribute('aria-controls') === 'strip-list') el.setAttribute('aria-expanded', String(id === key));
  const previous = S.list;
  S.list = { key, origin: previous?.key === key ? previous.origin : origin, preview: previous?.key === key ? previous.preview : null };
  if (previous?.key !== key) $('strip-list').classList.remove('list-enter');
  renderList();
  if (previous?.key !== key && kindOf(item).entrance) {
    playEntrance($('strip-list'), 'list-enter');
  }
  requestListPreview();
}

function playEntrance(el, name) {
  if (!el || reducedMotion.matches) return;
  el.classList.remove(name);
  void el.offsetWidth; // Restart even when moving directly between adjacent groups.
  el.classList.add(name);
}

function requestListPreview() {
  const item = listItem();
  if (!item?.preview) return;
  previewSeq += 1;
  post({ t: 'preview', id: item.page, seq: previewSeq });
}

function hideList() {
  if (nativeDrag) return;
  cancel('showList');
  cancel('hideList');
  S.list = null;
  $('strip-list').classList.remove('list-enter');
  for (const el of strip.elements.values()) if (el.getAttribute('aria-controls') === 'strip-list') el.setAttribute('aria-expanded', 'false');
  $('strip-list').hidden = true;
  syncLayer();
}

function positionList() {
  const item = listItem();
  const r = item && strip.shown.nodes.get(item.id);
  if (!r) return;
  const list = $('strip-list');
  const width = Math.min(350, window.innerWidth - 16);
  list.style.width = `${width}px`;
  const x = r.x - r.w / 2 + strip.shown.pan + (S.strip?.left ?? 0);
  list.style.left = `${clamp(x - 2, 8, window.innerWidth - width - 8)}px`;
  list.style.top = `${HEADER + REST - 6}px`;
  list.style.maxHeight = `${Math.max(0, (S.windowHeight ?? 800) - HEADER - REST - 2)}px`;
}

function currentLoading() {
  if (S.surface === 'start') return !!S.frame?.newTree && S.loading?.page == null && S.loading?.state === 'busy';
  return S.loading?.page === current() && S.loading.state === 'busy';
}

function paintLoading() {
  const bar = $('load-progress');
  const busy = currentLoading();
  bar.hidden = !busy;
  const progress = busy ? S.loading.progress : null;
  bar.classList.toggle('waiting', busy && progress == null);
  bar.firstElementChild.style.width = `${progress == null ? 20 : Math.max(2, progress)}%`;
  if (progress == null) bar.removeAttribute('aria-valuenow');
  else bar.setAttribute('aria-valuenow', progress);
}

function renderLoadingAction() {
  if (S.list) renderList();
}

function renderList() {
  const item = listItem();
  if (!item) { hideList(); return; }
  const v = visit(item.page);
  if (!v) { hideList(); return; }
  const list = $('strip-list');
  const page = item.preview;
  const source = page && item.page === current() && v.source;
  const preview = S.list.preview;
  const parts = [
    ['head', `<div class="bl-head"${page ? ` role="group" aria-label="${esc(item.name)}" tabindex="0" data-page="${item.page}"` : ''}>${item.kind === 'earlier' ? '<span class="bl-glyph">···</span>' : markedFav(v, 16)}<div><b>${esc(item.title)}</b><span>${esc(item.meta)}${v.live ? ' · Still open' : ''}</span></div></div>`],
    ['snippet', page && preview?.snippet ? `<div class="bl-snippet">${esc(preview.snippet)}</div>` : ''],
    ['source', source ? `<div class="card-source"><div class="card-meta">${esc(source.text)}</div></div>` : ''],
    ['heading', item.heading ? `<div class="menu-heading">${esc(item.heading)}</div>` : ''],
    ...item.rows.map((row) => {
      const v = visit(row.id);
      const files = filesOf(row.id);
      const leaf = files.length ? `<button class="leaves" aria-haspopup="menu" aria-expanded="false" data-files="${row.id}" aria-label="${esc(`Files from ${v.label}`)}"><i class="leaf${files.every((f) => f.missing) ? ' missing' : ''}"></i></button>` : '';
      return [`row-${row.id}`, v ? `<div class="list-entry"><button class="menu-row${markClass(v)}" data-id="${row.id}" title="${esc(v.label)}" aria-current="${row.id === current() ? 'page' : 'false'}" aria-label="${esc(row.name)}">${markedFav(v, 16)}<span class="grow">${esc(row.label)}</span><span class="menu-detail">${esc(row.detail)}</span></button>${leaf}</div>` : ''];
    }),
    ['more', item.more ? `<button class="menu-row more" data-act="more">${icon('expand')}<span class="grow">${esc(item.more)}</span>${shortcutChip('tree-workbench')}</button>` : ''],
    ...item.actions.map((action) => {
      const stop = action.kind === 'reload' && item.page === current() && currentLoading();
      return [`action-${action.kind}`, `<button class="action bl-action" data-act="${stop ? 'stop' : action.kind}">${esc(stop ? 'Stop' : action.label)}</button>`];
    }),
  ];
  const kept = new Set();
  let cursor = list.firstElementChild;
  for (const [key, html] of parts) {
    if (!html) continue;
    kept.add(key);
    let el = [...list.children].find((el) => el.dataset.key === key);
    if (!el) {
      const template = document.createElement('template');
      template.innerHTML = html;
      el = template.content.firstElementChild;
      el.dataset.key = key;
      el.dataset.html = html;
    } else if (el.dataset.html !== html) {
      const template = document.createElement('template');
      template.innerHTML = html;
      const fresh = template.content.firstElementChild;
      const row = el.querySelector('.menu-row[data-id]');
      const newRow = fresh.querySelector('.menu-row[data-id]');
      if (row && newRow) {
        row.innerHTML = newRow.innerHTML;
        row.className = newRow.className;
        row.title = newRow.title;
        row.setAttribute('aria-label', newRow.getAttribute('aria-label'));
        row.setAttribute('aria-current', newRow.getAttribute('aria-current'));
        const leaves = el.querySelector('.leaves');
        const newLeaves = fresh.querySelector('.leaves');
        if (leaves && newLeaves) leaves.innerHTML = newLeaves.innerHTML;
        else if (newLeaves) el.appendChild(newLeaves);
        else leaves?.remove();
      } else el.innerHTML = fresh.innerHTML;
      el.className = fresh.className;
      if (fresh.dataset.act) el.dataset.act = fresh.dataset.act;
      el.dataset.html = html;
    }
    if (key === 'head') {
      if (page) { el.dataset.page = item.page; el.tabIndex = 0; el.setAttribute('role', 'group'); el.setAttribute('aria-label', item.name); }
      else { delete el.dataset.page; el.removeAttribute('tabindex'); el.removeAttribute('role'); el.removeAttribute('aria-label'); }
    }
    if (el !== cursor) list.insertBefore(el, cursor);
    cursor = el.nextElementSibling;
  }
  for (const el of [...list.children]) if (!kept.has(el.dataset.key)) el.remove();
  for (const el of list.querySelectorAll('.bl-head, .menu-row[data-id]')) el.draggable = !!(el.dataset.page || el.dataset.id);
  list.dataset.item = item.id;
  list.hidden = false;
  positionList();
  syncLayer();
}

$('strip-list').addEventListener('click', (event) => {
  const file = event.target.closest('[data-files]');
  if (file) { openPopover('files', file, file.dataset.files); return; }
  const row = event.target.closest('[data-id]');
  const action = event.target.closest('[data-act]')?.dataset.act;
  const item = listItem();
  if (!item) return;
  const v = visit(item.page);
  if (row) { activate(row.dataset.id); return; }
  if (!action) return;
  hideList();
  if (action === 'more') {
    canopy.toggleTree();
    requestAnimationFrame(() => setTreeSide(S.treeSide, true, item.moreTarget));
  }
  else if (action === 'stop') { intent('stop'); }
  else if (action === 'reload') { uncover(); intent('reload'); }
  else if (action === 'open') activate(item.page);
  else if (action === 'dismiss') intent('dismiss', { id: item.page });
  else if (action === 'branch') {
    if (v.queued) intent('branch-queued', { id: item.page });
    else openPalette({ scope: item.page, text: '' });
  } else if (action === 'source') activate(v.parent);
  else if (action === 'comment') intent('open-citation', { id: item.page });
});

// ---------- waiting pages and pinned controls ----------

function applyStrip(layouts) {
  S.strip = layouts;
  $('strip-clip').style.left = `${layouts.left}px`;
  $('strip-clip').style.width = `${layouts.room}px`;
  placePins();
  renderNav();
}

/// Back and Forward name the page they open.
function renderNav() {
  $('nav').hidden = !S.strip?.left;
  $('strip-placeholder').style.left = `${(S.strip?.left ?? 0) + 12}px`;
  for (const [kind, label] of [['back', 'Back'], ['forward', 'Forward']]) {
    const button = $(`nav-${kind}`);
    const target = S.strip?.[kind];
    button.disabled = !target;
    button.setAttribute('aria-label', target ? `${label} to ${target.label}` : label);
    button.title = shortcutTitle(target ? `${label} to ${target.label}` : label, kind);
  }
}

function placePins() {
  const placed = new Map((S.strip?.pins ?? []).map((p) => [p.kind, p]));
  for (const el of $('pins').children) {
    const p = placed.get(el.dataset.pin);
    el.hidden = !p;
    if (p) Object.assign(el.style, { left: `${p.x - p.w / 2}px`, top: `${p.y - p.s / 2}px`, width: `${p.w}px`, height: `${p.s}px` });
  }
}

function renderMailPin() {
  const pin = S.frame?.mail?.pin;
  if (!pin) return;
  $('mail-pin').innerHTML = icon('mail') + (pin.count ? `<span class="count">${esc(pin.count)}</span>` : '');
  $('mail-pin').setAttribute('aria-label', pin.name);
  $('mail-pin').toggleAttribute('data-stale', pin.stale);
  setTooltip($('mail-pin'), pin.name);
}

/// Pages playing sound away from the screen stay one click away, whatever tree is shown.
function renderPlayingPin() {
  const ids = S.strip?.playing ?? [];
  const first = visit(ids[0]);
  const pin = $('playing-pin');
  pin.dataset.go = first?.id ?? '';
  pin.innerHTML = first ? visitFav(first, 16) + icon('sound') + (ids.length > 1 ? `<span class="count">${ids.length}</span>` : '') : '';
  const names = ids.map((id) => visit(id)?.label).filter(Boolean);
  const label = first ? `Playing sound: ${names.join(', ')}` : 'Nothing playing';
  pin.setAttribute('aria-label', first ? `${label}. Go to ${first.label}` : label);
  setTooltip(pin, first ? `${label} · a click goes to ${first.label}` : '');
}

// ---------- held page filmstrip ----------

function switcherBox(el, box) {
  el.hidden = !box;
  if (!box) return;
  [el.style.left, el.style.top, el.style.width, el.style.height] = box.map(n => `${n}px`);
}

function layoutSwitcher(layout) {
  const deck = $('switcher');
  for (const [name, box] of Object.entries({
    backdrop: layout.backdrop, stage: layout.stage, caption: layout.caption, row: layout.row,
    run: layout.run, more: layout.more, keys: layout.footer,
  })) switcherBox($(`switcher-${name}`), box);
  deck.style.setProperty('--picture-height', `${layout.picture_height}px`);
  for (const [id, { el }] of S.switcher.rows) switcherBox(el, layout.cards[id]);
  switcherBox(S.switcher.divider, layout.divider);
  scrollSwitcherSelection();
}

function switcherCard(v) {
  return `<div class="switcher-fallback">${visitFav(v, 20)}<b>${esc(v.label)}</b><span>${esc(v.pictureStatus)}</span></div>`;
}

function showSwitcher(data) {
  rememberFocus('switcher');
  closeReader(false);
  closePalette(false);
  closePopover(false);
  hideList();
  const run = $('switcher-run');
  const divider = document.createElement('div');
  divider.className = 'switcher-divider';
  divider.setAttribute('aria-hidden', 'true');
  run.append(divider);
  const rows = new Map();
  for (const v of data.rows) {
    const el = document.createElement('div');
    el.id = `switcher-choice-${v.id}`;
    el.className = 'switcher-option';
    el.dataset.id = v.id;
    el.dataset.kind = v.kind;
    el.setAttribute('role', 'option');
    el.setAttribute('aria-label', v.name);
    el.title = v.kind === 'here' ? `${v.label} · You are here` : v.label;
    el.innerHTML = `<div class="switcher-picture">${switcherCard(v)}</div>
      <div class="switcher-label">${markedFav(v, 14, false)}<span>${esc(v.kind === 'here' ? 'You are here' : v.shortName)}</span></div>`;
    run.append(el);
    rows.set(v.id, { el, v });
  }
  S.switcher = { opening: data.opening, rows, divider, selected: data.selected };
  $('switcher').hidden = false;
  $('switcher-row').scrollLeft = 0;
  $('switcher-more').textContent = `${data.moreWaiting} more waiting`;
  layoutSwitcher(data.layout);
  selectSwitcher(data.selected);
  syncLayer();
  $('switcher-row').focus({ preventScroll: true });
  const opening = S.switcher;
  requestAnimationFrame(() => {
    if (S.switcher !== opening) return;
    if (data.timing) opening.revealed = performance.now();
    post({ t: 'switcher', opening: opening.opening, action: 'painted' });
  });
}

function scrollSwitcherSelection() {
  const selected = S.switcher.rows.get(S.switcher.selected)?.el;
  if (!selected) return;
  const row = $('switcher-row');
  const bounds = row.getBoundingClientRect();
  const card = selected.getBoundingClientRect();
  if (card.left - 6 < bounds.left) row.scrollLeft += card.left - 6 - bounds.left;
  else if (card.right + 6 > bounds.right) row.scrollLeft += card.right + 6 - bounds.right;
}

function patchSwitcher(data) {
  const sw = S.switcher;
  if (sw?.opening !== data.opening) return;
  const kept = new Set(data.rows.map(v => v.id));
  for (const [id, row] of sw.rows) {
    if (kept.has(id)) continue;
    row.images?.forEach(img => img.removeAttribute('src'));
    row.el.remove();
    sw.rows.delete(id);
  }
  for (const v of data.rows) {
    const row = sw.rows.get(v.id);
    if (!row || row.v.name === v.name && row.v.shortName === v.shortName && row.v.caption === v.caption) continue;
    row.v = { ...v, pictureStatus: row.v.pictureStatus };
    row.el.setAttribute('aria-label', v.name);
    row.el.title = v.kind === 'here' ? `${v.label} · You are here` : v.label;
    row.el.lastElementChild.innerHTML = `${markedFav(v, 14, false)}<span>${esc(v.kind === 'here' ? 'You are here' : v.shortName)}</span>`;
  }
  $('switcher-more').textContent = `${data.moreWaiting} more waiting`;
  selectSwitcher(sw.rows.has(sw.selected) ? sw.selected : data.selected);
  layoutSwitcher(data.layout);
}

function selectSwitcher(id) {
  const sw = S.switcher;
  const chosen = sw.rows.get(id);
  if (!chosen) return;
  sw.selected = id;
  const { el, v } = chosen;
  for (const row of sw.rows.values()) row.el.setAttribute('aria-selected', String(row === chosen));
  $('switcher-row').setAttribute('aria-activedescendant', el.id);
  paintSwitcherStage(chosen);
  $('switcher-caption').title = `${v.label} · ${v.site} · ${v.caption}`;
  $('switcher-caption').innerHTML = `<div>${markedFav(v, 16, false)}<b>${esc(v.label)}</b><span>${esc(v.site)} · ${esc(v.caption)}</span></div>`;
  scrollSwitcherSelection();
}

function paintSwitcherStage(row) {
  const stage = $('switcher-stage');
  if (row.large) stage.replaceChildren(row.large);
  else stage.innerHTML = switcherCard(row.v);
}

async function fillSwitcherPicture({ id, large, thumb }) {
  const sw = S.switcher;
  const row = sw.rows.get(id);
  if (!row) return;
  try {
    if (!thumb) throw new Error('Snapshot unavailable');
    row.images = [thumb, large].filter(Boolean).map(url => {
      const image = new Image();
      image.alt = '';
      image.src = url;
      return image;
    });
    await Promise.all(row.images.map(img => img.decode()));
    const [tile, stage] = row.images;
    if (S.switcher !== sw || !sw.rows.has(id)) return;
    row.large = stage;
    row.el.firstElementChild.replaceChildren(tile);
    if (sw.selected === id) paintSwitcherStage(row);
    row.el.dataset.picture = 'ready';
    if (sw.revealed !== undefined && [...sw.rows.values()].every(({ el }) => el.dataset.picture === 'ready')) {
      requestAnimationFrame(() => {
        if (S.switcher === sw) sw.picturesPainted = performance.now() - sw.revealed;
      });
    }
  } catch {
    if (S.switcher !== sw || !sw.rows.has(id)) return;
    row.v.pictureStatus = 'Preview unavailable';
    row.el.firstElementChild.innerHTML = switcherCard(row.v);
    if (sw.selected === id) paintSwitcherStage(row);
  }
}

function closeSwitcher(restoreFocus) {
  for (const row of S.switcher.rows.values()) row.images?.forEach(img => img.removeAttribute('src'));
  S.switcher = null;
  $('switcher').hidden = true;
  $('switcher-run').replaceChildren();
  $('switcher-stage').replaceChildren();
  $('switcher-caption').replaceChildren();
  $('switcher-row').removeAttribute('aria-activedescendant');
  syncLayer();
  if (restoreFocus) returnFocus('switcher');
  else focusReturns.delete('switcher');
}

function switcherPointer(action, id) {
  if (id && S.switcher) post({ t: 'switcher', opening: S.switcher.opening, action, id });
}

let switcherPointerPosition;
document.addEventListener('pointermove', (event) => {
  const p = `${event.screenX},${event.screenY}`;
  if (p === switcherPointerPosition) return;
  switcherPointerPosition = p;
  if (!event.buttons) switcherPointer('hover', event.target.closest('.switcher-option')?.dataset.id);
}, true);
$('switcher-row').addEventListener('click', event => {
  switcherPointer('open', event.target.closest('.switcher-option')?.dataset.id);
});
$('switcher-stage').addEventListener('click', () => switcherPointer('open', S.switcher?.selected));

function enterQueue(el) {
  cancel('waitingClose');
  if (nativeDrag || S.layer !== 'none' || S.popover) return;
  schedule('waitingOpen', 120, () => {
    if (nativeDrag || S.layer !== 'none' || S.popover || nodeAt(el)?.kind !== 'overflow') return;
    openQueue(el);
  });
}

function leaveQueue() {
  cancel('waitingOpen');
  const popover = S.popover;
  if (popover?.kind === 'waiting') schedule('waitingClose', 220, () => {
    if (S.popover === popover) closePopover();
  });
}

function openQueue(el) {
  cancel('waitingOpen');
  cancel('waitingClose');
  if (S.popover?.kind === 'waiting' && S.popover.anchor === el) return;
  closePopover(false);
  openPopover('waiting', el, null);
}

$('playing-pin').addEventListener('click', () => { const id = $('playing-pin').dataset.go; if (id) activate(id); });
$('popover').addEventListener('mouseenter', () => cancel('waitingClose'));
$('popover').addEventListener('mouseleave', event => {
  if (!event.relatedTarget?.closest('.node.overflow')) leaveQueue();
});

// ---------- popovers: Saved text menu, trees that do not fit, a tree's name ----------

function setTooltip(anchor, title) {
  if (S.popover?.anchor === anchor) {
    S.popover.title = title;
    anchor.removeAttribute('title');
  } else if (title == null) anchor.removeAttribute('title');
  else anchor.title = title;
}

/// Opens a menu under `anchor`, or under `below` when the anchor sits on top of something
/// the menu should not cover.
function openPopover(kind, anchor, data, below = anchor) {
  const popover = $('popover');
  const parent = anchor.closest('#popover') ? S.popover : null;
  rememberFocus('popover', anchor);
  S.popover = { kind, anchor, data, parent, listKey: kind === 'node' ? S.list?.key : null };
  anchor.setAttribute('aria-expanded', 'true');
  setTooltip(anchor, anchor.getAttribute('title'));
  renderPopover();
  popover.hidden = false;
  positionPopover(anchor, below);
  post({ t: 'focus', target: 'chrome' });
  controlsIn(popover)[0]?.focus();
  if (S.layer !== 'palette') setLayer('menu');
  syncLayer();
}

function positionPopover(anchor, below) {
  const popover = $('popover');
  const a = anchor.getBoundingClientRect();
  const b = below.getBoundingClientRect();
  const width = popover.offsetWidth;
  popover.style.left = `${clamp(a.left, 8, window.innerWidth - width - 8)}px`;
  const windowHeight = S.windowHeight ?? window.innerHeight;
  const bottom = S.overlay === 'tree' ? Math.min(windowHeight, $('tree-sheet').querySelector('.sheet-box').getBoundingClientRect().bottom) : windowHeight;
  const belowRoom = bottom - b.bottom - 14;
  const above = belowRoom < 200 && a.top > belowRoom;
  popover.style.maxHeight = `${Math.max(40, above ? a.top - 20 : belowRoom - 14)}px`;
  popover.style.top = `${above ? Math.max(8, a.top - popover.offsetHeight - 6) : b.bottom + 6}px`;
}

function keepPopoverInside() {
  const popover = $('popover');
  if (!S.popover || popover.hidden) return;
  const left = S.popover.anchor.getBoundingClientRect().left;
  popover.style.left = `${clamp(left, 8, window.innerWidth - popover.offsetWidth - 8)}px`;
}

function closePopover(restoreFocus = true) {
  if (!S.popover) return;
  const { anchor, title, parent, listKey } = S.popover;
  anchor.setAttribute('aria-expanded', 'false');
  S.popover = null;
  setTooltip(anchor, title);
  $('popover').hidden = true;
  if (listKey && restoreFocus) {
    setLayer(restingLayer());
    showList(listKey);
  }
  if (!restoreFocus && S.overlay === 'tree' && canFocus(anchor)) {
    anchor.focus({ preventScroll: true });
  }
  if (S.layer === 'menu') setLayer(restingLayer());
  if (restoreFocus && parent) {
    openPopover(parent.kind, parent.anchor, parent.data);
    const row = anchor.id ? $(anchor.id) : [...$('popover').querySelectorAll('[data-id]')].find((el) => el.dataset.id === anchor.dataset.id && el.dataset.start === anchor.dataset.start);
    row?.focus();
  } else if (restoreFocus) returnFocus('popover');
  syncLayer();
}

// ---------- saved logins for the page's sign-in fields ----------

const loginNames = () => S.logins?.names ?? [];
const loginLabel = (name) => name || 'No user name';

/// The key button in the tab row. A login waiting to be saved comes before the logins that
/// could be filled: it is the one that needs an answer.
function renderLoginChip() {
  const chip = $('login-chip');
  const names = loginNames();
  chip.hidden = !S.save && !names.length && !S.locked;
  chip.dataset.offer = String(!!S.save);
  if (S.save) {
    chip.innerHTML = `${icon('key')}<span>${esc(saveAction(S.save))}?</span>`;
    setTooltip(chip, `${S.save.update ? 'Update the saved password' : 'Save the login'} for ${loginLabel(S.save.username)} on ${S.save.site} in ${S.save.keeper}`);
  } else if (S.locked && !names.length) {
    chip.innerHTML = `${icon('key')}<span>${esc(`Unlock ${S.locked}`)}</span>`;
    setTooltip(chip, shortcutTitle(`Unlock ${S.locked} to fill a saved login`, 'fill-login'));
  } else if (names.length) {
    const one = names.length === 1;
    chip.innerHTML = `${icon('key')}<span>${esc(one ? loginLabel(names[0]) : `${names.length} logins`)}</span>`;
    setTooltip(chip, shortcutTitle(one ? `Fill the saved login for ${loginLabel(names[0])}` : 'Choose a saved login to fill', 'fill-login'));
  }
  if (S.popover?.kind === 'save-login' && !S.save) closePopover(false);
  if (S.popover?.kind === 'logins') {
    if (names.length > 1 && !S.save) refreshPopover();
    else closePopover(false);
  }
  fitTabs();
}

const saveAction = (offer) => `${offer.update ? 'Update in' : 'Save to'} ${offer.keeper}`;

function answerSave(save) {
  const offer = S.save?.offer;
  closePopover(false);
  if (offer != null) intent('save-login', { offer, save });
}

/// One saved login is filled at once; several open the list under the chip.
function fillLogin() {
  const names = loginNames();
  if (!names.length && S.locked) { intent('unlock-logins'); return; }
  if (!names.length) { canopy.toast('info', 'No saved login for this page'); return; }
  if (names.length === 1) { pickLogin(0); return; }
  if (S.popover?.kind === 'logins') { closePopover(); return; }
  openPopover('logins', $('login-chip'));
  post({ t: 'focus', target: 'chrome' });
  $('popover').querySelector('[data-login]')?.focus();
}

function pickLogin(index) {
  intent('fill-login', { offer: S.logins.offer, index });
}

function visitRow(v) {
  const detail = v.queued ? 'waiting' : v.site.split(' · ').slice(1).join(' · ');
  return `<button class="menu-row" data-id="${v.id}">${visitFav(v, 16)}<span class="grow">${esc(v.label)}</span><span class="menu-detail">${esc(detail)}</span></button>`;
}

function visibleMailAccounts(mail) {
  if (!mail.tabs?.some((tab) => tab.key === S.mailTab)) S.mailTab = 'all';
  return mail.accounts.filter((account) => S.mailTab === 'all' || account.address === S.mailTab);
}

const MAIL_HUES = ['#3b7ad6', '#d9962e', '#d1547a', '#49a888', '#8f6ed0'];

function mailAccountDot(account) {
  return account?.hue === undefined ? ''
    : `<span class="mail-account-dot" style="background:${MAIL_HUES[account.hue % MAIL_HUES.length]}" title="${esc(account.address ?? account.name)}" aria-hidden="true"></span>`;
}

function mailTile(row) {
  const { shape, icon } = row.tile;
  const letter = esc((Array.from(row.from)[0] || '?').toUpperCase());
  const image = icon
    ? `<img src="icon?sender=1&u=${encodeURIComponent(icon)}&r=${S.mailIconRevision}" alt="" onload="this.parentElement.classList.add('has-icon')" onerror="this.parentElement.classList.remove('has-icon')">` : '';
  const dot = row.fresh ? '<i class="mail-new"></i>' : '';
  return `<span class="mail-tile ${shape}" style="--sender-color:${monogramColor(row.from)}" aria-hidden="true"><b>${letter}</b>${image}${dot}</span>`;
}

function mailMark(row) {
  return row.markName ? `<button class="icon-button mail-mark" data-account="${esc(row.account)}" data-id="${esc(row.id)}" title="Mark read" aria-label="${esc(row.markName)}">${icon('read')}</button>` : '';
}

function mailNote(row) {
  return row.unmarked ? `<span class="mail-unmarked">${esc(row.unmarked)}</span>` : '';
}

function mailRow(row, accountDot, when) {
  return `<div class="mail-item${row.fresh ? ' fresh' : ''}"><button class="mail-row" data-account="${esc(row.account)}" data-id="${esc(row.id)}" data-sender="${esc(row.sender)}" aria-label="${esc(row.name)}">`
    + mailTile(row) + '<span class="mail-body">'
    + `<span class="mail-line"><span class="mail-from">${esc(row.from)}</span><span class="mail-when">${esc(when)}</span></span>`
    + `<span class="mail-line mail-preview"><span class="mail-subject">${esc(row.subject)}</span><span class="mail-snippet">${esc(row.snippet)}</span>${accountDot}</span>`
    + mailNote(row)
    + '</span></button>' + mailMark(row) + '</div>';
}

function mailTimeline(rows, rowDot) {
  let previousDay;
  return rows.map((row) => {
    const heading = previousDay === row.day ? '' : `<div class="mail-day" role="heading" aria-level="2">${esc(row.day)}</div>`;
    previousDay = row.day;
    return heading + mailRow(row, rowDot(row), row.when);
  }).join('');
}

function mailChild(row, accountDot, showFrom) {
  return `<div class="mail-item mail-kid${row.fresh ? ' fresh' : ''}"><button class="mail-row" data-account="${esc(row.account)}" data-id="${esc(row.id)}" data-sender="${esc(row.sender)}" aria-label="${esc(row.name)}">`
    + '<span class="mail-body"><span class="mail-line mail-preview">'
    + (row.fresh ? '<i class="mail-child-new" aria-hidden="true"></i>' : '')
    + (showFrom ? `<span class="mail-from">${esc(row.from)}</span>` : '')
    + `<span class="mail-subject">${esc(row.subject)}</span><span class="mail-snippet">${esc(row.snippet)}</span>${accountDot}<span class="mail-when">${esc(row.ago)}</span></span>`
    + mailNote(row) + '</span></button>' + mailMark(row) + '</div>';
}

function mailSenders(rows, rowDot) {
  const groups = new Map();
  for (const row of rows) {
    if (!groups.has(row.sender)) groups.set(row.sender, []);
    groups.get(row.sender).push(row);
  }
  return [...groups].map(([sender, group]) => {
    const newest = group[0];
    if (group.length === 1) return mailRow(newest, rowDot(newest), newest.ago);

    const open = S.openMailSenders.has(sender);
    const newCount = group.filter((row) => row.fresh).length;
    const namesDiffer = group.some((row) => row.from !== newest.from);
    const from = namesDiffer ? newest.senderSite ?? newest.from : newest.from;
    const summary = `${newCount ? `${newCount} new, ` : ''}${group.length} unread`;
    const more = `and ${group.length - 1} more`;
    const name = [from, open ? summary : `${newest.subject}, ${more}`, newest.ago].filter(Boolean).join(', ');

    return `<div class="mail-item${newCount ? ' fresh' : ''}"><button class="mail-group-head" data-sender="${esc(sender)}" aria-expanded="${open}" aria-label="${esc(name)}">`
      + mailTile({ ...newest, from, fresh: newCount > 0 })
      + `<span class="mail-body"><span class="mail-line"><span class="mail-from">${esc(from)}</span><span class="count mail-fold">${group.length}${icon('chevron')}</span><span class="mail-when">${esc(newest.ago)}</span></span>`
      + `<span class="mail-line mail-preview"><span class="mail-subject">${esc(open ? summary : newest.subject)}</span>`
      + (open ? '' : `<span class="mail-snippet">${more}</span>`)
      + '</span></span></button></div>'
      + (open ? '<div class="mail-kids">' + group.map((row) => mailChild(row, rowDot(row), namesDiffer)).join('') + '</div>' : '');
  }).join('');
}

// A refresh rebuilds an open popover's rows. The row that had the keyboard keeps it.
function refreshPopover() {
  if (!S.popover) return;
  const popover = $('popover');
  if (S.popover.kind === 'mail') {
    const active = document.activeElement;
    const address = active?.dataset.account;
    const id = active?.dataset.id;
    const key = active?.dataset.key;
    const sender = active?.dataset.sender ?? active?.closest('.mail-kid')?.querySelector('.mail-row')?.dataset.sender;
    const role = ['mail-tab', 'mail-row', 'mail-mark', 'mail-inbox', 'mail-group-head', 'mail-group-toggle']
      .find((c) => active?.classList.contains(c));
    const ids = address ? [...popover.querySelectorAll('.mail-row')].filter((row) => row.dataset.account === address)
      .map((row) => row.dataset.id) : [];
    const index = ids.indexOf(id);
    const near = index < 0 ? [] : [...ids.slice(index + 1), ...ids.slice(0, index).reverse()];
    const hadFocus = popover.contains(active);
    const scrollTop = popover.scrollTop;
    renderPopover();
    if (!S.popover) return;
    keepPopoverInside();
    popover.scrollTop = scrollTop;
    if (hadFocus) {
      const controls = role ? [...popover.querySelectorAll(`.${role}`)] : [];
      const sameMessage = (wanted) => controls.find((el) => el.dataset.account === address && el.dataset.id === wanted);
      let row = role === 'mail-row' || role === 'mail-mark' ? sameMessage(id)
        : controls.find((el) => role === 'mail-tab' ? el.dataset.key === key
          : role === 'mail-group-head' ? el.dataset.sender === sender : el.dataset.account === address);
      row ||= near.map(sameMessage).find(Boolean);
      if (!row && sender !== undefined && S.frame.mail.grouping === 'sender') {
        const related = [...popover.querySelectorAll('.mail-group-head, .mail-row')];
        row = related.find((el) => el.dataset.sender === sender);
      }
      (row || (role === 'mail-tab' && popover.querySelector('.mail-tab[aria-selected="true"]')) || popover).focus({ preventScroll: true });
      row?.scrollIntoView({ block: 'nearest' });
    }
    syncLayer();
    return;
  }
  if (S.popover.kind === 'waiting') {
    const active = document.activeElement;
    const id = active?.dataset.id;
    const hadFocus = popover.contains(active);
    renderPopover();
    if (hadFocus && S.popover) {
      const rows = [...popover.querySelectorAll('[data-id]')];
      (rows.find(row => row.dataset.id === id) ?? rows[0])?.focus({ preventScroll: true });
    }
    return;
  }
  const index = [...popover.querySelectorAll('button')].indexOf(document.activeElement);
  renderPopover();
  if (index >= 0 && S.popover) popover.querySelectorAll('button')[index]?.focus();
}

function renderPopover() {
  const popover = $('popover');
  const { kind, data } = S.popover;
  if (['errors', 'settings', 'tree'].includes(kind) && popover.dataset.kind !== kind) popover.replaceChildren();
  popover.dataset.kind = kind;
  popover.setAttribute('role', ['node', 'tree', 'bookmark', 'move-tree'].includes(kind) ? 'menu' : 'dialog');
  popover.setAttribute('aria-label', kind === 'node' ? 'Page actions' : kind === 'tree' ? 'Tree actions' : kind === 'memory' ? 'Saved text' : kind === 'errors' ? 'Recent errors' : kind.replaceAll('-', ' '));
  popover.classList.toggle('waiting', kind === 'waiting');
  popover.classList.toggle('mail', kind === 'mail');
  if (kind !== 'node') delete popover.dataset.nodeHtml;
  if (kind === 'node') {
    const v = visit(data);
    if (!v) { closePopover(false); return; }
    const html = `<div class="node-menu-title">${visitFav(v, 16)}<span>${esc(v.shortName)}</span></div>` + v.nodeMenu.map((r) => `<button class="menu-row" role="menuitem" data-node-action="${r.kind}">${esc(r.label)}</button>`).join('');
    if (popover.dataset.nodeHtml !== html || popover.dataset.nodeId !== data) {
      popover.innerHTML = html;
      popover.dataset.nodeHtml = html;
      popover.dataset.nodeId = data;
      for (const button of popover.querySelectorAll('[data-node-action]')) button.onclick = () => {
        const action = button.dataset.nodeAction;
        if (action === 'choose-move-tree') {
          const { anchor, listKey } = S.popover;
          const request = ++nextMoveRequest;
          closePopover(false);
          openPopover('move-tree', anchor, { head: data, request, choices: null });
          S.popover.listKey = listKey;
          post({ t: 'branch-move-choices', head: data, request });
          return;
        }
        closePopover();
        if (action === 'open') activate(data);
        else if (action === 'new-tree') { uncover(); intent('open-page-in-tree', { id: data }); }
        else if (action === 'branch') openPalette({ scope: data, text: '' });
        else if (action === 'copy') post({ t: 'copy', text: visit(data).url });
        else intent(action, { id: data });
      };
    }
  } else if (kind === 'move-tree') {
    const choices = data.choices;
    popover.innerHTML = '<div class="menu-heading">Choose a tree</div>'
      + (choices?.warning ? `<div class="menu-hint">${esc(choices.warning)}</div>` : '')
      + (choices ? (choices.trees.length ? choices.trees.map((tree) => `<button class="menu-row" role="menuitem" data-move-tree="${tree.id}">${esc(tree.title)}</button>`).join('') : '<div class="menu-hint">No trees available</div>') : '<div class="menu-hint">Loading trees…</div>')
      + '<button class="menu-row" role="menuitem" data-move-cancel>Cancel</button>';
    for (const button of popover.querySelectorAll('[data-move-tree]')) button.onclick = () => {
      const to = { kind: 'tree', id: button.dataset.moveTree };
      closePopover();
      intent('move-branch', { head: data.head, to });
    };
    popover.querySelector('[data-move-cancel]').onclick = () => closePopover();
  } else if (kind === 'tree') {
    const rows = TREE_MENUS[data.source](data.id);
    if (!rows) { closePopover(false); return; }
    patchChildren(popover, rows.map((r) => `<button class="menu-row" role="menuitem" data-key="${r.act}" data-tab-action="${r.act}">${esc(r.label)}</button>`).join(''));
    popover.querySelectorAll('[data-tab-action]').forEach((el, i) => { el.onclick = () => {
      const row = rows[i], anchor = S.popover.anchor;
      closePopover();
      if (row.act === 'rename') {
        openPopover('rename', anchor, { ...data, name: row.name });
        S.popover.field.focus(); S.popover.field.select();
      } else treeAct(data.id, row.act);
    }; });
  } else if (kind === 'bookmark') {
    popover.innerHTML = '<button class="menu-row" role="menuitem" data-remove-bookmark>Remove bookmark</button>';
    popover.querySelector('button').addEventListener('click', () => { closePopover(); intent('remove-bookmark', { id: Number(data) }); });
  } else if (kind === 'folder') {
    const b = S.frame.start.bookmarks;
    if (data.hidden) {
      popover.innerHTML = (b.hidden?.folders ?? []).map((f) => `<button class="menu-row" data-start="folder" data-id="${esc(f.name)}">${icon('folder')}<span class="grow">${esc(f.name)}</span><span class="menu-detail">${f.count}</span></button>`).join('');
    } else {
      const folder = [...b.folders, ...(b.hidden?.folders ?? [])].find((f) => f.name === data.folder);
      const rows = data.more ? b.more?.rows : folder?.rows;
      popover.innerHTML = (rows ?? []).map((r) => `<button class="menu-row" data-start="bookmark" data-id="${r.id}" title="${esc(r.label + '\n' + r.url)}">${fav(r.url, true, 16)}<span class="grow">${esc(r.label)}</span></button>`).join('');
    }
  } else if (kind === 'mail') {
    const mail = S.frame.mail;
    if (!mail.accounts.length) { closePopover(false); return; }
    popover.setAttribute('aria-label', 'Unread mail');
    const accounts = visibleMailAccounts(mail);
    const byAddress = new Map(accounts.map((a) => [a.address, a]));
    const rows = mail.rows.filter((row) => byAddress.has(row.account));
    const rowDot = (row) => mail.tabs && S.mailTab === 'all' ? mailAccountDot(byAddress.get(row.account)) : '';
    const header = mail.tabs ? '<div class="segmented mail-tabs" role="tablist" aria-label="Mail accounts">'
      + mail.tabs.map((tab) => `<button class="mail-tab" role="tab" data-key="${esc(tab.key)}" aria-selected="${tab.key === S.mailTab}" tabindex="${tab.key === S.mailTab ? 0 : -1}" title="${esc(tab.name)}" aria-label="${esc(tab.name)}">${mailAccountDot(tab)}<span class="mail-tab-label">${esc(tab.label)}</span>${tab.count ? `<span class="count">${esc(tab.count)}</span>` : ''}</button>`).join('') + '</div>'
      : `<div class="mail-heading" role="heading" aria-level="1"><span>${esc(accounts[0].address)}</span><span class="menu-detail">${esc(accounts[0].count)}</span></div>`;
    const bySender = mail.grouping === 'sender';
    popover.innerHTML = '<div class="mail-header">' + header
      + `<button class="label-button mail-group-toggle" aria-pressed="${bySender}" aria-label="Group by sender" title="Group by sender">${icon('forest')}</button></div>`
      + (mail.notice ? `<div class="mail-notice" role="status">${esc(mail.notice)}</div>` : '')
      + (rows.length ? (bySender ? mailSenders(rows, rowDot) : mailTimeline(rows, rowDot))
        : '<div class="mail-empty">No unread mail</div>')
      + '<div class="mail-footers">' + accounts.map((a) =>
        `<button class="menu-row mail-inbox" data-account="${esc(a.account)}" aria-label="${esc(a.name + (a.more ? '. ' + a.more : ''))}">`
        + (mail.tabs ? mailAccountDot(a) : '')
        + `<span class="grow">${esc(a.address)}</span><span class="menu-detail">${esc(a.more ?? 'Open inbox')}</span></button>`).join('') + '</div>';
    popover.querySelectorAll('.mail-tab').forEach((tab) => {
      const select = (key) => {
        S.mailTab = key;
        refreshPopover();
        [...popover.querySelectorAll('.mail-tab')].find((el) => el.dataset.key === key)?.focus({ preventScroll: true });
      };
      tab.addEventListener('click', () => select(tab.dataset.key));
      tab.addEventListener('keydown', (event) => {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        const tabs = mail.tabs;
        const index = tabs.findIndex((item) => item.key === tab.dataset.key);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length;
        event.preventDefault();
        event.stopPropagation();
        select(tabs[next].key);
      });
    });
    popover.querySelector('.mail-group-toggle').onclick = (event) => {
      event.currentTarget.focus({ preventScroll: true });
      intent('mail-grouping', { grouping: bySender ? 'day' : 'sender' });
    };
    popover.querySelectorAll('.mail-group-head').forEach((head) => head.onclick = () => {
      head.focus({ preventScroll: true });
      const sender = head.dataset.sender;
      if (S.openMailSenders.has(sender)) S.openMailSenders.delete(sender);
      else S.openMailSenders.add(sender);
      refreshPopover();
    });
    popover.querySelectorAll('.mail-mark').forEach((button) => button.addEventListener('click', (event) => {
      // Reflow can put the next mail under the second click of a double click.
      if (event.detail > 1) return;
      intent('mark-mail-read', { account: button.dataset.account, id: button.dataset.id });
    }));
    popover.querySelectorAll('.mail-row').forEach((row) => row.addEventListener('click', () => {
      closePopover(false);
      intent('open-mail', { account: row.dataset.account, id: row.dataset.id });
    }));
    popover.querySelectorAll('.mail-inbox').forEach((row) => row.addEventListener('click', () => {
      closePopover(false);
      intent('open-inbox', { account: row.dataset.account });
    }));
  } else if (kind === 'waiting') {
    const { ids, order } = S.frame.waiting;
    if (!ids.length) { closePopover(false); return; }
    patchChildren(popover, `<div data-key="heading" class="pop-head"><span class="menu-heading grow">Waiting in this tree · ${ids.length}</span><span class="menu-detail">${esc(order)}</span></div>`
      + ids.map((id, i) => {
        const v = visit(id);
        return `<button class="menu-row${i === 0 ? ' next' : ''}" data-id="${id}" aria-current="${id === current() ? 'page' : 'false'}" aria-label="${esc(v.name)}">${markedFav(v, 16)}<span class="grow">${esc(v.label)}</span><span class="menu-detail">${esc(v.detail)}</span>${i === 0 ? shortcutChip('switch-page') : ''}</button>`;
      }).join(''));
    popover.querySelectorAll('[data-id]').forEach(row => { row.onclick = () => activate(row.dataset.id); });
  } else if (kind === 'save-login') {
    const save = S.save;
    if (!save) { closePopover(false); return; }
    popover.innerHTML = `<div class="menu-heading">${save.update ? 'Update the saved password?' : 'Save this login?'}</div>`
      + `<div class="menu-row" id="save-login-account">${icon('key')}<span class="grow">${esc(loginLabel(save.username))}</span><span class="menu-detail">${esc(save.site)}</span></div>`
      + `<div class="card-actions save-actions"><button class="action" data-save="1">${esc(saveAction(save))}</button><button class="action" data-save="0">Not now</button></div>`;
    popover.querySelectorAll('[data-save]').forEach((button) => button.addEventListener('click', () => answerSave(button.dataset.save === '1')));
  } else if (kind === 'logins') {
    popover.innerHTML = '<div class="menu-heading">Saved logins for this site</div>'
      + loginNames().map((name, index) => `<button class="menu-row" data-login="${index}">${icon('key')}<span class="grow">${esc(loginLabel(name))}</span></button>`).join('');
    popover.querySelectorAll('[data-login]').forEach((row) => row.addEventListener('click', () => {
      const index = Number(row.dataset.login);
      closePopover(false);
      pickLogin(index);
    }));
  } else if (kind === 'trees') {
    const hidden = [...$('tab-row').children].filter((tab) => tab.hidden).map((tab) => tab.dataset.id);
    const rows = (S.frame?.tabs ?? []).filter((tree) => hidden.includes(tree.id));
    const kept = new Map([...popover.children].map((row) => [row.dataset.tree, row]));
    const nodes = rows.map((tree) => {
      const root = visit(tree.id);
      const row = kept.get(tree.id) ?? document.createElement('div');
      row.className = 'overflow-tree';
      row.dataset.tree = tree.id;
      const html = `<button class="menu-row" data-id="${tree.id}">${root ? visitFav(root, 16) : ''}<span class="grow">${esc(tree.title)}</span>`
          + (tree.pending ? `<span class="count" aria-label="${tree.pending} waiting">${tree.pending}</span>` : '') + '</button>'
        + `<button class="overflow-settle" data-settle="${tree.id}" title="Settle tree" aria-label="Settle ${esc(tree.title)}">${icon('close')}</button>`;
      if (row.dataset.html !== html) {
        row.dataset.html = html;
        const select = row.querySelector('[data-id]');
        const settle = row.querySelector('[data-settle]');
        row.innerHTML = html;
        if (select) {
          select.innerHTML = row.querySelector('[data-id]').innerHTML;
          row.querySelector('[data-id]').replaceWith(select);
        }
        if (settle) row.querySelector('[data-settle]').replaceWith(settle);
        row.querySelector('[data-id]').onclick = () => {
          closePopover();
          uncover();
          treeAct(tree.id, 'open');
        };
        row.querySelector('[data-settle]').onclick = () => {
          treeAct(tree.id, 'settle');
        };
      }
      return row;
    });
    [...popover.children].filter((row) => !nodes.includes(row)).forEach((row) => row.remove());
    nodes.forEach((row, index) => { if (popover.children[index] !== row) popover.insertBefore(row, popover.children[index] ?? null); });
  } else if (kind === 'rename') {
    // A frame that arrives while a name is being typed must not reset the field.
    if (S.popover.field) return;
    const title = data.name;
    popover.innerHTML = '<div class="menu-heading">Tree name</div>'
      + `<input id="tree-name" class="menu-input" value="${esc(title)}" maxlength="80" spellcheck="false" aria-label="Tree name">`
      + '<div class="menu-hint">Leave it empty to name the tree after its pages.</div>';
    const field = S.popover.field = $('tree-name');
    field.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter') return;
      event.preventDefault();
      // Enter on the name as it was changes nothing: it stays automatic if it was.
      if (field.value.trim() !== title) intent('rename-tree', { id: data.id, name: field.value });
      closePopover();
    });
  } else if (kind === 'files') {
    const pages = (Array.isArray(data) ? data : [data]).filter((id) => filesOf(id).length);
    const files = pages.flatMap(filesOf);
    if (!files.length) {
      closePopover(false);
      return;
    }
    popover.innerHTML = `<div class="menu-heading">${pages.length > 1 ? `${files.length} files from ${pages.length} pages` : files.length === 1 ? 'Downloaded from this page' : `${files.length} files downloaded from this page`}</div>`
      + files.map((f) => `<div class="file-row${f.missing ? ' missing' : ''}">`
        + `<button class="menu-row" data-file="${f.id}" title="${esc(f.missing ? `${f.name} was moved or deleted` : `Open ${f.name}`)}">`
        + `${icon('file')}<span class="grow">${esc(f.name)}</span><span class="menu-detail">${esc(f.detail)}</span></button>`
        + `<button class="icon-button" data-folder="${f.id}" title="${esc(`Open ${f.folder}`)}" aria-label="${esc(`Open ${f.folder}, where ${f.name} was saved`)}">${icon('folder')}</button>`
        + '</div>').join('');
    popover.querySelectorAll('[data-file]').forEach((row) => row.addEventListener('click', () => {
      closePopover();
      intent('open-file', { file: Number(row.dataset.file) });
    }));
    popover.querySelectorAll('[data-folder]').forEach((button) => button.addEventListener('click', () => {
      closePopover();
      intent('open-file-folder', { file: Number(button.dataset.folder) });
    }));
  } else if (kind === 'memory') {
    popover.innerHTML = '<div class="menu-heading">Saved text</div>'
      + captureRow()
      + `<button class="menu-row" id="remember-selection"><span class="grow">Remember selection</span>${shortcutChip('remember-selection', 'text')}</button>`
      + `<button class="menu-row" id="read-captures"${current() ? '' : ' disabled'}><span class="grow">Read saved text</span></button>`;
    wireCaptureRow();
    $('remember-selection').addEventListener('click', () => { closePopover(); intent('remember'); });
    $('read-captures').addEventListener('click', () => { closePopover(); intent('read-captures', { id: current() }); });
  } else if (kind === 'errors') {
    patchChildren(popover, '<div class="errors-head" data-key="head"><b>Recent errors</b><button id="clear-errors" class="action">Clear</button></div>'
      + (S.errors.recent.length ? S.errors.recent.map((e, i) => `<div class="menu-error" data-key="error-${i}">${esc(e.text)}<small>${esc(e.when)}</small></div>`).join('') : '<div class="menu-hint" data-key="empty">No recent errors</div>'));
    $('clear-errors').disabled = !S.errors.recent.length;
    $('clear-errors').onclick = () => post({ t: 'errors', action: 'clear' });
  } else if (kind === 'passwords') {
    const { active, next, bitwarden } = S.passwordSettings;
    const choice = (key, name, detail, enabled) =>
      `<button class="menu-row" id="password-${key}" aria-pressed="${next === name}"${enabled ? '' : ' disabled'}><span class="grow">${name}</span><span class="menu-detail">${detail}</span>${next === name ? '<span aria-hidden="true">✓</span>' : ''}</button>`;
    popover.innerHTML = '<div class="menu-heading">Passwords</div>'
      + `<div class="menu-hint" id="password-active">${active ? `Saving logins to ${esc(active)}` : 'Saving logins is unavailable'}</div>`
      + '<div class="menu-heading">Use after restart</div>'
      + choice('vault', 'Canopy vault', 'This computer', true)
      + choice('bitwarden', 'Bitwarden', '', bitwarden)
      + (next && !['Canopy vault', 'Bitwarden'].includes(next) ? `<div class="menu-hint">Configured helper: ${esc(next)}</div>` : '')
      + (!bitwarden ? '<div class="menu-hint">Install the Canopy Bitwarden helper to use Bitwarden.</div>' : '')
      + '<div class="menu-hint">Existing logins stay in their current vault.</div>';
    $('password-vault').onclick = () => intent('password-vault');
    $('password-bitwarden').onclick = () => intent('password-bitwarden');
  } else if (kind === 'settings') {
    const theme = S.frame?.theme ?? 'dark';
    const page = current();
    const blocking = S.frame?.blocking;
    const site = pageShows() ? blocking?.site : null;
    const onPage = !!(page && visit(page));
    const bookmarked = !!S.frame?.bookmarked;
    const percent = S.pageScale?.page === page ? S.pageScale.percent : 100;
    const back = S.strip?.back;
    const forward = S.strip?.forward;
    const item = (id, label, command, enabled = true) =>
      `<button class="menu-row" role="menuitem" id="${id}" data-key="${id}"${enabled ? '' : ' disabled'}><span class="grow">${esc(label)}</span>${shortcutChip(command)}</button>`;
    const zoomButton = (id, text, name, command) =>
      `<button id="${id}" aria-label="${esc(name)}" title="${esc(shortcutTitle(name, command))}"${onPage ? '' : ' disabled'}>${text}</button>`;
    popover.setAttribute('aria-label', 'Menu');
    patchChildren(popover, '<div class="menu-heading" data-key="page-title">This page</div>'
      + item('menu-find', 'Find in page', 'find', onPage)
      + item('menu-bookmark', bookmarked ? 'Remove bookmark' : 'Bookmark this page', 'bookmark', onPage)
      + item('menu-copy', 'Copy address', '', onPage)
      + item('menu-reload', 'Reload', 'reload', onPage)
      + `<div class="menu-row menu-zoom" data-key="zoom"><span class="grow">Page size</span><span class="zoom-steps">${zoomButton('menu-zoom-out', '−', 'Smaller', 'page-size-decrease')}${zoomButton('menu-zoom-reset', `${percent}%`, 'Reset page size', 'page-size-reset')}${zoomButton('menu-zoom-in', '+', 'Larger', 'page-size-increase')}</span></div>`
      + switchRow('site-blocking', site)
      + '<div class="menu-separator" data-key="trees-separator"></div><div class="menu-heading" data-key="trees-title">Trees</div>'
      + item('menu-back', back ? `Back to ${back.label}` : 'Back', 'back', !!back)
      + item('menu-forward', forward ? `Forward to ${forward.label}` : 'Forward', 'forward', !!forward)
      + item('menu-new-tree', 'New tree', 'new-tree')
      + item('menu-forest', 'Forest', 'forest')
      + item('show-trees-help', 'How trees work', 'trees-help')
      + '<div class="menu-separator" data-key="settings-separator"></div><div class="menu-heading" data-key="title">Settings</div>'
      + '<div class="menu-heading" data-key="theme-title">Theme</div><div class="segmented" id="theme-mode" data-key="theme">'
      + ['dark', 'light', 'system'].map((mode) => `<button data-mode="${mode}" aria-pressed="${mode === theme}">${mode[0].toUpperCase()}${mode.slice(1)}</button>`).join('')
      + '</div><div class="menu-separator" data-key="capture-separator"></div>'
      + `<button class="menu-row" role="menuitem" id="menu-passwords" data-key="passwords" aria-haspopup="dialog"${S.passwordSettings ? '' : ' disabled'}><span class="grow">Passwords</span><span class="menu-detail">${esc(S.passwordSettings?.active ?? 'Unavailable')}</span></button>`
      + captureRow()
      + switchRow('blocking', blocking)
      + `<button class="menu-row" role="menuitem" id="recent-errors" data-key="errors"><span class="grow">Recent errors</span>${S.errors.unread ? '<i class="error-dot"></i>' : ''}</button>`
      + '<div class="menu-separator" data-key="quit-separator"></div>'
      + item('menu-quit', 'Quit', 'quit'));
    const act = (id, fn) => { $(id).onclick = fn; };
    act('menu-passwords', () => openPopover('passwords', $('menu-passwords')));
    act('menu-find', () => { closePopover(false); intent('find-open'); });
    act('menu-bookmark', () => { closePopover(false); intent('toggle-bookmark'); });
    act('menu-copy', () => { const url = visit(page)?.url; closePopover(false); if (url) post({ t: 'copy', text: url }); });
    act('menu-reload', () => { closePopover(false); uncover(); intent('reload'); });
    act('menu-zoom-out', () => intent('page-scale', { action: 'decrease' }));
    act('menu-zoom-reset', () => intent('page-scale', { action: 'reset' }));
    act('menu-zoom-in', () => intent('page-scale', { action: 'increase' }));
    act('menu-back', () => { closePopover(false); uncover(); intent('back'); });
    act('menu-forward', () => { closePopover(false); uncover(); intent('forward'); });
    act('menu-new-tree', () => { closePopover(false); window.canopy.newTree(); });
    act('menu-forest', () => { closePopover(false); window.canopy.toggleForest(); });
    act('menu-quit', () => { closePopover(false); intent('quit'); });
    $('recent-errors').onclick = () => { closePopover(false); openPopover('errors', $('settings-button')); post({ t: 'errors', action: 'read' }); };
    $('show-trees-help').onclick = () => { closePopover(false); intent('trees-help', { show: true }); };
    wireCaptureRow();
    if (blocking) act('blocking', () => intent('blocking', { on: !blocking.on }));
    if (site) act('site-blocking', () => intent('block-site', {
      on: !site.on, origin: site.origin, document: site.document,
    }));
    popover.querySelectorAll('[data-mode]').forEach((button) => button.onclick = () => {
      S.frame.theme = button.dataset.mode;
      applyTheme();
      intent('theme', { mode: button.dataset.mode });
      refreshPopover();
    });
  }
}

function switchRow(id, row) {
  if (!row) return '';
  return `<button class="menu-row" role="switch" aria-checked="${row.on}" id="${id}" data-key="${id}"><span class="grow">${esc(row.label)}</span>${row.detail ? `<span class="menu-detail">${esc(row.detail)}</span>` : ''}<span class="switch" aria-hidden="true"></span></button>`;
}

function captureRow() {
  const on = !!S.frame?.captureEnabled;
  return `<button class="menu-row" role="switch" aria-checked="${on}" id="automatic-capture" data-key="capture" title="Keep the text of opened pages on this computer. Form fields are never kept."><span class="grow">Remember pages</span><span class="switch" aria-hidden="true"></span></button>`;
}

function wireCaptureRow() {
  $('automatic-capture').onclick = () => {
    const on = !!S.frame?.captureEnabled;
    S.frame.captureEnabled = !on;
    intent('capture', { on: !on });
    refreshPopover();
    buildStrip(strip.target ?? targetLayout());
  };
}

// ---------- palette ----------

let paletteSeq = 0, paletteTimer = 0;

function openPalette({ scope, text, typed = false, fromChrome = true }) {
  intent('cancel');
  if (S.surface === 'start') scope = null;
  closePopover(false);
  rememberFocus('palette', fromChrome ? document.activeElement : document.body);
  post({ t: 'paletteScope', id: scope });
  S.palette = { scope, rows: [], selected: 0, seq: 0 };
  setLayer('palette');
  renderPaletteContext();
  $('palette-icon').innerHTML = icon('search');
  $('palette').hidden = false;
  const input = $('palette-input');
  input.value = text;
  input.focus();
  if (typed) input.setSelectionRange(text.length, text.length);
  else input.select();
  queryPalette();
  syncLayer();
}

function renderPaletteContext() {
  if (!S.palette) return;
  const v = S.palette.scope && visit(S.palette.scope);
  const content = v
    ? `From ${visitFav(v, 12)}<span></span>`
    : '<span class="tag">New tree</span>';
  const anchor = $('palette-context');
  if (anchor.__content !== content) {
    anchor.innerHTML = content;
    anchor.__content = content;
  }
  if (v && anchor.lastElementChild.textContent !== v.label) {
    anchor.lastElementChild.textContent = v.label;
  }
}

function closePalette(restoreFocus = true) {
  if (!S.palette) return;
  clearTimeout(paletteTimer);
  S.palette = null;
  post({ t: 'paletteScope', id: null });
  $('palette').hidden = true;
  $('palette-results').innerHTML = '';
  $('palette-input').removeAttribute('aria-activedescendant');
  setLayer(restingLayer());
  if (restoreFocus) returnFocus('palette');
  syncLayer();
}

function queryPalette() {
  if (!S.palette) return;
  paletteSeq += 1;
  S.palette.seq = paletteSeq;
  S.palette.asked = $('palette-input').value;
  post({ t: 'palette', q: S.palette.asked, seq: paletteSeq });
}

function renderPalette() {
  const { rows, selected } = S.palette;
  let group = null;
  $('palette-results').innerHTML = rows.map((row, i) => {
    const heading = row.group !== group ? `<div class="palette-group">${esc(row.group)}</div>` : '';
    group = row.group;
    const glyph = typeof row.icon === 'string' ? icon(row.icon) : fav(row.icon.url, row.icon.opened, 16);
    return heading + `<button class="palette-item" role="option" id="palette-option-${i}" aria-selected="${i === selected}" tabindex="-1" data-row="${i}" data-kind="${row.kind}">${glyph}`
      + `<span class="palette-title">${esc(row.title)}</span>`
      + (row.detail ? `<span class="palette-detail">${esc(row.detail)}</span>` : '') + '</button>';
  }).join('');
  const active = $('palette-results').querySelector('[aria-selected="true"]');
  if (active) $('palette-input').setAttribute('aria-activedescendant', active.id);
  else $('palette-input').removeAttribute('aria-activedescendant');
  active?.scrollIntoView({ block: 'nearest' });
}

function runPalette(index, newTree) {
  const palette = S.palette;
  if (!palette) return;
  const query = $('palette-input').value;
  // Rows still listed for earlier words say nothing about these: Rust runs their own default.
  const row = palette.shown === query ? palette.rows[index] : null;
  post({ t: 'run', q: query, item: row ? row.item : null, newTree, scope: palette.scope });
  const reading = row?.kind !== 'saved';
  closePalette(reading);
  if (reading) uncoverForPage();
}

$('palette-input').addEventListener('input', () => {
  clearTimeout(paletteTimer);
  paletteTimer = setTimeout(queryPalette, 40);
});
$('palette-input').addEventListener('keydown', (event) => {
  const palette = S.palette;
  if (!palette) return;
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    const delta = event.key === 'ArrowDown' ? 1 : -1;
    palette.selected = clamp(palette.selected + delta, 0, Math.max(palette.rows.length - 1, 0));
    renderPalette();
  } else if (event.key === 'Enter') {
    runPalette(palette.selected, event.altKey);
  } else if (event.key === 'Escape') {
    closePalette();
  } else {
    return;
  }
  event.preventDefault();
});
$('palette-results').addEventListener('click', (event) => {
  const row = event.target.closest('[data-row]');
  if (row) runPalette(Number(row.dataset.row), event.altKey);
});
$('palette').querySelector('.scrim').addEventListener('mousedown', () => closePalette());

// ---------- current-tree sheet ----------

function renderSheet(reveal) {
  if (S.overlay !== 'tree') return;
  const focused = document.activeElement?.closest('#tree-sheet [data-id]');
  const focusedIndex = focused ? [...focused.parentElement.children].indexOf(focused) : -1;
  const view = S.frame?.treeView;
  if (!view) { closeOverlay(); return; }
  $('tree-title').textContent = view.title;
  $('tree-detail').textContent = view.detail;
  $('tree-layer').style.width = `${view.width}px`;
  $('tree-layer').style.height = `${view.height}px`;
  $('tree-edges').setAttribute('width', view.width);
  $('tree-edges').setAttribute('height', view.height);
  $('tree-edges').innerHTML = view.edges.map((e) => `<path class="${e.kind}" d="M${e.c[0]} C${e.c[1]} ${e.c[2]} ${e.c[3]}"/>`).join('');
  const rows = new Map(view.rows.map((r) => [r.id, r]));
  for (const [container, items, tile] of [[$('tree-nodes'), view.tiles, true], [$('tree-list'), view.rows, false]]) {
    const kept = new Map([...container.children].map((el) => [el.dataset.id, el]));
    let cursor = container.firstElementChild;
    for (const item of items) {
      const r = rows.get(item.id), v = visit(item.id);
      const el = kept.get(item.id) || document.createElement('button');
      kept.delete(item.id);
      el.dataset.id = item.id;
      el.className = `${tile ? 'tree-tile' : 'tree-row'}${r.path ? ' path' : ''}${markClass(v)}`;
      el.title = v.label;
      el.setAttribute('aria-current', item.id === current() ? 'page' : 'false');
      el.setAttribute('role', 'treeitem');
      el.setAttribute('aria-label', `${v.name}${r.summary ? ', ' + r.summary : ''}`);
      el.setAttribute('aria-level', r.level);
      if (r.branch) el.setAttribute('aria-expanded', String(!r.folded));
      else el.removeAttribute('aria-expanded');
      el.style.cssText = tile
        ? `left:${item.x-item.w/2}px;top:${item.y-item.s/2}px;width:${item.w}px;height:${item.s}px`
        : `padding-left:${8+r.depth*22}px`;
      const html = `<span data-fold="${r.branch ? item.id : ''}" class="tree-fold">${r.branch ? (r.folded ? '▸' : '▾') : ''}</span>${markedFav(v, 16)}<span class="${tile ? 't' : 'grow'}">${esc(tile ? r.tileTitle : r.title)}${r.summary ? `<small>${esc(r.summary)}</small>` : ''}</span>${!tile ? `<span class="menu-detail">${esc(v.queued ? v.detail : v.when)}${v.live ? ' · Still open' : ''}</span>` : ''}`;
      if (el.dataset.html !== html) { el.innerHTML = html; el.dataset.html = html; }
      if (el !== cursor) container.insertBefore(el, cursor);
      cursor = el.nextElementSibling;
    }
    for (const el of kept.values()) el.remove();
  }
  $('tree-waiting').setAttribute('aria-pressed', String(view.waitingOnly));
  $('tree-empty').hidden = view.rows.length !== 0;
  if (document.activeElement !== $('tree-filter')) $('tree-filter').value = view.query;
  setTreeSide(S.treeSide, reveal);
  if (focused && !focused.isConnected) focusTree(view.rows[Math.min(focusedIndex, view.rows.length - 1)]?.id);
}

function setTreeSide(side, reveal = true, target = current()) {
  S.treeSide = side;
  $('tree-board').hidden = side !== 'tree';
  $('tree-list').hidden = side !== 'list';
  for (const button of $('tree-side').children) {
    const selected = button.dataset.side === side;
    button.setAttribute('aria-pressed', String(selected));
  }
  if (reveal) {
    const body = $(side === 'tree' ? 'tree-board' : 'tree-list');
    const selected = body.querySelector(`[data-id="${target}"]`);
    if (selected) {
      const a = selected.getBoundingClientRect(), b = body.getBoundingClientRect();
      body.scrollLeft += a.left - b.left - (b.width-a.width)/2;
      body.scrollTop += a.top - b.top - (b.height-a.height)/2;
    }
  }
}

$('tree-side').addEventListener('click', (event) => {
  const button = event.target.closest('[data-side]');
  if (button) { S.treePreference = button.dataset.side; setTreeSide(button.dataset.side); focusTree(current()); }
});
$('tree-sheet').addEventListener('click', (event) => {
  const page = event.target.closest('[data-id]');
  if (event.target.closest('[data-fold]')?.dataset.fold) post({ t: 'workbench', id: page.dataset.id });
  else if (page) activate(page.dataset.id);
});
$('tree-filter').addEventListener('input', () => post({ t: 'workbench', query: $('tree-filter').value }));
$('tree-waiting').addEventListener('click', () => post({ t: 'workbench', waiting: !S.frame.treeView.waitingOnly }));
$('tree-board').addEventListener('wheel', (event) => {
  if (event.ctrlKey || !event.shiftKey || event.deltaX || !event.deltaY) return;
  event.preventDefault();
  $('tree-board').scrollLeft += event.deltaY;
}, { passive: false });
function focusTree(id) {
  for (const el of $('tree-sheet').querySelectorAll('.keyboard-focus')) el.classList.remove('keyboard-focus');
  const body = $(S.treeSide === 'tree' ? 'tree-nodes' : 'tree-list');
  const el = body.querySelector(`[data-id="${id}"]`) || body.querySelector('[data-id]');
  (el || $('tree-filter')).focus({ preventScroll: true });
  el?.classList.add('keyboard-focus');
  el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}
$('tree-sheet').addEventListener('pointerdown', () => {
  for (const el of $('tree-sheet').querySelectorAll('.keyboard-focus')) el.classList.remove('keyboard-focus');
});
$('tree-sheet').addEventListener('keydown', (event) => {
  if (S.popover || event.ctrlKey || event.altKey || event.metaKey || event.isComposing) return;
  const field = event.target === $('tree-filter');
  const rows = S.frame.treeView.rows;
  const id = event.target.closest('[data-id]')?.dataset.id;
  const index = rows.findIndex((r) => r.id === id);
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    const next = field ? 0 : clamp(index + (event.key === 'ArrowDown' ? 1 : -1), 0, rows.length - 1);
    focusTree(rows[next]?.id);
  } else if (!field && ['ArrowLeft', 'ArrowRight'].includes(event.key)) {
    if (rows[index]?.branch) post({ t: 'workbench', id, folded: event.key === 'ArrowLeft' });
  } else if (!field && event.key === 'Enter' && id) activate(id);
  else if (!field && event.key === 'Delete' && id) {
    if (visit(id).nodeMenu.some((r) => r.kind === 'dismiss')) intent('dismiss', { id });
  } else if (!field && (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) && id) {
    openPopover('node', event.target.closest('[data-id]'), id);
  } else if (!field && event.key === ' ') {
    event.preventDefault();
    return;
  } else if (!field && event.key.length === 1) {
    $('tree-filter').focus();
    $('tree-filter').value += event.key;
    post({ t: 'workbench', query: $('tree-filter').value });
  } else if (tabKey(event)) {
    const controls = [...$('tree-sheet').querySelectorAll('button, input')].filter((el) => el.getClientRects().length);
    const index = controls.indexOf(document.activeElement);
    controls[(index + (event.shiftKey ? controls.length - 1 : 1)) % controls.length]?.focus();
  } else return;
  event.preventDefault();
});
$('tree-sheet').querySelector('.scrim').addEventListener('mousedown', closeOverlay);
$('zoom-pin').innerHTML = icon('expand') + icon('collapse');
$('zoom-pin').addEventListener('click', (event) => {
  toggleOverlay('tree');
  if (event.detail && S.overlay === 'tree') rememberFocus('overlay', document.body);
});

// ---------- surfaces ----------

const OVERLAYS = {
  tree: {
    el: 'tree-sheet', button: 'zoom-pin', leavesStart: true,
    available: () => !!S.frame?.treeView,
    show(reveal) {
      renderSheet(false);
      if (reveal) requestAnimationFrame(() => {
        if (S.overlay !== 'tree') return;
        const view = S.frame.treeView;
        setTreeSide(view.overflows ? 'list' : S.treePreference, true);
        post({ t: 'focus', target: 'chrome' });
        focusTree(current());
      });
    },
  },
  forest: {
    el: 'forest', button: 'forest-button',
    available: () => !!S.frame,
    show() { renderForest(); },
  },
};
const restingLayer = () => (S.palette || S.overlay || S.surface !== 'reading') ? 'palette' : 'none';
const pageShows = () => S.surface === 'reading' && !S.overlay;

function setSurface(surface) {
  if (S.surface === surface) return;
  S.surface = surface;
  S.startForNewTree = false;
  post({ t: 'surface', start: surface === 'start' });
  if (surface === 'start') { if (S.frame) S.frame.start = null; S.startPaint = null; $('start-content').replaceChildren(); if (S.overlay !== 'forest') focusStart(); }
  renderTabs();
  renderPlaceholder();
  $('start').hidden = surface !== 'start';
  setLayer(restingLayer());
  syncLayer();
}

function setOverlay(kind) {
  if (S.overlay === kind) return;
  const previous = OVERLAYS[S.overlay];
  if (S.overlay === 'tree') post({ t: 'workbench', query: '', waiting: false });
  if (previous) {
    $(previous.el).hidden = true;
    $(previous.button).setAttribute('aria-pressed', 'false');
  }
  if (kind && !S.overlay) rememberFocus('overlay');
  const forestChanged = (S.overlay === 'forest') !== (kind === 'forest');
  S.overlay = kind;

  document.body.dataset.overlay = kind ?? '';
  const overlay = OVERLAYS[kind];
  if (overlay) {
    if (overlay.leavesStart) setSurface('reading');
    hideList();
    closePopover(false);
    $(overlay.el).hidden = false;
    if (kind === 'forest') { $('forest').dataset.pending = ''; $('forest-canopy').hidden = S.forestWants !== 'canopy'; $('canopy-legend').hidden = S.forestWants !== 'canopy'; $('forest-history').hidden = S.forestWants !== 'history'; }
    $(overlay.button).setAttribute('aria-pressed', 'true');
    if (kind !== 'forest') overlay.show(true);
  }
  setLayer(restingLayer());
  syncLayer();
  if (forestChanged) {
    if (kind === 'forest') requestForest();
    else { resetCanopy(); S.forest = null; post({ t: 'forest', do: 'close' }); }
  }
}

function requestForest() {
  if (S.overlay === 'forest' && $('forest').hasAttribute('data-pending') && window.innerHeight >= S.windowHeight - 1) {
    post({ t: 'forest', do: 'open', view: S.forestWants, ...canopyRoom() });
  }
}
window.addEventListener('resize', requestForest);
window.addEventListener('resize', keepPopoverInside);

function canFocus(el) {
  if (!el?.isConnected || el.disabled || el.tabIndex < 0 || el.closest('[hidden], [inert]')) return false;
  const style = getComputedStyle(el), rect = el.getBoundingClientRect();
  return style.visibility === 'visible' && style.display !== 'none' && style.opacity !== '0'
    && rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.right > 0
    && rect.top < window.innerHeight && rect.left < window.innerWidth;
}

function closeOverlay() {
  setOverlay(null);
  returnFocus('overlay');
}

function toggleOverlay(kind) {
  if (S.overlay === kind) { closeOverlay(); return; }
  if (!OVERLAYS[kind].available()) return;
  intent('cancel');
  closePalette(false);
  setOverlay(kind);
}

function uncover() {
  closeReader(false);
  closePalette(false);
  closePopover(false);
  hideList();
  setOverlay(null);
  setSurface('reading');
}

// The new tree page stays up until the next frame says whether the page asked for opens a
// new tree: then the page stays until it commits, as a browser's new tab page does.
function uncoverForPage() {
  if (S.surface !== 'start') { uncover(); return; }
  S.leaveStart = true;
  closeReader(false);
  closePalette(false);
  closePopover(false);
  hideList();
  setOverlay(null);
}

function focusStart() {
  post({ t: 'focus', target: 'chrome' });
  $('start-search').focus({ preventScroll: true });
  $('start').scrollTop = 0;
}

function renderPlaceholder() {
  const start = S.surface === 'start';
  $('strip-placeholder').hidden = !start;
  $('strip-clip').hidden = start;
}

function bandHead(label, hint = '', mark = '') {
  return `<div class="band-head">${mark ? icon(mark) : ''}<span class="grow">${esc(label)}</span><span class="hint">${esc(hint)}</span></div>`;
}

function miniHtml(mini) {
  const edges = mini.edges.map((e) => `<path class="${e.wait ? 'wait' : ''}" d="M ${e.c[0]} C ${e.c[1]} ${e.c[2]} ${e.c[3]}"/>`).join('');
  const stub = mini.cut ? `<path class="wait" d="M -6 ${mini.s / 2} H 0"/>` : '';
  return `<span class="mt" style="width:${mini.w}px;height:${mini.h}px"><svg width="${mini.w}" height="${mini.h}">${edges}${stub}</svg>`
    + mini.nodes.map((n) => `<span class="mt-node${n.last ? ' last' : ''}${n.waiting ? ' waiting' : ''}" style="left:${n.x}px;top:${n.y}px">${fav(n.url ?? visit(n.id)?.url ?? '', !n.waiting, mini.s)}</span>`).join('') + '</span>';
}

const TREE_MENUS = {
  forest: (id) => forestMenu(id),
  tab: (id) => S.frame?.tabs.find((t) => t.id === id)?.menu,
  start: (id) => S.frame?.start?.shelf.find((c) => c.id === id)?.menu,
};

function treeCard(t, attrs) {
  return `<button class="tree-card" ${attrs} data-id="${t.id}" aria-label="${esc(t.name)}" title="${esc(t.tip)}">`
    + `<span class="pc-tree">${miniHtml(t.mini)}</span><span class="pc-foot"><span class="pc-name">${esc(t.title)}</span>`
    + (t.badge ? `<span class="badge">${esc(t.badge)}</span>` : '')
    + (t.waiting ? `<span class="count" title="${esc(t.waitingTip)}">${t.waiting}</span>` : '')
    + `</span>${t.page ? `<span class="pc-page">${esc(t.page)}</span>` : ''}<span class="pc-meta">${esc(t.meta)}</span></button>`;
}

function shelfBand(start) {
  return '<section>' + bandHead(start.shelfTitle, start.shelfHint)
    + (start.shelf.length ? '<div class="shelf-row">' + start.shelf.map((t) =>
      treeCard(t, 'data-start="tree"')).join('') + '</div>'
      : `<div class="hint-line">${esc(start.shelfHint)}</div>`) + '</section>';
}

function suggestedBand(start) {
  if (!start.suggested.length) return '';
  return '<section>' + bandHead('Suggested', 'From your history', 'spark') + '<div class="sg-row">' + start.suggested.map((s) => {
    const g = s.glyph;
    const glyph = s.kind === 'tree' ? miniHtml(g.mini) : s.kind === 'path' ? fav(g.first, true, 14) + '<i>→</i>' + fav(g.second, true, 14) : fav(g.url, true, 16);
    return `<div class="sg" style="grid-column:${s.slot + 1}"><button class="sg-main" data-start="suggestion" data-id="${esc(s.key)}" title="${esc(s.tip)}"><span class="sg-glyph ${s.kind}">${glyph}</span><span class="sg-text"><span class="sg-title">${esc(s.title)}</span><span class="sg-why">${esc(s.why)}</span></span></button><span class="sg-actions"><button class="sg-act" data-start="keep" data-id="${esc(s.key)}" title="Keep" aria-label="Keep">${icon('bookmark')}</button><button class="sg-act" data-start="decline" data-id="${esc(s.key)}" title="Stop suggesting this" aria-label="Stop suggesting this">${icon('close')}</button></span></div>`;
  }).join('') + '</div></section>';
}

function folderChip(f) {
  return `<button class="folder-chip" data-start="folder" data-id="${esc(f.name)}" title="${esc(f.name)}">${icon('folder')}<span>${esc(f.name)}</span><i>${f.count}</i></button>`;
}

function bookmarksBand(start) {
  const b = start.bookmarks;
  const chips = b.folders.map(folderChip).join('') + (b.hidden ? `<button class="folder-chip" data-start="folders">${esc(b.hidden.label)}</button>` : '');
  const tiles = b.tiles.map((r) => `<button class="start-tile" data-start="bookmark" data-id="${r.id}" title="${esc(r.label + '\n' + r.url)}"><span class="glyph">${fav(r.url, true, 24)}</span><span class="label">${esc(r.label)}</span></button>`).join('');
  const more = b.more ? `<button class="start-tile" data-start="more"><span class="glyph">${icon('chevron')}</span><span class="label">${esc(b.more.label)}</span></button>` : '';
  return '<section><div class="band-head"><span class="grow">Bookmarks</span>' + chips + '</div>'
    + (b.empty ? `<div class="hint-line bookmark-hint">${esc(S.keyboard?.bookmarkHint)}</div>` : `<div class="start-tiles">${tiles}${more}</div>`) + '</section>';
}

function togglePopover(kind, anchor, data) {
  if (S.popover?.kind === kind && S.popover.anchor === anchor) { closePopover(false); anchor.focus(); return; }
  const origin = anchor.closest('#popover') ? S.popover.anchor : anchor;
  openPopover(kind, origin, data);
  post({ t: 'focus', target: 'chrome' });
  $('popover').querySelector('button')?.focus();
}

const BANDS = [shelfBand, bookmarksBand, suggestedBand];
const START = {
  bookmark: { open: (id) => { closePopover(false); uncoverForPage(); intent('open-bookmark', { id: Number(id) }); }, menu: 'bookmark' },
  folder: { open: (name, el) => togglePopover('folder', el, { folder: name }) },
  more: { open: (_, el) => togglePopover('folder', el, { more: true }) },
  folders: { open: (_, el) => togglePopover('folder', el, { hidden: true }) },
  tree: { open: (id) => treeAct(id, 'open'), menu: 'tree' },
  suggestion: { open: (key) => { intent('open-suggestion', { key }); uncoverForPage(); } },
  keep: { open: (key) => intent('keep-suggestion', { key }) },
  decline: { open: (key) => intent('decline-suggestion', { key }) },
};

function helpDrawing(kind) {
  const page = { url: 'canopy://page', live: false, queued: false };
  if (kind === 'waiting') return markedFav({ ...page, queued: true }, 16);
  if (kind === 'unread') return markedFav({ ...page, queued: true, loaded: true }, 16);
  if (kind === 'live') return markedFav({ ...page, live: true }, 16);
  if (kind === 'leaf') return '<span class="leaf"></span>';
  if (kind === 'branches') return '<span class="branch-count help-count">3</span>';
  if (kind === 'settle') return icon('close');
  return miniHtml({ w: 62, h: 22, s: 14, cut: false,
    nodes: [{ x: 0, y: 4, url: page.url }, { x: 40, y: 4, url: page.url, last: true }],
    edges: [{ c: ['14 11', '25 11', '25 11', '40 11'] }] });
}

function renderHelp() {
  const box = $('trees-help');
  box.hidden = !S.frame?.treesHelp;
  const help = S.keyboard?.help;
  if (!help || box.__paint === help) return;
  if (box.__paint) {
    box.querySelectorAll('li > span:last-child').forEach((el, index) => {
      el.textContent = help.lines[index].text;
    });
    box.__paint = help;
    return;
  }
  box.__paint = help;
  box.innerHTML = `<div class="help-head"><h2 id="trees-help-title">${esc(help.title)}</h2><button id="trees-help-dismiss" class="action">Got it</button></div>`
    + '<ol>' + help.lines.map((line) => `<li><span class="help-drawing" aria-hidden="true">${helpDrawing(line.mark)}</span><span>${esc(line.text)}</span></li>`).join('') + '</ol>'
    + '<div class="help-legend" aria-label="Page marks">' + help.legend.map((group) => `<span class="help-legend-group">${group.map((item) => `<span class="help-legend-mark"><i aria-hidden="true">${helpDrawing(item.mark)}</i>${esc(item.text)}</span>`).join('')}</span>`).join('') + '</div>';
  $('trees-help-dismiss').onclick = () => { intent('trees-help', { show: false }); focusStart(); };
}

function renderStart() {
  renderHelp();
  if (!S.frame?.start || S.surface !== 'start') return;
  const paint = JSON.stringify(S.frame.start);
  if (S.startPaint === paint) return;
  S.startPaint = paint;
  const focused = document.activeElement?.closest('[data-start]');
  const anchor = S.popover?.anchor.closest('[data-start]');
  const find = (el) => el && [...$('start-content').querySelectorAll('[data-start]')].find((n) => n.dataset.start === el.dataset.start && n.dataset.id === el.dataset.id);
  $('start-content').innerHTML = BANDS.map((band) => band(S.frame.start)).join('');
  if (focused) { const next = find(focused); if (next) next.focus(); else focusStart(); }
  if (anchor) {
    const next = find(anchor);
    if (next) {
      S.popover.anchor = next;
      setTooltip(next, next.getAttribute('title'));
    } else closePopover(false);
  }
}

// ---------- reader ----------

function showReader(html, list) {
  if (!S.reader) rememberFocus('reader');
  closePalette(false);
  closePopover(false);
  hideList();
  const box = $('reader-box');
  box.className = `reader-box${list ? ' list' : ''}`;
  box.innerHTML = html + `<button class="icon-button reader-close" aria-label="Close">${icon('close')}</button>`;
  box.querySelector('.reader-close').addEventListener('click', closeReader);
  S.reader = true;
  $('reader').hidden = false;
  post({ t: 'focus', target: 'chrome' });
  box.querySelector('.reader-close').focus();
  syncLayer();
}

function closeReader(restoreFocus = true) {
  if (!S.reader) return;
  S.reader = null;
  $('reader').hidden = true;
  if (restoreFocus) returnFocus('reader');
  syncLayer();
}
$('reader').querySelector('.scrim').addEventListener('mousedown', closeReader);

const when = (at) => new Date(at).toLocaleString(undefined, { day: '2-digit', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });

// ---------- find bar ----------

/// The count shows only for the query in the field; a newer edit dims the last one.
function renderFind() {
  const find = S.find;
  $('find-bar').hidden = !find;
  document.body.toggleAttribute('data-find', !!find);
  if (!find) return;
  const state = find.query === $('find-input').value ? find.matches.state : 'pending';
  $('find-bar').dataset.state = state;
  const count = $('find-count');
  if (state === 'position') count.textContent = `${find.matches.index} of ${find.matches.total}`;
  else if (state === 'none') count.textContent = 'No matches';
  else if (state === 'empty') count.textContent = '';
  for (const id of ['find-previous', 'find-next']) $(id).disabled = state === 'empty' || state === 'none';
}

const findIntent = (action, extra = {}) => { if (S.find) intent('find', { session: S.find.id, action, ...extra }); };

// ---------- Rust's side ----------

function renderPageScale() {
  const badge = $('page-scale');
  const value = S.pageScale;
  const shown = value && value.page === S.frame?.current && value.percent !== 100;
  const changed = badge.hidden === !!shown;
  badge.hidden = !shown;
  if (shown) badge.textContent = `${value.percent}%`;
  if (changed) fitTabs();
}

let nextMoveRequest = 0;

window.canopy = {
  navigated() { uncover(); },
  branchMoveChoices(reply) {
    if (pagePress?.request === reply.request && pagePress.head === reply.head) {
      pagePress.choices = { parents: new Set(reply.parents), trees: new Set(reply.trees.map(tree => tree.id)), ownTree: reply.ownTree, warning: reply.warning };
      return;
    }
    const picker = S.popover;
    if (picker?.kind !== 'move-tree' || picker.data.request !== reply.request || picker.data.head !== reply.head) return;
    picker.data.choices = reply;
    renderPopover();
    keepPopoverInside();
    controlsIn($('popover'))[0]?.focus();
    syncLayer();
  },
  pageScale(value) { S.pageScale = value; renderPageScale(); if (S.popover?.kind === 'settings') refreshPopover(); },
  viewport(height) { S.windowHeight = height; document.documentElement.style.setProperty('--window-height', `${height}px`); positionList(); syncLayer(); },
  start(value, initial = false) {
    if (!S.frame || S.surface !== 'start') return;
    S.frame.start = value.start;
    renderStart();
    if (initial) requestAnimationFrame(() => requestAnimationFrame(() => { if (S.surface === 'start') post({ t: 'suggestions' }); }));
  },
  forest(value) {
    if (S.overlay !== 'forest') return;
    if (value.view === 'canopy' && !acceptCanopy(value.canopy)) return;
    const first = !S.forest || S.forest.view !== value.view;
    if (first) resetCanopy();
    delete $('forest').dataset.pending;
    S.forest = value;
    $('forest').hidden = false;
    renderForest();
    if (first) {
      focusForest();
      if (value.view === 'canopy') playEntrance(nodeOfPage(current())?.el, 'forest-locate');
    }
    if (S.popover?.data?.source === 'forest') refreshPopover();
  },
  loading(value) {
    const previous = S.loading;
    S.loading = value;
    paintLoading();
    if (previous?.page !== value.page || previous?.state !== value.state) {
      rebuild('loadingAction', renderLoadingAction, []);
    }
  },
  present(frame) {
    const previous = current();
    const newChild = previous && frame.visits[frame.current]?.parent === previous && !visit(frame.current);
    const previousTree = S.frame?.tabs.find((tab) => tab.active)?.id;
    const previousNewTree = S.frame?.newTree?.url ?? null;
    const previousCommitted = S.frame?.committed ?? null;
    S.frame = { ...frame, start: frame.start ?? S.frame?.start };
    visibleMailAccounts(S.frame.mail);
    applyStrip(frame.strip);
    paintLoading();
    renderPlayingPin();
    renderMailPin();
    applyTheme();
    // Overlays stay over the old page until the new one commits, as they always have.
    if (frame.committed !== previousCommitted && !['tree', 'forest'].includes(S.overlay)) setOverlay(null);
    if (frame.current !== previous) {
      hideList();
      const sameTree = previousTree && previousTree === frame.tabs.find((tab) => tab.active)?.id;
      if (S.surface !== 'reading' && !(S.surface === 'start' && sameTree)) setSurface('reading');
    }
    if (S.leaveStart) {
      S.leaveStart = false;
      if (!frame.newTree && S.surface === 'start') setSurface('reading');
    }
    // A new tree is shown from the moment it is asked for, on the new tree page. If the
    // reader came from a page, a load that ends without a page takes them back to it.
    if (frame.newTree && frame.newTree.url !== previousNewTree && S.surface !== 'start') {
      setSurface('start');
      S.startForNewTree = true;
    }
    if (!frame.newTree && S.startForNewTree) {
      S.startForNewTree = false;
      if (S.surface === 'start' && current()) setSurface('reading');
    }
    if (!frame.tabs.length && S.surface === 'reading') setSurface('start');
    if (S.popover?.kind === 'rename' && !TREE_MENUS[S.popover.data.source](S.popover.data.id)) closePopover();
    renderTabs();
    renderPageScale();
    if (newChild) animateStrip(160, frame.current);
    else animateStrip(previous === null && strip.shown.nodes.size === 0 ? 0 : MOVE);
    renderStart();
    renderPlaceholder();
    renderForest();
    renderSheet(false);
    if (S.list) { renderList(); requestListPreview(); }
    if (S.popover) refreshPopover();
    flushQueuedPulses();
    renderPaletteContext();
    syncLayer();
  },
  tabsFit(layout) {
    if (S.frame) S.frame.tabFit = layout;
    for (const item of layout.items) {
      const tab = [...$('tab-row').children].find((el) => el.dataset.id === item.id);
      if (!tab) continue;
      tab.hidden = item.mode === 'hidden';
      tab.classList.toggle('icon-only', item.mode === 'icon');
      tab.classList.toggle('short', item.short);
      tab.style.width = `${item.width}px`;
    }
    $('tab-overflow').hidden = !layout.overflow;
    const tabs = [...$('tab-row').children];
    if (!tabs.some((tab) => !tab.hidden && tab.tabIndex === 0)) {
      const stop = tabs.find((tab) => !tab.hidden && tab.getAttribute('aria-selected') === 'true') ?? tabs.find((tab) => !tab.hidden);
      for (const tab of tabs) tab.tabIndex = tab === stop ? 0 : -1;
    }
    if (S.popover?.kind === 'trees') refreshPopover();
  },
  strip(layouts) {
    applyStrip(layouts);
    renderPlayingPin();
    renderMailPin();
    animateStrip(0);
    fitTabs();
  },
  icons(url, revision, mailSites) {
    if (mailSites) {
      S.mailIconRevision += 1;
      for (const img of document.querySelectorAll('.mail-tile img')) {
        const src = new URL(img.src);
        if (!mailSites.includes(src.searchParams.get('u'))) continue;
        src.searchParams.set('r', S.mailIconRevision);
        img.src = src.href;
      }
      return;
    }
    S.revisions.set(iconSite(url), revision);
    for (const img of document.querySelectorAll('.fav img')) {
      const src = new URL(img.src);
      if (iconSite(src.searchParams.get('u')) !== iconSite(url)) continue;
      src.searchParams.set('r', revision);
      img.src = src.href;
    }
  },
  logins(offer) {
    S.logins = offer.names.length ? offer : null;
    renderLoginChip();
  },
  passwordSettings(value) {
    S.passwordSettings = value;
    if (['passwords', 'settings'].includes(S.popover?.kind)) refreshPopover();
  },
  loginsLocked(keeper) {
    S.locked = keeper;
    renderLoginChip();
  },
  saveLogin(offer) {
    S.save = offer;
    renderLoginChip();
    if (S.popover?.kind === 'save-login') refreshPopover();
  },
  fillLogin() { fillLogin(); },
  switcher(action, value) {
    if (action === 'show') { showSwitcher(value); return; }
    if (S.switcher?.opening !== value.opening) return;
    if (action === 'select') selectSwitcher(value.selected);
    else if (action === 'patch') rebuild('switcher', patchSwitcher, [value]);
    else if (action === 'layout') layoutSwitcher(value.layout);
    else if (action === 'picture') fillSwitcherPicture(value);
    else if (action === 'close') closeSwitcher(value.restoreFocus);
  },
  openPalette(options) { openPalette(options); },
  shortcuts(value) {
    S.keyboard = value;
    for (const el of document.querySelectorAll('[data-shortcut-title]')) {
      el.dataset.baseTitle ??= el.title;
      setTooltip(el, shortcutTitle(el.dataset.baseTitle, el.dataset.shortcutTitle));
    }
    for (const el of document.querySelectorAll('[data-shortcut-chip]')) {
      el.textContent = shortcut(el.dataset.shortcutChip, 'chip');
      el.hidden = !el.textContent;
    }
    for (const el of document.querySelectorAll('[data-switcher-key]')) {
      el.textContent = value.switcher[el.dataset.switcherKey];
    }
    for (const el of document.querySelectorAll('[data-shortcut-text]')) {
      el.textContent = shortcut(el.dataset.shortcutText);
    }
    renderHelp();
    if (!S.frame) return;
    renderTabs();
    renderNav();
    renderLoginChip();
    for (const node of strip.target?.nodes ?? []) {
      const el = strip.elements.get(node.id);
      if (el && node.tip) setTooltip(el, stripTip(node.tip));
    }
    if (S.list) renderList();
    if (['settings', 'waiting', 'memory'].includes(S.popover?.kind)) refreshPopover();
    const bookmarkHint = $('start-content').querySelector('.bookmark-hint');
    if (bookmarkHint) bookmarkHint.textContent = value.bookmarkHint;
    const toast = $('toast');
    if (!toast.hidden && toast.classList.contains('settled')) {
      toast.lastChild.textContent = undoShortcut();
      $('toast-status').textContent = toast.textContent;
    }
  },
  openErrors() { openPopover('errors', $('settings-button')); },
  showTreesHelp() { closeReader(false); closePopover(false); hideList(); this.newTree(); renderHelp(); },
  newTree() {
    cancel('toast');
    $('toast').hidden = true;
    intent('cancel');
    closePalette(false);
    setOverlay(null);
    setSurface('start');
    focusStart();
    syncLayer();
  },
  toggleForest(view = 'canopy', fromChrome = true) { toggleForest(view, fromChrome); },
  workbench(view) {
    if (!S.frame) return;
    const id = document.activeElement?.closest('[data-id]')?.dataset.id;
    S.frame.treeView = view;
    renderSheet(false);
    if (id && !document.activeElement?.closest('[data-id]')) focusTree(id);
    syncLayer();
  },
  tabSelected() { closePopover(false); setOverlay(null); },
  toggleTree(fromChrome = true) { toggleOverlay('tree'); if (!fromChrome && S.overlay === 'tree') rememberFocus('overlay', document.body); },
  errors(value) {
    S.errors = value;
    $('settings-button').classList.toggle('has-errors', value.unread);
    $('settings-button').setAttribute('aria-label', value.unread ? 'Settings · unread errors' : 'Settings');
    if (['errors', 'settings'].includes(S.popover?.kind)) refreshPopover();
    if (S.popover?.kind === 'errors' && value.unread) post({ t: 'errors', action: 'read' });
  },
  waiting(text, id) {
    if (text != null) this.toast('info', text);
    if (id == null) return;
    queuedPulses.add(id);
    flushQueuedPulses();
  },
  toast(kind, text, deleteId) {
    const toast = $('toast');
    $('toast-status').textContent = text + (kind === 'settled' ? ` · Undo${undoShortcut()}` : kind === 'deleted' ? ' · Undo' : '');
    toast.textContent = text;
    if (kind === 'settled' || kind === 'deleted') {
      const button = document.createElement('button');
      button.textContent = 'Undo';
      toast.appendChild(document.createTextNode(' · '));
      button.onclick = () => {
        toast.hidden = true; syncLayer();
        if (kind === 'settled') intent('undo-settle'); else intent('undo-delete', { id: deleteId });
      };
      toast.appendChild(button);
      if (kind === 'settled') toast.appendChild(document.createTextNode(undoShortcut()));
    }
    toast.className = kind;
    toast.hidden = false;
    syncLayer();
    clearTimeout(timers.toast);
    timers.toast = setTimeout(() => {
      toast.classList.add('leaving');
      timers.toast = setTimeout(() => { toast.hidden = true; syncLayer(); }, 200);
    }, kind === 'settled' || kind === 'deleted' ? 6000 : 2600);
  },
  toggleFocus() {
    inputOrigin = 'keyboard';
    const active = document.activeElement;
    if (document.hasFocus() && active?.closest('#tabs, #strip, #strip-list, #pins')) {
      hideList();
      post({ t: 'focus', target: 'page' });
      return;
    }
    const pill = strip.target?.nodes.find((n) => n.kind === 'pill');
    const el = pill && strip.elements.get(pill.id);
    if (!el) return;
    post({ t: 'focus', target: 'chrome' });
    el.focus();
  },
  /// Closes the topmost chrome layer.
  formReturn(open) {
    if (open && !S.formReturn) rememberFocus('formReturn');
    S.formReturn = open;
    $('form-return').hidden = !open;
    syncLayer();
    if (open) $('form-return-cancel').focus();
  },
  escape(surface) {
    switch (surface) {
      case 'formReturn': answerFormReturn(false); break;
      case 'reader': closeReader(); break;
      case 'palette': closePalette(); break;
      case 'popover': closePopover(); break;
      case 'list': { const key = S.list.key; strip.elements.get(key)?.focus(); hideList(); break; }
      case 'treeFilters':
        $('tree-filter').value = '';
        post({ t: 'workbench', query: '', waiting: false });
        break;
      case 'overlay': closeOverlay(); break;
      case 'start':
        // Leaving a new tree that is still loading gives it up.
        if (S.frame?.newTree) intent('stop');
        setSurface('reading');
        post({ t: 'focus', target: 'page' });
        break;
    }
  },
  paletteResults(seq, { rows, first }) {
    if (!S.palette || seq !== S.palette.seq) return;
    S.palette.shown = S.palette.asked;
    S.palette.rows = rows;
    S.palette.selected = Math.max(rows.findIndex((row) => row.item === first), 0);
    renderPalette();
  },
  previewResult(seq, preview) {
    if (seq === previewSeq && S.list && listItem()?.page === preview.id) { S.list.preview = preview; renderList(); }
  },
  reader(c) {
    const source = c.source ? `<div class="reader-source" id="capture-source">${esc(`From ${c.source.url}\n${c.source.linkText}\n${c.source.excerpt}`.trim())}</div>` : '';
    showReader(`<div class="reader-title">${esc(c.title)}</div>`
      + `<div class="reader-meta">${c.kind} remembered on ${esc(when(c.at))} · ${esc(c.url)}</div>${source}`
      + '<div class="card-actions" style="margin-top:0">'
      + '<button class="action" data-act="live">Open the page</button>'
      + `<button class="action" data-act="source" id="capture-restore-source"${c.parent ? '' : ' disabled'}>${esc(c.returnLabel)}</button>`
      + `<button class="action" data-act="citation"${c.citation ? '' : ' disabled'}>Open source comment</button></div>`
      + `<div class="reader-text" id="capture-text">${esc(c.text)}</div>`, false);
    $('reader-box').querySelectorAll('[data-act]').forEach((button) => button.addEventListener('click', () => {
      const act = button.dataset.act;
      closeReader();
      if (act === 'live') intent('open-saved', { capture: c.capture });
      else if (act === 'source') activate(c.parent);
      else intent('open-citation', { id: c.visit });
    }));
  },
  readerList(captures) {
    showReader('<div class="reader-title">Saved text for this page</div><div class="reader-list">'
      + captures.map((c) => `<button class="menu-row" data-capture="${c.capture}"><span class="grow">${esc(`${when(c.at)} · ${c.title}`)}</span></button>`).join('')
      + '</div>', true);
    $('reader-box').querySelectorAll('[data-capture]').forEach((row) => row.addEventListener('click', () => {
      intent('read-capture', { capture: Number(row.dataset.capture) });
    }));
  },
  systemDark(dark) {
    S.systemDark = dark;
    applyTheme();
  },
  /// Shows a find session, or with null closes the bar: 'user' closed it and Rust gave
  /// the page the keyboard, 'context' means the page changed under it.
  find(value, kind) {
    const input = $('find-input');
    if (!value) {
      const focused = $('find-bar').contains(document.activeElement);
      S.find = null;
      renderFind();
      fitTabs();
      if (kind === 'context' && focused) post({ t: 'focus', target: 'page' });
      return;
    }
    if (kind === 'focus') {
      uncover();
      if (S.find?.id !== value.id) input.value = value.query;
    }
    const opening = !S.find;
    S.find = value;
    renderFind();
    if (opening) fitTabs();
    if (kind === 'focus') {
      input.focus();
      input.select();
    }
  },
  /// For tests: what the view shows, without reaching into the DOM. `deferred` names the
  /// calls held back while a button is down.
  state() {
    return {
      mail: S.frame?.mail ?? null,
      loading: S.loading,
      buttons: refresh.buttons, start: S.frame?.start, deferred: [...refresh.pending.keys()],
      surface: S.surface, overlay: S.overlay, treeSide: S.treeSide, animating: !!strip.anim, list: S.list?.key ?? null, layer: S.layer, palette: !!S.palette,
      pageScale: S.pageScale,
      reader: !!S.reader, popover: S.popover?.kind ?? null, reported: S.reported, logins: loginNames(), save: S.save, locked: S.locked,
      dark: document.documentElement.classList.contains('dark'),
      nodes: [...strip.elements.keys()], rows: S.palette?.rows ?? [],
      waiting: S.frame?.waiting.ids ?? [], pins: (S.strip?.pins ?? []).map((p) => p.kind), room: S.strip?.room ?? null,
      find: S.find && { ...S.find, input: $('find-input').value, count: $('find-count').textContent, state: $('find-bar').dataset.state },
    };
  },
};

const queuedPulses = new Set();
function flushQueuedPulses() {
  if (!queuedPulses.size || refresh.buttons || refresh.painting || nativeDrag) return;
  const targets = new Set();
  const waiting = new Set(S.frame?.waiting.ids);
  for (const id of queuedPulses) {
    if (!waiting.has(id)) continue;
    const target = strip.target?.nodes.find(node => node.kind === 'tab' && node.id === id)
      ?? strip.target?.nodes.find(node => node.kind === 'overflow');
    if (target) targets.add(strip.elements.get(target.id));
  }
  queuedPulses.clear();
  for (const el of targets) {
    playEntrance(el, 'waiting-pulse');
    el.onanimationend = () => el.classList.remove('waiting-pulse');
  }
}

// A release must finish dispatching its click before any pressed element is replaced.
const refresh = { buttons: 0, pending: new Map(), release: null, painting: false };
function rebuild(kind, apply, args) {
  if (!refresh.buttons && !nativeDrag) { apply(...args); return; }
  const key = kind === 'icons' ? `icons:${args[0]}` : kind;
  refresh.pending.delete(key);
  refresh.pending.set(key, { apply, args });
}
function releaseRefresh() {
  clearTimeout(refresh.release);
  refresh.release = setTimeout(() => {
    refresh.buttons = 0;
    if (nativeDrag) return;
    const pending = [...refresh.pending.values()];
    refresh.pending.clear();
    refresh.painting = true;
    try { for (const { apply, args } of pending) apply(...args); }
    finally { refresh.painting = false; }
    flushQueuedPulses();
  });
}
for (const kind of ['present', 'strip', 'icons', 'start', 'forest', 'logins', 'loginsLocked', 'saveLogin', 'paletteResults', 'previewResult', 'reader', 'readerList']) {
  const apply = window.canopy[kind].bind(window.canopy);
  window.canopy[kind] = (...args) => rebuild(kind, apply, args);
}
for (const type of ['mousedown', 'mouseup', 'mousemove', 'mouseover']) {
  window.addEventListener(type, (event) => {
    if (event.buttons) {
      clearTimeout(refresh.release);
      refresh.buttons = event.buttons;
    } else if (refresh.buttons) releaseRefresh();
  }, true);
}
window.addEventListener('blur', () => { releaseRefresh(); if (!nativeDrag) hideList(); });
window.addEventListener('mouseup', () => { if (!nativeDrag) pagePress = null; }, true);
document.addEventListener('focusout', (event) => {
  if (!S.list || S.popover) return;
  const next = event.relatedTarget;
  if (!next || (!next.closest('#strip-list') && next.closest('.node')?.dataset.id !== S.list.key)) hideList();
});
document.addEventListener('visibilitychange', () => {
  if (document.hidden) releaseRefresh();
});

// ---------- static wiring ----------

$('start-search').insertAdjacentHTML('afterbegin', icon('search'));
$('start-search').addEventListener('click', () => openPalette({ scope: null, text: '' }));
for (const container of [$('start-content'), $('popover')]) {
  container.addEventListener('click', (event) => {
    const el = event.target.closest('[data-start]');
    if (el) START[el.dataset.start]?.open(el.dataset.id, el);
  });
  container.addEventListener('contextmenu', (event) => {
    const el = event.target.closest('[data-start]');
    const menu = el && START[el.dataset.start]?.menu;
    if (!menu) return;
    event.preventDefault();
    openPopover(menu, el, menu === 'tree' ? { source: 'start', id: el.dataset.id } : el.dataset.id);
    post({ t: 'focus', target: 'chrome' });
    $('popover').querySelector('button')?.focus();
  });
}
document.addEventListener('keydown', (event) => {
  if (S.surface !== 'start' || S.palette || S.popover || S.reader || S.overlay
      || event.ctrlKey || event.altKey || event.metaKey || event.isComposing
      || event.target.closest('input, textarea, [contenteditable]') || [...event.key].length !== 1 || event.key === ' ') return;
  event.preventDefault();
  openPalette({ scope: null, text: event.key, typed: true });
});

$('new-tree').innerHTML = icon('plus');
$('nav-back').innerHTML = icon('back');
$('nav-forward').innerHTML = icon('forward');
$('new-tree-tab').querySelector('.mk').innerHTML = icon('forest');
$('new-tree-close').innerHTML = icon('close');
$('forest-button').innerHTML = icon('forest') + '<span>Canopy</span>';
$('settings-button').innerHTML = icon('settings');
$('tab-overflow').innerHTML = icon('chevron');
$('search-pill').insertAdjacentHTML('afterbegin', icon('search'));
$('find-previous').innerHTML = icon('chevron');
$('find-next').innerHTML = icon('chevron');
$('find-close').innerHTML = icon('close');
$('find-input').addEventListener('input', () => {
  findIntent('query', { text: $('find-input').value });
  renderFind();
});
$('find-input').addEventListener('keydown', (event) => {
  if (event.key !== 'Enter') return;
  event.preventDefault();
  findIntent(event.shiftKey ? 'previous' : 'next');
});
// Buttons leave the keyboard in the field.
$('find-bar').addEventListener('mousedown', (event) => { if (event.target.closest('button')) event.preventDefault(); });
$('find-previous').addEventListener('click', () => findIntent('previous'));
$('find-next').addEventListener('click', () => findIntent('next'));
$('find-close').addEventListener('click', () => findIntent('close'));

const treeDragType = 'application/x-canopy-tree';
const pageDragType = 'application/x-canopy-page';
const PAGE_DRAG_NODES = '#strip-nodes .node[data-page], #strip-list .bl-head[data-page], #strip-list .menu-row[data-id]';
let pagePress = null;
let nativeDrag = null;

function pageElement(target) {
  if (S.overlay || S.popover || S.palette || S.reader || S.formReturn
      || target.closest('[data-act], [data-files], .leaves, input, textarea, [contenteditable]')) return null;
  const el = target.closest(PAGE_DRAG_NODES);
  return el?.getClientRects().length ? el : null;
}

document.addEventListener('mousedown', event => {
  if (nativeDrag) return;
  pagePress = null;
  const source = event.button === 0 && pageElement(event.target);
  if (!source) return;
  const head = source.dataset.page || source.dataset.id;
  pagePress = { kind: 'page', head, source, request: ++nextMoveRequest, choices: null };
  post({ t: 'branch-move-choices', request: pagePress.request, head });
}, true);

function clearDragCue() {
  $('tab-row').classList.remove('drop-target');
  $('tab-row').style.removeProperty('--tab-drop-x');
  for (const el of document.querySelectorAll('.page-drop-allowed, .page-drop-refused')) el.classList.remove('page-drop-allowed', 'page-drop-refused');
  $('page-drag-warning').hidden = true;
}

function finishNativeDrag() {
  clearDragCue();
  for (const el of document.querySelectorAll('.dragging')) el.classList.remove('dragging');
  nativeDrag?.image?.remove();
  nativeDrag = null;
  pagePress = null;
  cancel('showList');
  cancel('hideList');
  // Native drag can finish outside chrome without delivering mouseup.
  releaseRefresh();
}

function tabDropAt(x, source) {
  const row = $('tab-row');
  const visible = [...row.children].filter(tab => !tab.hidden);
  const before = visible.find(tab => x < tab.getBoundingClientRect().left + tab.offsetWidth / 2);
  const anchor = before ?? visible[visible.length - 1];
  if (!anchor) return null;
  const r = anchor.getBoundingClientRect();
  const tabs = S.frame.tabs;
  const gap = tabs.findIndex(tab => tab.id === anchor.dataset.id) + (before ? 0 : 1);
  return {
    before: tabs.slice(gap).find(tab => tab.id !== source)?.id ?? null,
    x: (before ? r.left - 1 : r.right + 1) - row.getBoundingClientRect().left,
  };
}

function paintGap(drop) {
  $('tab-row').style.setProperty('--tab-drop-x', `${drop.x}px`);
  $('tab-row').classList.add('drop-target');
}

function pageDropAt(event) {
  const choices = nativeDrag.choices;
  const tab = event.target.closest('#tab-row .tab');
  if (tab) return { element: tab, to: choices?.trees.has(tab.dataset.id) ? { kind: 'tree', id: tab.dataset.id } : null };
  const page = pageElement(event.target);
  if (page) {
    const id = page.dataset.page || page.dataset.id;
    return { element: page, to: choices?.parents.has(id) ? { kind: 'page', id } : null };
  }
  // Aggregates and controls must never fall through to the empty-strip target.
  const blankStrip = ['strip', 'strip-bg', 'strip-clip', 'strip-canvas', 'strip-nodes', 'strip-edges', 'glow'].includes(event.target.id)
    || !!event.target.closest('#strip-edges');
  const blankTabs = event.target === $('tab-row');
  if (!blankStrip && !blankTabs) return { element: event.target.closest('.node, button, #strip-list'), to: null };
  if (!choices?.ownTree) return { element: blankTabs ? $('tab-row') : $('strip-bg'), to: null };
  const reorder = choices.ownTree.kind === 'reorder';
  const alongTabs = blankTabs && reorder;
  const gap = tabDropAt(alongTabs ? event.clientX : Infinity, choices.ownTree.tree);
  const place = alongTabs && gap?.before ? { kind: 'before', tree: gap.before } : { kind: 'end' };
  return { to: { kind: 'own-tree', place }, gap };
}

function localDrag(event) {
  return nativeDrag && [...event.dataTransfer.types].includes(nativeDrag.kind === 'tree' ? treeDragType : pageDragType);
}

document.addEventListener('dragstart', event => {
  const tab = event.target.closest('#tab-row .tab');
  const page = pageElement(event.target);
  if (tab && !event.target.closest('[data-settle]')) {
    nativeDrag = { kind: 'tree', tree: tab.dataset.id, source: tab };
    pagePress = null;
    event.dataTransfer.setData(treeDragType, nativeDrag.tree);
  } else if (page && pagePress?.source === page) {
    nativeDrag = pagePress;
    const image = document.createElement('div');
    image.className = 'page-drag-image';
    image.textContent = visit(nativeDrag.head).label;
    document.body.appendChild(image);
    nativeDrag.image = image;
    event.dataTransfer.setDragImage(image, 16, 16);
    event.dataTransfer.setData(pageDragType, JSON.stringify({ head: nativeDrag.head, gesture: nativeDrag.request }));
  } else {
    event.preventDefault();
    return;
  }
  crossPress = null;
  cancel('showList');
  cancel('hideList');
  previewSeq += 1;
  clearTimeout(refresh.release);
  event.dataTransfer.effectAllowed = 'move';
  const drag = nativeDrag;
  requestAnimationFrame(() => { if (nativeDrag === drag) drag.source.classList.add('dragging'); });
});

function nativeDragOver(event) {
  if (!localDrag(event)) return;
  clearDragCue();
  event.stopPropagation();
  if (nativeDrag.kind === 'tree') {
    if (!event.target.closest('#tab-row')) { event.dataTransfer.dropEffect = 'none'; return; }
    const drop = tabDropAt(event.clientX, nativeDrag.tree);
    if (!drop) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = 'move';
    paintGap(drop);
    return;
  }
  const drop = pageDropAt(event);
  event.dataTransfer.dropEffect = drop.to ? 'move' : 'none';
  drop.element?.classList.add(drop.to ? 'page-drop-allowed' : 'page-drop-refused');
  if (!drop.to) return;
  event.preventDefault();
  if (drop.gap) paintGap(drop.gap);
  if (nativeDrag.choices.warning && drop.to.kind !== 'own-tree') {
    $('page-drag-warning').textContent = nativeDrag.choices.warning;
    $('page-drag-warning').hidden = false;
  }
}
document.addEventListener('dragenter', nativeDragOver);
document.addEventListener('dragover', nativeDragOver);
document.addEventListener('dragleave', event => {
  if (!nativeDrag) return;
  if (!event.relatedTarget || !document.body.contains(event.relatedTarget)) clearDragCue();
});
document.addEventListener('drop', event => {
  if (!localDrag(event)) return;
  event.preventDefault();
  event.stopPropagation();
  const drag = nativeDrag;
  if (drag.kind === 'tree') {
    const drop = event.target.closest('#tab-row') && event.dataTransfer.getData(treeDragType) === drag.tree
      ? tabDropAt(event.clientX, drag.tree) : null;
    finishNativeDrag();
    if (drop) intent('place-tree', { id: drag.tree, place: 'tabs', before: drop.before });
    return;
  }
  let payload;
  try { payload = JSON.parse(event.dataTransfer.getData(pageDragType)); } catch { finishNativeDrag(); return; }
  const drop = payload.head === drag.head && payload.gesture === drag.request ? pageDropAt(event) : null;
  finishNativeDrag();
  if (drop?.to) intent('move-branch', { head: drag.head, to: drop.to });
});
document.addEventListener('dragend', finishNativeDrag);

let crossPress = null;
window.addEventListener('mousedown', (event) => {
  crossPress = null;
  const cross = event.target.closest('#tab-row [data-settle], #strip-nodes .node.tab [data-act="dismiss"]');
  if (event.button !== 0 || !cross || event.detail > 1) return;
  const r = cross.getBoundingClientRect();
  if (getComputedStyle(cross).visibility === 'visible' && Number(getComputedStyle(cross).opacity) > 0
      && event.clientX >= r.left && event.clientX < r.right && event.clientY >= r.top && event.clientY < r.bottom) {
    crossPress = cross;
  }
}, true);
$('tab-row').addEventListener('click', (event) => {
  const pressed = crossPress;
  crossPress = null;
  const close = event.target.closest('[data-settle]');
  if (close) {
    if (close === pressed && event.detail <= 1) treeAct(close.dataset.settle, 'settle');
    return;
  }
  const tab = event.target.closest('[data-id]');
  if (!tab) return;
  uncover();
  treeAct(tab.dataset.id, 'open');
});
$('tab-row').addEventListener('focusin', (event) => {
  for (const tab of $('tab-row').children) tab.tabIndex = tab === event.target ? 0 : -1;
});
$('tab-row').addEventListener('keydown', (event) => {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
  const tabs = [...$('tab-row').children].filter((tab) => !tab.hidden);
  const index = tabs.indexOf(document.activeElement);
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length;
  for (const tab of tabs) tab.tabIndex = tab === tabs[next] ? 0 : -1;
  tabs[next]?.focus();
  event.preventDefault();
});
$('tab-row').addEventListener('mousedown', (event) => {
  if (event.button === 1 && event.target.closest('[data-id]')) event.preventDefault();
});
$('tab-row').addEventListener('auxclick', (event) => {
  const tab = event.target.closest('[data-id]');
  if (event.button === 1 && tab) treeAct(tab.dataset.id, 'settle');
});
$('tab-row').addEventListener('contextmenu', (event) => {
  const tab = event.target.closest('[data-id]');
  if (!tab) return;
  event.preventDefault();
  openPopover('tree', tab, { source: 'tab', id: tab.dataset.id });
  post({ t: 'focus', target: 'chrome' });
  $('popover').querySelector('button')?.focus();
});
$('tab-overflow').addEventListener('click', (event) => {
  event.stopPropagation();
  if (S.popover?.kind === 'trees') closePopover();
  else openPopover('trees', $('tab-overflow'));
});
$('new-tree').addEventListener('click', () => window.canopy.newTree());
$('nav-back').addEventListener('click', () => intent('back'));
$('nav-forward').addEventListener('click', () => intent('forward'));
$('new-tree-close').addEventListener('click', (event) => {
  event.stopPropagation();
  window.canopy.escape('start');
});
$('new-tree-tab').addEventListener('click', focusStart);
$('new-tree-tab').addEventListener('keydown', (event) => {
  if (event.target === $('new-tree-tab') && ['Enter', ' '].includes(event.key)) {
    event.preventDefault();
    focusStart();
  }
});
$('forest-button').addEventListener('click', () => window.canopy.toggleForest());
$('search-pill').addEventListener('click', () => openPalette({ scope: S.surface === 'start' ? null : current(), text: '' }));
$('login-chip').addEventListener('click', (event) => {
  event.stopPropagation();
  if (!S.save) fillLogin();
  else if (S.popover?.kind === 'save-login') closePopover();
  else openPopover('save-login', $('login-chip'));
});
$('settings-button').addEventListener('click', (event) => {
  event.stopPropagation();
  if (S.popover?.kind === 'settings') closePopover();
  else openPopover('settings', $('settings-button'));
});
$('memory-dot').addEventListener('click', (event) => {
  event.stopPropagation();
  if (S.popover?.kind === 'memory') closePopover();
  else openPopover('memory', $('memory-dot'));
});

const listEl = $('strip-list');
listEl.addEventListener('mouseenter', () => { cancel('hideList'); cancel('showList'); });
listEl.addEventListener('mouseleave', (event) => {
  if (nativeDrag) return;
  if (event.relatedTarget?.closest('#strip-nodes')) return;
  schedule('hideList', 220, () => { if (!listHasKeyboardFocus()) hideList(); });
});

// A press outside an open popover closes it.
document.addEventListener('mousedown', (event) => {
  if (S.popover && !$('popover').contains(event.target) && !S.popover.anchor.contains(event.target)) closePopover();
});

// Over the transparent part of the window, the view only stays while something is open;
// a press there closes the attached list.
document.addEventListener('mousedown', (event) => {
  if (event.target === document.documentElement || event.target === document.body) {
    hideList();
  }
});

new ResizeObserver(() => fitTabs()).observe($('tabs'));
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', applyTheme);

document.addEventListener('DOMContentLoaded', () => post({ t: 'ready' }));

function answerFormReturn(proceed, focusPage = true) {
  if (!S.formReturn) return;
  const question = S.formReturn;
  canopy.formReturn(null);
  post({ t: 'form-return', question, proceed });
  if (focusPage) { if (proceed) post({ t: 'focus', target: 'page' }); else returnFocus('formReturn'); }
}
$('form-return-cancel').addEventListener('click', () => answerFormReturn(false));
$('form-return-continue').addEventListener('click', () => answerFormReturn(true));
$('form-return').querySelector('.scrim').addEventListener('mousedown', () => answerFormReturn(false));
document.addEventListener('mousedown', (event) => {
  if (S.formReturn && !event.target.closest('.form-return-box')) {
    answerFormReturn(false, !!event.target.closest('#form-return .scrim'));
  }
}, true);
document.addEventListener('keydown', (event) => {
  if (!S.formReturn || !tabKey(event)) return;
  event.preventDefault();
  const cancel = $('form-return-cancel');
  const proceed = $('form-return-continue');
  (document.activeElement === cancel ? proceed : cancel).focus();
});

document.addEventListener('focusin', (event) => {
  if (S.formReturn && !event.target.closest('.form-return-box')) {
    $('form-return-cancel').focus();
  }
}, true);

const NODE_TARGETS = '#strip-nodes [data-page], #strip-list [data-id], #strip-list [data-page], #tree-nodes [data-id], #tree-list [data-id], #popover.waiting [data-id], #forest-canopy [data-page]';
document.addEventListener('keydown', (event) => {
  if (event.defaultPrevented || S.popover || !(event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10'))) return;
  const el = event.target.closest(NODE_TARGETS);
  const id = el?.dataset.page || el?.dataset.id;
  if (!id || !visit(id)) return;
  event.preventDefault();
  openPopover('node', el, id);
});

document.addEventListener('contextmenu', (event) => {
  const field = event.target.closest('input, textarea, [contenteditable="true"]');
  if (field || (event.target.closest('#capture-text') && !getSelection().isCollapsed)) return;
  if (event.defaultPrevented) return;
  event.preventDefault();
  const el = event.target.closest(NODE_TARGETS);
  const id = el?.dataset.page || el?.dataset.id;
  if (!id || !visit(id)) return;
  openPopover('node', el, id);
});

document.addEventListener('keydown', (event) => {
  if (event.defaultPrevented) return;
  if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
    if (event.target.closest('input, textarea, [contenteditable]')) return;
    event.preventDefault();
    event.target.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
    return;
  }
  if (S.list && !S.popover && event.target.closest('#strip-list') && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
    const rows = controlsIn($('strip-list'));
    const index = rows.indexOf(document.activeElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : rows.length - 1)) % rows.length;
    rows[next]?.focus();
    event.preventDefault();
    return;
  }
  const root = S.popover ? $('popover') : S.reader ? $('reader') : S.palette ? $('palette') : S.overlay === 'forest' ? $('forest') : null;
  if (!root) return;
  const rows = controlsIn(root);
  let index = rows.indexOf(document.activeElement);
  if (tabKey(event)) index += event.shiftKey ? rows.length - 1 : 1;
  else if (S.popover && !event.target.matches('input') && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
    index = event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1 : index + (event.key === 'ArrowDown' ? 1 : rows.length - 1);
  } else return;
  rows[index % rows.length]?.focus();
  event.preventDefault();
});
window.addEventListener('focus', () => {
  if (pageShows() && document.activeElement === document.body) {
    controlsIn(document.body).at(-1)?.focus();
  }
});

function openMail() {
  S.mailIconRevision += 1;
  openPopover('mail', $('mail-pin'));
  intent('mail-opened');
}
$('mail-pin').addEventListener('click', (event) => {
  event.stopPropagation();
  if (S.popover?.kind === 'mail') closePopover(); else openMail();
});
$('mail-pin').addEventListener('keydown', (event) => {
  if (event.key !== 'ArrowDown') return;
  event.preventDefault();
  openMail();
});
$('page-scale').addEventListener('click', () => intent('page-scale', { action: 'reset' }));
