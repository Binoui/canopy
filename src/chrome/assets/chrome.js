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
  bookmark: '<path d="M6 3h12v18l-6-4-6 4z" stroke-width="1.7"/>',
  expand: '<path d="M9 4H4v5M15 4h5v5M4 15v5h5M20 15v5h-5" stroke-width="1.7"/>',
  collapse: '<path d="M4 9h5V4M20 9h-5V4M9 20v-5H4M15 20v-5h5" stroke-width="1.7"/>',
  pinned: '<path d="M9 3h6l-1 7 4 4v2H6v-2l4-4zM12 16v6" stroke-width="1.6"/>',
  plus: '<path d="M12 5v14M5 12h14" stroke-width="1.9"/>',
  search: '<circle cx="11" cy="11" r="6.5" stroke-width="1.9"/><path d="M20 20l-4.2-4.2" stroke-width="1.9"/>',
  spark: '<path d="M11 2.5l1.9 5.6 5.6 1.9-5.6 1.9L11 17.5l-1.9-5.6L3.5 10l5.6-1.9z" fill="currentColor" stroke="none"/><path d="M18.5 14l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9z" fill="currentColor" stroke="none" opacity=".7"/>',
  forest: '<circle cx="4.5" cy="12" r="2" stroke-width="1.8"/><circle cx="19.5" cy="5.5" r="2" stroke-width="1.8"/><circle cx="19.5" cy="12" r="2" stroke-width="1.8"/><circle cx="19.5" cy="18.5" r="2" stroke-width="1.8"/><path d="M6.5 12h11M10 12c2.5 0 3-6.5 7.5-6.5M10 12c2.5 0 3 6.5 7.5 6.5" stroke-width="1.8"/>',
  chevron: '<path d="M7 10l5 5 5-5" stroke-width="2"/>',
  globe: '<circle cx="12" cy="12" r="8.5" stroke-width="1.7"/><path d="M3.5 12h17M12 3.5c2.6 2.6 2.6 14.4 0 17M12 3.5c-2.6 2.6-2.6 14.4 0 17" stroke-width="1.7"/>',
  close: '<path d="M6 6l12 12M18 6L6 18" stroke-width="1.9"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" stroke-width="1.7"/><path d="M14 3v5h5" stroke-width="1.7"/>',
  key: '<circle cx="7.5" cy="15.5" r="5.5" stroke-width="1.7"/><path d="M21 2l-9.6 9.6M15.5 7.5l3 3L22 7l-3-3" stroke-width="1.7"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" stroke-width="1.7"/>',
  settings: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" stroke-width="1.6"/><circle cx="12" cy="12" r="3" stroke-width="1.6"/>',
};

const $ = (id) => document.getElementById(id);
const post = (message) => window.ipc.postMessage(JSON.stringify(message));
/// Intents that load a page also give the keyboard back to it, as the GTK buttons that
/// never took focus did.
const NAVIGATES = new Set(['reload', 'restore', 'select-tree', 'settle-tree', 'settle-current', 'open-suggestion', 'open-bookmark', 'open-citation', 'open-saved', 'branch-queued', 'fill-login', 'save-login']);
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
  logins: null,
  save: null,
  locked: null,
  strip: null,
  rev: 0,
  systemDark: null,
  surface: 'reading',
  overlay: null,
  formReturn: false,
  treeSide: 'tree',
  layer: 'none',
  hovered: null,
  list: null,
  popover: null,
  palette: null,
  reader: null,
  errors: [],
  forestLabels: true,
  camera: { x: 0, y: 0, zoom: 1 },
  forestDirty: true,
  reported: '',
};

const visit = (id) => S.frame?.visits[id];
const current = () => S.frame?.current ?? null;
/// The files downloaded from a page of the current tree, oldest first.
const filesOf = (id) => S.frame?.files?.[id] ?? [];

// ---------- favicons ----------

function hostOf(url) {
  try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return '?'; }
}

/// A page icon from Rust's cache, over the host's monogram, which shows until (or unless)
/// the icon loads.
function fav(url, opened, size) {
  const host = hostOf(url);
  let hash = 2166136261;
  for (let i = 0; i < host.length; i++) hash = Math.imul(hash ^ (host.charCodeAt(i) & 255), 16777619) >>> 0;
  const color = MONOGRAM[hash % MONOGRAM.length];
  const letter = esc((host[0] || '?').toUpperCase());
  const src = `icon?u=${encodeURIComponent(url)}&o=${opened ? 1 : 0}&r=${S.rev}`;
  return `<span class="fav" style="width:${size}px;height:${size}px;background:${color};border-radius:${Math.min(3, size / 4)}px">`
    + `<b style="font-size:${Math.round(size * 0.62)}px">${letter}</b>`
    + `<img src="${src}" alt="" onload="this.parentNode.style.background='none';this.previousSibling.style.visibility='hidden'" onerror="this.remove()"></span>`;
}
const waitState = (v) => v.queued ? (v.loaded ? 'loaded' : 'unloaded') : null;
const markClass = (v) => v.dismissed ? ' dismissed' : waitState(v) === 'loaded' ? ' pending loaded' : waitState(v) ? ' pending' : '';
const visitFav = (v, size) => fav(v.url, waitState(v) !== 'unloaded', size);
const markedFav = (v, size, live = v.live) => `<span class="mk">${visitFav(v, size)}`
  + (live ? '<i class="live-dot"></i>' : '')
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
  if (S.layer !== 'none' || S.overlay) return;
  S.hovered = key;
  cancel('hideList');
  schedule('showList', 250, () => { if (S.hovered === key) showList(key); });
}
function leaveNode(event) {
  S.hovered = null;
  cancel('showList');
  if (event.relatedTarget?.closest('#strip-list')) return;
  if (S.list) schedule('hideList', 220, hideList);
}
function setLayer(layer) {
  S.layer = layer;
  if (layer !== 'none') hideList();
  syncLayer();
}

/// Tells Rust how far down the view must reach (null for the whole window), and whether
/// Escape is ours. Below that, the page gets the pointer.
function syncLayer() {
  const modal = !!(S.formReturn || S.reader || S.palette || S.popover || S.list || S.overlay
    || (S.surface !== 'reading' && current()));
  let height = HEADER + REST;
  if (S.formReturn || S.palette || S.reader || S.overlay || S.surface !== 'reading') {
    height = null;
  } else {
    for (const el of [S.list && $('strip-list'), S.popover && $('popover')]) {
      if (!el) continue;
      const bottom = el.hidden ? HEADER + REST + 420 : el.getBoundingClientRect().bottom;
      height = Math.max(height, bottom + 30);
    }
    height = Math.ceil(height);
  }
  const settledToast = !$('toast').hidden && $('toast').classList.contains('settled');
  const undo = settledToast ? $('toast').querySelector('button').getBoundingClientRect() : null;
  const toastRect = undo ? [Math.floor(undo.x), Math.floor(undo.y), Math.ceil(undo.width), Math.ceil(undo.height)] : null;
  const report = `${height}/${modal}/${JSON.stringify(toastRect)}`;
  if (report !== S.reported) {
    S.reported = report;
    post({ t: 'layer', height, modal, toastRect });
  }
}

// ---------- theme ----------

function applyTheme() {
  const mode = S.frame?.theme ?? 'dark';
  const system = S.systemDark ?? matchMedia('(prefers-color-scheme: dark)').matches;
  document.documentElement.classList.toggle('dark', mode === 'dark' || (mode === 'system' && system));
}

// ---------- tab row ----------

function renderTabs() {
  const row = $('tab-row');
  const trees = S.frame?.tabs ?? [];
  row.innerHTML = trees.map((tree, index) => {
    const root = visit(tree.id);
    const pages = `${tree.pages} page${tree.pages === 1 ? '' : 's'}`;
    const pending = tree.pending ? `, ${tree.pending} pending` : '';
    const key = index < 9 ? ` · Ctrl+${index + 1}` : '';
    return `<button class="tab${tree.active ? ' on' : ''}" data-id="${tree.id}" title="${esc(`${tree.title} · ${pages}${pending}${key}`)}" aria-label="${esc(`${tree.title}, ${pages}${pending}`)}">`
      + (root ? `<span class="mk">${visitFav(root, 14)}${tree.live ? '<i class="live-dot"></i>' : ''}</span>` : '')
      + `<span class="tab-title">${esc(tree.title)}</span>`
      + (tree.pinned ? `<span class="tab-pinned-mark" aria-label="Pinned">${icon('pinned')}</span>` : '')
      + (tree.pending ? `<span class="count">${tree.pending}</span>` : '')
      + `<span class="tab-settle" data-settle="${tree.id}" title="Settle${tree.active ? ' · Ctrl+W' : ''}" aria-hidden="true">${icon('close')}</span>`
      + '</button>';
  }).join('');
  fitTabs();
}

/// Hides the trees that do not fit behind the overflow arrow, keeping creation order and
/// the active tree visible.
function fitTabs() {
  const row = $('tab-row');
  const tabs = [...row.children];
  tabs.forEach((tab) => { tab.hidden = false; });
  const overflow = $('tab-overflow');
  overflow.hidden = true;
  const header = $('tabs');
  const fixed = [...header.children].filter((el) => el !== row && !el.classList.contains('spacer'))
    .reduce((sum, el) => sum + (el.hidden ? 0 : el.offsetWidth + 4), 0);
  const room = header.clientWidth - 14 - fixed - 36;
  const widths = tabs.map((tab) => tab.offsetWidth + 2);
  if (widths.reduce((a, b) => a + b, 0) <= room + 36) return;
  const shown = [];
  let used = 0;
  tabs.forEach((tab, i) => {
    if (used + widths[i] <= room) { shown.push(i); used += widths[i]; }
  });
  const active = tabs.findIndex((tab) => tab.classList.contains('on'));
  if (active >= 0 && !shown.includes(active)) {
    while (shown.length && used + widths[active] > room) used -= widths[shown.pop()];
    shown.push(active);
  }
  tabs.forEach((tab, i) => { tab.hidden = !shown.includes(i); });
  overflow.hidden = false;
}

// ---------- strip ----------

const strip = {
  shown: { height: REST, pan: 0, nodes: new Map(), edges: new Map() },
  from: null,
  target: null,
  anim: null,
  elements: new Map(),
  badges: new Map(),
  leaves: new Map(),
};

const targetLayout = () => S.strip ?? null;

function animateStrip(duration) {
  const layout = targetLayout();
  if (!layout) return;
  strip.from = strip.shown;
  strip.target = layout;
  buildStrip(layout);
  if (!duration) {
    strip.anim = null;
    step(performance.now(), 1);
    pruneStrip();
    syncLayer();
    return;
  }
  strip.anim = { start: performance.now(), duration };
  syncLayer();
  requestAnimationFrame(tick);
}

function tick(now) {
  if (!strip.anim) return;
  const t = (now - strip.anim.start) / strip.anim.duration;
  step(now, t);
  if (t < 1) {
    requestAnimationFrame(tick);
  } else {
    strip.anim = null;
    pruneStrip();
    syncLayer();
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

/// One element per node, kept across frames so hovers and transitions survive a re-layout.
function buildStrip(layout) {
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
    if (!el) {
      el = document.createElement('button');
      el.className = 'node';
      el.dataset.id = n.id;
      el.addEventListener('mouseenter', () => enterNode(el.dataset.id));
      el.addEventListener('mouseleave', leaveNode);
      el.addEventListener('focus', () => showList(el.dataset.id));
      el.addEventListener('click', () => {
        const item = strip.target.nodes.find((n) => n.id === el.dataset.id);
        if (['run', 'earlier', 'after'].includes(item.kind)) showList(item.id);
        else activate(item.page);
      });
      el.addEventListener('keydown', walkTrunk);
      nodes.appendChild(el);
      strip.elements.set(n.id, el);
    }
    if (!['run', 'earlier', 'after'].includes(n.kind)) el.dataset.page = n.page;
    else delete el.dataset.page;
    el.className = `node ${n.kind}` + (n.named ? ' named' : '') + markClass(v);
    el.setAttribute('aria-label', `${n.title}, ${n.branches.pages} branch pages, ${n.branches.waiting} waiting`);
    el.removeAttribute('title');
    const html = n.kind === 'after'
      ? (n.read ? `<span class="read-count">${n.read}</span>` : '') + (n.waiting ? `<span class="count wait-count"><i></i>${n.waiting}</span>` : '')
      : n.kind === 'earlier' ? `<span>${esc(n.label)}</span>`
      : n.kind === 'pill' ? visitFav(v, 16) + `<span class="pill-text"><span class="pill-title">${esc(v.label)}</span><span class="pill-site">${esc(v.site)}</span></span>` + (v.live ? '<i class="live-dot"></i>' : '')
      : visitFav(v, n.kind === 'root' || n.kind === 'page' ? 16 : 14) + (n.kind === 'parent' || n.kind === 'run' || n.named ? `<span class="item-label">${esc(n.label)}</span>` : '')
        + (v.live && n.kind !== 'run' ? `<i class="live-dot${filesOf(n.page).length ? ' above-leaves' : ''}"></i>` : '');
    if (el.dataset.html !== html) {
      el.innerHTML = html;
      el.dataset.html = html;
    }
    let badge = strip.badges.get(n.id);
    if (n.branches.pages && n.kind !== 'pill') {
      if (!badge) {
        badge = document.createElement('i');
        canvas.appendChild(badge);
        strip.badges.set(n.id, badge);
      }
      badge.className = `branch-count${n.branches.waiting ? ' waiting' : ''}`;
      badge.textContent = n.branches.pages;
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
      leaves.title = label;
      leaves.setAttribute('aria-label', `${label}, from ${v.label}`);
    } else if (leaves) {
      leaves.remove();
      strip.leaves.delete(n.id);
    }
  }
  const dot = $('memory-dot');
  dot.classList.toggle('on', !!S.frame?.captureEnabled);
  dot.title = S.frame?.captureEnabled ? 'Memory · Remembering pages' : 'Memory · Not remembering pages';
}

function step(now, t) {
  const k = ease(t);
  const from = strip.from, to = strip.target;
  const nodes = new Map();
  const targets = new Map(to.nodes.map((n) => [n.id, n]));
  for (const n of to.nodes) {
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
    edges.set(key, { kind: e.kind, to: e.to, c: e.c.map((p, i) => [mix(a[i][0], p[0], k), mix(a[i][1], p[1], k)]) });
  }
  const pan = mix(from.pan, to.pan, k);
  strip.shown = { height: mix(from.height, to.height, k), pan, nodes, edges, width: to.width };
  drawStrip();
}

function drawStrip() {
  const shown = strip.shown;
  $('strip').style.height = `${shown.height}px`;
  $('strip-clip').style.height = `${shown.height}px`;
  const bg = $('strip-bg');
  bg.style.height = `${shown.height}px`;
  $('strip-canvas').style.transform = `translateX(${shown.pan}px)`;
  for (const [id, r] of shown.nodes) {
    const el = strip.elements.get(id);
    if (!el) continue;
    el.style.left = `${r.x - r.w / 2}px`;
    el.style.top = `${r.y - r.s / 2}px`;
    el.style.width = `${r.w}px`;
    el.style.height = `${r.s}px`;
    el.style.opacity = r.o;
    el.style.visibility = r.w < 1 ? 'hidden' : '';
    const badge = strip.badges.get(id);
    if (badge) {
      badge.style.left = `${r.x + r.w / 2 + 7 - badge.offsetWidth}px`;
      badge.style.top = `${r.y + r.s / 2 + 6 - 14}px`;
      badge.style.opacity = r.o;
    }
    const leaves = strip.leaves.get(id);
    if (leaves) {
      // Leaves stand on the node's top edge, clear of its rounded corner.
      leaves.style.left = `${r.x - r.w / 2 + (leaves.classList.contains('on-pill') ? 5 : 0)}px`;
      leaves.style.top = `${r.y - r.s / 2 + 1 - leaves.offsetHeight}px`;
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
    dot.style.left = `${r.x + r.w / 2 - ACCESSORY}px`;
    dot.style.top = `${r.y - 9}px`;
    const reach = 40;
    const x = r.x - r.w / 2 + 16;
    Object.assign(glow.style, { left: `${x - reach}px`, top: `${TRUNK_Y - reach}px`, width: `${2 * reach}px`, height: `${2 * reach}px` });
  }
  const opacity = new Map([...shown.nodes].map(([id, n]) => [id, n.o]));
  $('strip-edges').innerHTML = [...shown.edges.values()].map((e) => {
    const [p0, p1, p2, p3] = e.c;
    const o = e.kind === 'path' ? 1 : (opacity.get(e.to) ?? 1);
    return `<path class="${e.kind}" style="opacity:${o}" d="M${p0} C${p1} ${p2} ${p3}"/>`;
  }).join('');
  positionList();

}

function activate(id) {
  uncover();
  if (id === current()) post({ t: 'focus', target: 'page' });
  else intent('restore', { id });
}

/// Left and Right walk the trunk while a node has the keyboard.
function walkTrunk(event) {
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

function showList(key) {
  if (S.layer !== 'none' || S.overlay || S.surface !== 'reading') return;
  cancel('hideList');
  cancel('showList');
  const item = strip.target?.nodes.find((n) => n.id === key);
  if (!item) return;
  const previous = S.list;
  S.list = { key, preview: previous?.key === key ? previous.preview : null };
  renderList();
  requestListPreview();
}

function requestListPreview() {
  const item = listItem();
  if (!item?.preview) return;
  previewSeq += 1;
  post({ t: 'preview', id: item.page, seq: previewSeq });
}

function hideList() {
  cancel('showList');
  cancel('hideList');
  S.list = null;
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
  const x = r.x - r.w / 2 + strip.shown.pan;
  list.style.left = `${clamp(x - 2, 8, window.innerWidth - width - 8)}px`;
  list.style.top = `${HEADER + REST - 6}px`;
  list.style.maxHeight = `${Math.max(0, (S.windowHeight ?? 800) - HEADER - REST - 2)}px`;
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
    ['head', `<div class="bl-head"${page ? ` tabindex="0" data-page="${item.page}"` : ''}>${item.kind === 'earlier' ? '<span class="bl-glyph">···</span>' : markedFav(v, 16)}<div><b>${esc(item.title)}</b><span>${esc(item.meta)}${v.live ? ' · Open in memory' : ''}</span></div></div>`],
    ['snippet', page && preview?.snippet ? `<div class="bl-snippet">${esc(preview.snippet)}</div>` : ''],
    ['source', source ? `<div class="card-source"><div class="card-meta">${esc(source.text)}</div></div>` : ''],
    ['heading', item.heading ? `<div class="menu-heading">${esc(item.heading)}</div>` : ''],
    ...item.rows.map((row) => {
      const v = visit(row.id);
      const files = filesOf(row.id);
      const leaf = files.length ? `<button class="leaves" data-files="${row.id}" aria-label="${esc(`Files from ${v.label}`)}"><i class="leaf${files.every((f) => f.missing) ? ' missing' : ''}"></i></button>` : '';
      return [`row-${row.id}`, v ? `<div class="list-entry"><button class="menu-row${markClass(v)}" data-id="${row.id}" title="${esc(v.label)}" aria-label="${esc(v.label)}">${markedFav(v, 16)}<span class="grow">${esc(row.label)}</span><span class="menu-detail">${esc(row.detail)}</span></button>${leaf}</div>` : ''];
    }),
    ['more', item.more ? `<button class="menu-row more" data-act="more">${icon('expand')}<span class="grow">${esc(item.more)}</span><kbd>Ctrl Shift E</kbd></button>` : ''],
    ...item.actions.map((action) => [`action-${action.kind}`, `<button class="action bl-action" data-act="${action.kind}">${esc(action.label)}</button>`]),
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
        const leaves = el.querySelector('.leaves');
        const newLeaves = fresh.querySelector('.leaves');
        if (leaves && newLeaves) leaves.innerHTML = newLeaves.innerHTML;
        else if (newLeaves) el.appendChild(newLeaves);
        else leaves?.remove();
      } else el.innerHTML = fresh.innerHTML;
      el.className = fresh.className;
      el.dataset.html = html;
    }
    if (key === 'head') {
      if (page) { el.dataset.page = item.page; el.tabIndex = 0; }
      else { delete el.dataset.page; el.removeAttribute('tabindex'); }
    }
    if (el !== cursor) list.insertBefore(el, cursor);
    cursor = el.nextElementSibling;
  }
  for (const el of [...list.children]) if (!kept.has(el.dataset.key)) el.remove();
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
  else if (action === 'reload') { uncover(); intent('reload'); }
  else if (action === 'open') { uncover(); intent('restore', { id: item.page }); }
  else if (action === 'branch') {
    if (v.queued) intent('branch-queued', { id: item.page });
    else openPalette({ scope: item.page, text: '' });
  } else if (action === 'source') activate(v.parent);
  else if (action === 'comment') intent('open-citation', { id: item.page });
});

// ---------- waiting pages and pinned controls ----------

function applyStrip(layouts) {
  S.strip = layouts;
  $('strip-clip').style.width = `${layouts.room}px`;
  placePins();
}

function placePins() {
  const placed = new Map((S.strip?.pins ?? []).map((p) => [p.kind, p]));
  for (const el of $('pins').children) {
    const p = placed.get(el.dataset.pin);
    el.hidden = !p;
    if (p) Object.assign(el.style, { left: `${p.x - p.w / 2}px`, top: `${p.y - p.s / 2}px`, width: `${p.w}px`, height: `${p.s}px` });
  }
}

function renderWaitingPin() {
  const ids = S.frame?.waiting.ids ?? [];
  const next = visit(ids[0]);
  $('waiting-pin').innerHTML = next ? markedFav(next, 16) + `<span class="count">${ids.length}</span>` : '';
  $('waiting-pin').title = next ? `Next waiting page: ${next.label} · Ctrl+Tab` : '';
}

function openNextWaiting(id = S.frame?.waiting.ids[0]) {
  if (!id) { canopy.toast('info', 'No waiting pages in this tree'); return; }
  closeReader();
  closePalette(false);
  closePopover(false);
  activate(id);
}

function enterWaitingPin() {
  cancel('waitingClose');
  if (S.layer !== 'none' || S.popover) return;
  schedule('waitingOpen', 120, () => {
    if (S.layer !== 'none' || S.popover || !S.frame?.waiting.ids.length) return;
    openPopover('waiting', $('waiting-pin'), null);
  });
}

function leaveWaitingPin() {
  cancel('waitingOpen');
  const popover = S.popover;
  if (popover?.kind === 'waiting') schedule('waitingClose', 220, () => {
    if (S.popover === popover) closePopover(false);
  });
}

$('waiting-pin').addEventListener('click', () => openNextWaiting());
$('waiting-pin').addEventListener('mouseenter', enterWaitingPin);
$('waiting-pin').addEventListener('mouseleave', leaveWaitingPin);
$('popover').addEventListener('mouseenter', () => cancel('waitingClose'));
$('popover').addEventListener('mouseleave', leaveWaitingPin);

// ---------- popovers: Memory menu, trees that do not fit, a tree's name ----------

/// Opens a menu under `anchor`, or under `below` when the anchor sits on top of something
/// the menu should not cover.
function openPopover(kind, anchor, data, below = anchor) {
  const popover = $('popover');
  const a = anchor.getBoundingClientRect();
  const b = below.getBoundingClientRect();
  S.popover = { kind, anchor, data };
  renderPopover();
  popover.hidden = false;
  const width = popover.offsetWidth;
  popover.style.left = `${clamp(a.left, 8, window.innerWidth - width - 8)}px`;
  const belowRoom = (S.windowHeight ?? window.innerHeight) - b.bottom - 14;
  const above = belowRoom < 200 && a.top > belowRoom;
  popover.style.maxHeight = `${Math.max(40, above ? a.top - 14 : belowRoom)}px`;
  popover.style.top = `${above ? Math.max(8, a.top - popover.offsetHeight - 6) : b.bottom + 6}px`;
  if (S.layer !== 'palette') setLayer('menu');
  syncLayer();
}

function closePopover(returnFocus = true) {
  if (!S.popover) return;
  const anchor = S.popover.anchor;
  S.popover = null;
  if (S.overlay === 'tree' && anchor?.isConnected) anchor.focus();
  $('popover').hidden = true;
  if (S.layer === 'menu') setLayer(restingLayer());
  if (returnFocus && pageShows() && !S.palette && !S.reader) post({ t: 'focus', target: 'page' });
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
  chip.classList.toggle('on', !!S.save);
  if (S.save) {
    chip.innerHTML = `${icon('key')}<span>${S.save.update ? 'Update password?' : 'Save login?'}</span>`;
    chip.title = `${S.save.update ? 'Update the saved password' : 'Save the login'} for ${loginLabel(S.save.username)} on ${S.save.site}`;
  } else if (S.locked && !names.length) {
    chip.innerHTML = `${icon('key')}<span>${esc(`Unlock ${S.locked}`)}</span>`;
    chip.title = `Unlock ${S.locked} to fill a saved login · Ctrl+Shift+L`;
  } else if (names.length) {
    const one = names.length === 1;
    chip.innerHTML = `${icon('key')}<span>${esc(one ? loginLabel(names[0]) : `${names.length} logins`)}</span>`;
    chip.title = `${one ? `Fill the saved login for ${loginLabel(names[0])}` : 'Choose a saved login to fill'} · Ctrl+Shift+L`;
  }
  if (S.popover?.kind === 'save-login' && !S.save) closePopover(false);
  if (S.popover?.kind === 'logins') {
    if (names.length > 1 && !S.save) refreshPopover();
    else closePopover(false);
  }
  fitTabs();
}

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
  const detail = v.queued ? 'pending' : v.site.split(' · ').slice(1).join(' · ');
  return `<button class="menu-row" data-id="${v.id}">${visitFav(v, 16)}<span class="grow">${esc(v.label)}</span><span class="menu-detail">${esc(detail)}</span></button>`;
}

// A refresh rebuilds an open popover's rows. The row that had the keyboard keeps it.
function refreshPopover() {
  const index = [...$('popover').querySelectorAll('button')].indexOf(document.activeElement);
  renderPopover();
  if (index >= 0 && S.popover) $('popover').querySelectorAll('button')[index]?.focus();
}

function renderPopover() {
  const popover = $('popover');
  const { kind, data } = S.popover;
  popover.classList.toggle('waiting', kind === 'waiting');
  if (kind !== 'node') delete popover.dataset.nodeHtml;
  if (kind === 'node') {
    const v = visit(data);
    if (!v) { closePopover(false); return; }
    const html = `<div class="node-menu-title">${visitFav(v, 16)}<span>${esc(v.shortName)}</span></div>` + v.nodeMenu.map((r) => `<button class="menu-row" data-node-action="${r.kind}">${esc(r.label)}</button>`).join('');
    if (popover.dataset.nodeHtml !== html || popover.dataset.nodeId !== data) {
      popover.innerHTML = html;
      popover.dataset.nodeHtml = html;
      popover.dataset.nodeId = data;
      for (const button of popover.querySelectorAll('[data-node-action]')) button.onclick = () => {
        const action = button.dataset.nodeAction;
        closePopover(false);
        if (action === 'open') activate(data);
        else if (action === 'new-tree') { uncover(); intent('open-page-in-tree', { id: data }); }
        else if (action === 'branch') openPalette({ scope: data, text: '' });
        else if (action === 'copy') post({ t: 'copy', text: visit(data).url });
        else intent(action, { id: data });
      };
    }
  } else if (kind === 'tab' || kind === 'unpin') {
    const tab = S.frame.tabs.find((t) => t.id === data);
    const menu = kind === 'unpin' ? { label: 'Unpin tree', pinned: false } : tab?.pinMenu;
    if (!menu) { closePopover(false); return; }
    popover.innerHTML = (kind === 'tab' ? '<button class="menu-row" data-tab-action="rename">Rename</button>' : '')
      + `<button class="menu-row" data-tab-action="pin">${esc(menu.label)}</button>`
      + (kind === 'tab' ? '<button class="menu-row" data-tab-action="settle">Settle tree</button>' : '');
    popover.querySelectorAll('[data-tab-action]').forEach((row) => row.addEventListener('click', () => {
      const anchor = S.popover.anchor;
      const action = row.dataset.tabAction;
      closePopover(false);
      if (action === 'rename') {
        openPopover('rename', anchor, data);
        S.popover.field.focus(); S.popover.field.select();
      } else intent(action === 'pin' ? 'pin-tree' : 'settle-tree', { id: data, pinned: menu.pinned });
    }));
  } else if (kind === 'bookmark') {
    popover.innerHTML = '<button class="menu-row" data-remove-bookmark>Remove bookmark</button>';
    popover.querySelector('button').addEventListener('click', () => { closePopover(false); intent('remove-bookmark', { id: Number(data) }); });
  } else if (kind === 'folder') {
    const b = S.frame.start.bookmarks;
    if (data.hidden) {
      popover.innerHTML = (b.hidden?.folders ?? []).map((f) => `<button class="menu-row" data-start="folder" data-id="${esc(f.name)}">${icon('folder')}<span class="grow">${esc(f.name)}</span><span class="menu-detail">${f.count}</span></button>`).join('');
    } else {
      const folder = [...b.folders, ...(b.hidden?.folders ?? [])].find((f) => f.name === data.folder);
      const rows = data.more ? b.more?.rows : folder?.rows;
      popover.innerHTML = (rows ?? []).map((r) => `<button class="menu-row" data-start="bookmark" data-id="${r.id}" title="${esc(r.label + '\n' + r.url)}">${fav(r.url, true, 16)}<span class="grow">${esc(r.label)}</span></button>`).join('');
    }
  } else if (kind === 'waiting') {
    const { ids, order } = S.frame.waiting;
    if (!ids.length) { closePopover(false); return; }
    popover.innerHTML = `<div class="pop-head"><span class="menu-heading grow">Waiting in this tree · ${ids.length}</span><span class="menu-detail">${esc(order)}</span></div>`
      + ids.map((id, i) => {
        const v = visit(id);
        return `<button class="menu-row${i === 0 ? ' next' : ''}" data-id="${id}">${markedFav(v, 16)}<span class="grow">${esc(v.label)}</span><span class="menu-detail">${esc(v.when)}</span>${i === 0 ? '<kbd>Ctrl Tab</kbd>' : ''}</button>`;
      }).join('');
    popover.querySelectorAll('[data-id]').forEach((row) => row.addEventListener('click', () => openNextWaiting(row.dataset.id)));
  } else if (kind === 'save-login') {
    const save = S.save;
    if (!save) { closePopover(false); return; }
    popover.innerHTML = `<div class="menu-heading">${save.update ? 'Update the saved password?' : 'Save this login?'}</div>`
      + `<div class="menu-row" id="save-login-account">${icon('key')}<span class="grow">${esc(loginLabel(save.username))}</span><span class="menu-detail">${esc(save.site)}</span></div>`
      + `<div class="menu-hint">Kept in ${esc(save.keeper)}.</div>`
      + `<div class="card-actions save-actions"><button class="action" data-save="1">${save.update ? 'Update' : 'Save'}</button><button class="action" data-save="0">Not now</button></div>`;
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
    popover.innerHTML = (S.frame?.tabs ?? []).filter((tree) => hidden.includes(tree.id)).map((tree) => {
      const root = visit(tree.id);
      return `<button class="menu-row" data-id="${tree.id}">${root ? visitFav(root, 16) : ''}<span class="grow">${esc(tree.title)}</span>`
        + (tree.pending ? `<span class="count">${tree.pending}</span>` : '') + '</button>';
    }).join('');
    popover.querySelectorAll('[data-id]').forEach((row) => row.addEventListener('click', () => {
      closePopover();
      uncover();
      intent('select-tree', { id: row.dataset.id });
    }));
  } else if (kind === 'rename') {
    // A frame that arrives while a name is being typed must not reset the field.
    if (S.popover.field) return;
    const title = (S.frame?.tabs ?? []).find((tree) => tree.id === data)?.title ?? '';
    popover.innerHTML = '<div class="menu-heading">Tree name</div>'
      + `<input id="tree-name" class="menu-input" value="${esc(title)}" maxlength="80" spellcheck="false" aria-label="Tree name">`
      + '<div class="menu-hint">Leave it empty to name the tree after its pages.</div>';
    const field = S.popover.field = $('tree-name');
    field.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter') return;
      // Enter on the name as it was changes nothing: it stays automatic if it was.
      if (field.value.trim() !== title) intent('rename-tree', { id: data, name: field.value });
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
    popover.innerHTML = '<div class="menu-heading">Memory</div>'
      + captureRow()
      + '<button class="menu-row" id="remember-selection"><span class="grow">Remember selection</span><kbd>Ctrl+S</kbd></button>'
      + `<button class="menu-row" id="read-captures"${current() ? '' : ' disabled'}><span class="grow">Read saved text</span></button>`
      + (S.errors.length ? '<div class="menu-separator"></div><div class="menu-heading">Recent errors</div>'
        + S.errors.map((e) => `<div class="menu-error">${esc(e)}</div>`).join('') : '');
    wireCaptureRow();
    $('remember-selection').addEventListener('click', () => { closePopover(); intent('remember'); });
    $('read-captures').addEventListener('click', () => { closePopover(); intent('read-captures', { id: current() }); });
  } else if (kind === 'settings') {
    const theme = S.frame?.theme ?? 'dark';
    popover.innerHTML = '<div class="menu-heading">Settings</div>'
      + '<div class="menu-heading">Theme</div><div class="segmented" id="theme-mode">'
      + ['dark', 'light', 'system'].map((mode) => `<button data-mode="${mode}" class="${mode === theme ? 'on' : ''}">${mode[0].toUpperCase()}${mode.slice(1)}</button>`).join('')
      + '</div><div class="menu-separator"></div>'
      + captureRow();
    wireCaptureRow();
    popover.querySelectorAll('[data-mode]').forEach((button) => button.addEventListener('click', () => {
      S.frame.theme = button.dataset.mode;
      applyTheme();
      intent('theme', { mode: button.dataset.mode });
      renderPopover();
    }));
  }
}

function captureRow() {
  const on = !!S.frame?.captureEnabled;
  return `<button class="menu-row" id="automatic-capture" title="Keep the text of opened pages on this computer. Form fields are never kept."><span class="grow">Remember pages</span><span class="switch${on ? ' on' : ''}"></span></button>`;
}

function wireCaptureRow() {
  $('automatic-capture').addEventListener('click', () => {
    const on = !!S.frame?.captureEnabled;
    S.frame.captureEnabled = !on;
    intent('capture', { on: !on });
    renderPopover();
    buildStrip(strip.target ?? targetLayout());
  });
}

// ---------- palette ----------

let paletteSeq = 0, paletteTimer = 0;

function openPalette({ scope, text, typed = false }) {
  intent('cancel');
  if (S.surface === 'start') scope = null;
  closePopover(false);
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

function closePalette(returnFocus = true) {
  if (!S.palette) return;
  clearTimeout(paletteTimer);
  S.palette = null;
  $('palette').hidden = true;
  $('palette-results').innerHTML = '';
  setLayer(restingLayer());
  if (returnFocus && pageShows()) post({ t: 'focus', target: 'page' });
  syncLayer();
}

function queryPalette() {
  if (!S.palette) return;
  paletteSeq += 1;
  S.palette.seq = paletteSeq;
  post({ t: 'palette', q: $('palette-input').value, seq: paletteSeq });
}

function renderPalette() {
  const { rows, selected } = S.palette;
  let group = null;
  $('palette-results').innerHTML = rows.map((row, i) => {
    const heading = row.group !== group ? `<div class="palette-group">${esc(row.group)}</div>` : '';
    group = row.group;
    const glyph = typeof row.icon === 'string' ? icon(row.icon) : fav(row.icon.url, row.icon.opened, 16);
    return heading + `<button class="palette-item${i === selected ? ' selected' : ''}" data-row="${i}" data-kind="${row.kind}">${glyph}`
      + `<span class="palette-title">${esc(row.title)}</span>`
      + (row.detail ? `<span class="palette-detail">${esc(row.detail)}</span>` : '') + '</button>';
  }).join('');
  $('palette-results').querySelector('.selected')?.scrollIntoView({ block: 'nearest' });
}

function runPalette(index, newTree) {
  const palette = S.palette;
  if (!palette) return;
  const row = palette.rows[index];
  const query = $('palette-input').value;
  post({ t: 'run', q: query, item: row ? row.item : null, newTree, scope: palette.scope });
  const reading = row?.kind !== 'saved';
  closePalette(reading);
  if (reading) uncover();
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
      el.className = `${tile ? 'tree-tile' : 'tree-row'}${r.path ? ' path' : ''}${item.id === current() ? ' current' : ''}${markClass(v)}`;
      el.title = v.label;
      el.setAttribute('aria-current', item.id === current() ? 'page' : 'false');
      el.setAttribute('aria-level', r.depth + 1);
      if (r.branch) el.setAttribute('aria-expanded', String(!r.folded));
      else el.removeAttribute('aria-expanded');
      el.style.cssText = tile
        ? `left:${item.x-item.w/2}px;top:${item.y-item.s/2}px;width:${item.w}px;height:${item.s}px`
        : `padding-left:${8+r.depth*22}px`;
      const html = `<span data-fold="${r.branch ? item.id : ''}" class="tree-fold">${r.branch ? (r.folded ? '▸' : '▾') : ''}</span>${markedFav(v, 16)}<span class="${tile ? 't' : 'grow'}">${esc(v.stub || (tile ? r.title : v.label))}${r.summary ? `<small>${esc(r.summary)}</small>` : ''}</span>${!tile ? `<span class="menu-detail">${esc(v.when)}${v.live ? ' · Open in memory' : ''}</span>` : v.live ? '<i class="live-dot"></i>' : ''}`;
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
    button.classList.toggle('on', selected);
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
  if (button) { setTreeSide(button.dataset.side); focusTree(current()); }
});
$('tree-sheet').addEventListener('click', (event) => {
  const page = event.target.closest('[data-id]');
  if (event.target.closest('[data-fold]')?.dataset.fold) post({ t: 'workbench', id: page.dataset.id });
  else if (page) activate(page.dataset.id);
});
$('tree-filter').addEventListener('input', () => post({ t: 'workbench', query: $('tree-filter').value }));
$('tree-waiting').addEventListener('click', () => post({ t: 'workbench', waiting: !S.frame.treeView.waitingOnly }));
$('tree-board').addEventListener('wheel', (event) => {
  if (event.ctrlKey || event.deltaX || !event.deltaY) return;
  const board = $('tree-board');
  if (board.scrollWidth <= board.clientWidth) return;
  event.preventDefault();
  board.scrollLeft += event.deltaY;
}, { passive: false });
function focusTree(id) {
  const body = $(S.treeSide === 'tree' ? 'tree-nodes' : 'tree-list');
  const el = body.querySelector(`[data-id="${id}"]`) || body.querySelector('[data-id]');
  (el || $('tree-filter')).focus({ preventScroll: true });
  el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}
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
    $('popover').querySelector('button')?.focus();
  } else if (!field && event.key.length === 1) {
    $('tree-filter').focus();
    $('tree-filter').value += event.key;
    post({ t: 'workbench', query: $('tree-filter').value });
  } else if (event.key === 'Tab') {
    const controls = [...$('tree-sheet').querySelectorAll('button, input')].filter((el) => el.getClientRects().length);
    const index = controls.indexOf(document.activeElement);
    controls[(index + (event.shiftKey ? controls.length - 1 : 1)) % controls.length]?.focus();
  } else return;
  event.preventDefault();
});
$('tree-sheet').querySelector('.scrim').addEventListener('mousedown', closeOverlay);
$('zoom-pin').innerHTML = icon('expand') + icon('collapse');
$('zoom-pin').addEventListener('click', () => toggleOverlay('tree'));

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
        setTreeSide(view.overflows ? 'list' : S.treeSide, true);
        post({ t: 'focus', target: 'chrome' });
        focusTree(current());
      });
    },
  },
  forest: {
    el: 'forest', button: 'forest-button',
    available: () => !!S.frame,
    show(reveal) { renderForest(); if (reveal) fitForest(false); },
  },
};
const restingLayer = () => (S.palette || S.overlay || S.surface !== 'reading') ? 'palette' : 'none';
const pageShows = () => S.surface === 'reading' && !S.overlay;

function setSurface(surface) {
  if (S.surface === surface) return;
  S.surface = surface;
  post({ t: 'surface', start: surface === 'start' });
  if (surface === 'start') focusStart();
  renderPlaceholder();
  $('start').hidden = surface !== 'start';
  setLayer(restingLayer());
  syncLayer();
}

function setOverlay(kind) {
  if (S.overlay === kind) return;
  const previous = OVERLAYS[S.overlay];
  if (previous) {
    $(previous.el).hidden = true;
    $(previous.button).classList.remove('on');
    $(previous.button).setAttribute('aria-pressed', 'false');
  }
  if (kind === 'tree') S.treeReturn = document.activeElement;
  S.overlay = kind;
  document.body.dataset.overlay = kind ?? '';
  const overlay = OVERLAYS[kind];
  if (overlay) {
    if (overlay.leavesStart) setSurface('reading');
    hideList();
    closePopover(false);
    $(overlay.el).hidden = false;
    $(overlay.button).classList.add('on');
    $(overlay.button).setAttribute('aria-pressed', 'true');
    overlay.show(true);
  }
  setLayer(restingLayer());
  syncLayer();
}

function closeOverlay() {
  const trigger = S.overlay === 'tree' ? S.treeReturn : null;
  setOverlay(null);
  if (trigger?.isConnected && trigger !== document.body) {
    trigger.focus();
    return;
  }
  if (pageShows()) post({ t: 'focus', target: 'page' });
  else if (S.surface === 'start') focusStart();
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

function focusStart() {
  post({ t: 'focus', target: 'chrome' });
  $('start-search').focus({ preventScroll: true });
  $('start').scrollTop = 0;
}

function renderPlaceholder() {
  $('strip-placeholder').hidden = !(S.surface === 'start' && !current());
}

function bandHead(label, hint = '', mark = '') {
  return `<div class="band-head">${mark ? icon(mark) : ''}<span class="grow">${esc(label)}</span><span class="hint">${esc(hint)}</span></div>`;
}

function miniHtml(mini) {
  const edges = mini.edges.map((e) => `<path class="${e.wait ? 'wait' : ''}" d="M ${e.c[0]} C ${e.c[1]} ${e.c[2]} ${e.c[3]}"/>`).join('');
  const stub = mini.cut ? `<path class="wait" d="M -6 ${mini.s / 2} H 0"/>` : '';
  return `<span class="mt" style="width:${mini.w}px;height:${mini.h}px"><svg width="${mini.w}" height="${mini.h}">${edges}${stub}</svg>`
    + mini.nodes.map((n) => `<span class="mt-node${n.last ? ' last' : ''}${n.waiting ? ' waiting' : ''}" style="left:${n.x}px;top:${n.y}px">${fav(visit(n.id).url, !n.waiting, mini.s)}</span>`).join('') + '</span>';
}

function pinnedBand(start) {
  return '<section>' + bandHead('Pinned trees', 'Opens the tree where you left it')
    + (start.pinned.length ? '<div class="pinned-row">' + start.pinned.map((t) =>
      `<button class="pinned-card" data-start="tree" data-id="${t.id}" title="${esc(t.tip)}">`
      + `<span class="pc-tree">${miniHtml(t.mini)}</span><span class="pc-foot"><span class="pc-name">${esc(t.title)}</span>`
      + (t.waiting ? `<span class="count" title="${t.waiting} waiting">${t.waiting}</span>` : '')
      + `</span><span class="pc-meta">${esc(t.meta)}</span></button>`).join('') + '</div>'
      : '<div class="hint-line">Pin a tree from its tab menu. It stays here and never settles on its own.</div>') + '</section>';
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
    + (b.empty ? '<div class="hint-line">Ctrl+D bookmarks the page you are on.</div>' : `<div class="start-tiles">${tiles}${more}</div>`) + '</section>';
}

function togglePopover(kind, anchor, data) {
  if (S.popover?.kind === kind && S.popover.anchor === anchor) { closePopover(false); anchor.focus(); return; }
  const origin = anchor.closest('#popover') ? S.popover.anchor : anchor;
  openPopover(kind, origin, data);
  post({ t: 'focus', target: 'chrome' });
  $('popover').querySelector('button')?.focus();
}

const BANDS = [pinnedBand, bookmarksBand, suggestedBand];
const START = {
  bookmark: { open: (id) => { closePopover(false); uncover(); intent('open-bookmark', { id: Number(id) }); }, menu: 'bookmark' },
  folder: { open: (name, el) => togglePopover('folder', el, { folder: name }) },
  more: { open: (_, el) => togglePopover('folder', el, { more: true }) },
  folders: { open: (_, el) => togglePopover('folder', el, { hidden: true }) },
  tree: { open: (id) => { uncover(); intent('select-tree', { id }); }, menu: 'unpin' },
  suggestion: { open: (key) => { intent('open-suggestion', { key }); uncover(); } },
  keep: { open: (key) => intent('keep-suggestion', { key }) },
  decline: { open: (key) => intent('decline-suggestion', { key }) },
};

function renderStart() {
  if (!S.frame) return;
  const paint = JSON.stringify([S.frame.start, S.rev]);
  if (S.startPaint === paint) return;
  S.startPaint = paint;
  const focused = document.activeElement?.closest('[data-start]');
  const anchor = S.popover?.anchor.closest('[data-start]');
  const find = (el) => el && [...$('start-content').querySelectorAll('[data-start]')].find((n) => n.dataset.start === el.dataset.start && n.dataset.id === el.dataset.id);
  $('start-content').innerHTML = BANDS.map((band) => band(S.frame.start)).join('');
  find(focused)?.focus();
  if (anchor) {
    const next = find(anchor);
    if (next) S.popover.anchor = next;
    else closePopover(false);
  }
}

// ---------- forest view ----------

function forestLayout() {
  return S.frame?.forest[S.forestLabels ? 'labels' : 'icons'];
}

function renderForest() {
  if (S.overlay !== 'forest' || !S.frame) {
    S.forestDirty = true;
    return;
  }
  S.forestDirty = false;
  const frame = S.frame;
  const layout = forestLayout();
  $('forest-count').textContent = frame.forest.count;
  $('forest-titles').classList.toggle('on', S.forestLabels);
  const path = new Set();
  for (let id = current(); id; id = visit(id)?.parent) path.add(id);
  const positions = new Map(layout.nodes.map((n) => [n.id, n]));
  let maxX = 0, maxY = 0;
  const nodes = layout.nodes.map((n) => {
    const v = visit(n.id);
    if (!v) return '';
    maxX = Math.max(maxX, n.x + n.w / 2);
    maxY = Math.max(maxY, n.y + n.s / 2);
    const cls = `forest-node${n.settled ? ' settled' : ''}${S.forestLabels ? '' : ' icon-only'}${path.has(n.id) ? ' path' : ''}${markClass(v)}`;
    return `<button class="${cls}" data-id="${n.id}" title="${esc(v.label + (v.live ? ' · Open in memory' : ''))}" style="left:${n.x - n.w / 2}px;top:${n.y - n.s / 2}px;width:${n.w}px;height:${n.s}px">`
      + markedFav(v, 16, false) + (v.live ? '<i class="live-dot"></i>' : '') + (S.forestLabels ? `<span>${esc(v.label)}</span>` : '') + '</button>';
  }).join('');
  const heads = layout.heads.map((h) => `<button class="forest-head${h.settled ? ' settled' : ''}" data-id="${h.id}" style="top:${h.y}px"><b>${esc(h.title)}</b><span>${esc(h.detail)}</span></button>`).join('');
  const edges = layout.nodes.map((b) => {
    const v = visit(b.id);
    const a = v?.parent && positions.get(v.parent);
    if (!a) return '';
    const x = a.x + a.w / 2, end = b.x - b.w / 2, dx = (end - x) / 2;
    const cls = path.has(b.id) ? 'path' : v.queued ? 'pending' : '';
    return `<path class="${cls}" d="M${x},${a.y} C${x + dx},${a.y} ${end - dx},${b.y} ${end},${b.y}"/>`;
  }).join('');
  const svg = $('forest-edges');
  svg.setAttribute('width', maxX + 40);
  svg.setAttribute('height', maxY + 40);
  svg.innerHTML = edges;
  const d = layout.divider;
  const divider = d ? `<div class="forest-divider" style="top:${d.y}px"><b>${esc(d.title)}</b><span>${esc(d.detail)}</span></div>` : '';
  const content = heads + divider + nodes;
  const layer = $('forest-nodes');
  if (layer.__content !== content) {
    layer.innerHTML = content;
    layer.__content = content;
  }
  applyCamera();
}

function applyCamera() {
  const c = S.camera;
  $('forest-layer').style.transform = `translate(${c.x}px, ${c.y}px) scale(${c.zoom})`;
}

let cameraTween = 0;
function fitForest(animate) {
  const layout = forestLayout();
  if (!layout) return;
  const canvas = $('forest-canvas');
  const to = { x: 0, y: 0, zoom: clamp((canvas.clientWidth - 20) / Math.max(layout.width, 1), 0.3, 1) };
  cancelAnimationFrame(cameraTween);
  if (!animate) {
    S.camera = to;
    applyCamera();
    return;
  }
  const from = { ...S.camera };
  const start = performance.now();
  const frame = (now) => {
    const t = (now - start) / 220;
    const k = ease(t);
    S.camera = { x: mix(from.x, to.x, k), y: mix(from.y, to.y, k), zoom: mix(from.zoom, to.zoom, k) };
    applyCamera();
    if (t < 1) cameraTween = requestAnimationFrame(frame);
  };
  cameraTween = requestAnimationFrame(frame);
}

(() => {
  const canvas = $('forest-canvas');
  let drag = null;
  canvas.addEventListener('mousedown', (event) => {
    if (event.button !== 0) return;
    drag = { x: event.clientX, y: event.clientY, camera: { ...S.camera }, moved: false };
  });
  window.addEventListener('mousemove', (event) => {
    if (!drag) return;
    const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) {
      drag.moved = true;
      canvas.classList.add('dragging');
    }
    if (drag.moved) {
      S.camera = { ...drag.camera, x: drag.camera.x + dx, y: drag.camera.y + dy };
      applyCamera();
    }
  });
  window.addEventListener('mouseup', () => {
    if (drag?.moved) setTimeout(() => { drag = null; }, 0);
    else drag = null;
    canvas.classList.remove('dragging');
  });
  canvas.addEventListener('click', (event) => {
    if (drag?.moved) return;
    const node = event.target.closest('[data-id]');
    if (!node) return;
    activate(node.dataset.id);
  });
  canvas.addEventListener('wheel', (event) => {
    event.preventDefault();
    const c = S.camera;
    if (event.ctrlKey) {
      const zoom = clamp(c.zoom * Math.exp(-event.deltaY * 0.0012 * (event.deltaMode ? 33 : 1)), 0.3, 1.6);
      const box = canvas.getBoundingClientRect();
      const px = event.clientX - box.left, py = event.clientY - box.top;
      const ratio = zoom / c.zoom;
      S.camera = { zoom, x: px - (px - c.x) * ratio, y: py - (py - c.y) * ratio };
    } else {
      S.camera = { ...c, x: c.x - event.deltaX, y: c.y - event.deltaY };
    }
    applyCamera();
  }, { passive: false });
})();

$('forest-titles').addEventListener('click', () => {
  S.forestLabels = !S.forestLabels;
  renderForest();
  fitForest(true);
});
$('forest-fit').addEventListener('click', () => fitForest(true));
$('forest-close').addEventListener('click', closeOverlay);
$('forest').querySelector('.scrim').addEventListener('mousedown', closeOverlay);

// ---------- reader ----------

function showReader(html, list) {
  closePalette(false);
  closePopover(false);
  hideList();
  const box = $('reader-box');
  box.className = `reader-box${list ? ' list' : ''}`;
  box.innerHTML = html + `<button class="icon-button reader-close" aria-label="Close">${icon('close')}</button>`;
  box.querySelector('.reader-close').addEventListener('click', closeReader);
  S.reader = true;
  $('reader').hidden = false;
  syncLayer();
}

function closeReader(returnFocus = true) {
  if (!S.reader) return;
  S.reader = null;
  $('reader').hidden = true;
  if (returnFocus && pageShows()) post({ t: 'focus', target: 'page' });
  syncLayer();
}
$('reader').querySelector('.scrim').addEventListener('mousedown', closeReader);

const when = (at) => new Date(at).toLocaleString(undefined, { day: '2-digit', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });

// ---------- Rust's side ----------

window.canopy = {
  viewport(height) { S.windowHeight = height; document.documentElement.style.setProperty('--window-height', `${height}px`); positionList(); syncLayer(); },
  start(value) { if (!S.frame) return; S.frame.start = value; renderStart(); },
  present(frame) {
    const previous = current();
    S.frame = frame;
    applyStrip(frame.strip);
    renderWaitingPin();
    applyTheme();
    if (frame.current !== previous) {
      hideList();
      if (S.overlay !== 'tree') setOverlay(null);
      if (S.surface !== 'reading') setSurface('reading');
    }
    if (!frame.tabs.length && S.surface === 'reading') setSurface('start');
    if (S.popover?.kind === 'rename' && !frame.tabs.some((tree) => tree.id === S.popover.data)) closePopover();
    renderTabs();
    animateStrip(previous === null && strip.shown.nodes.size === 0 ? 0 : MOVE);
    renderStart();
    renderPlaceholder();
    renderForest();
    renderSheet(false);
    if (S.list) { renderList(); requestListPreview(); }
    if (S.popover) refreshPopover();
    renderPaletteContext();
    syncLayer();
  },
  strip(layouts) {
    applyStrip(layouts);
    renderWaitingPin();
    animateStrip(0);
    fitTabs();
  },
  icons(revision) {
    S.rev = revision;
    renderPaletteContext();
    if (!S.frame) return;
    renderTabs();
    renderWaitingPin();
    if (S.popover?.kind === 'waiting' || S.popover?.kind === 'node') refreshPopover();
    buildStrip(strip.target ?? targetLayout());
    renderStart();
    renderForest();
  },
  logins(offer) {
    S.logins = offer.names.length ? offer : null;
    renderLoginChip();
  },
  loginsLocked(keeper) {
    S.locked = keeper;
    renderLoginChip();
  },
  saveLogin(offer) {
    S.save = offer;
    renderLoginChip();
  },
  fillLogin() { fillLogin(); },
  nextWaiting() { openNextWaiting(); },
  openPalette(options) { openPalette(options); },
  newTree() {
    intent('cancel');
    closePalette(false);
    setOverlay(null);
    setSurface('start');
    focusStart();
  },
  toggleForest() { toggleOverlay('forest'); },
  workbench(view) {
    if (!S.frame) return;
    const id = document.activeElement?.closest('[data-id]')?.dataset.id;
    S.frame.treeView = view;
    renderSheet(false);
    if (id && !document.activeElement?.closest('[data-id]')) focusTree(id);
  },
  toggleTree(fromChrome = true) { toggleOverlay('tree'); if (!fromChrome && S.overlay === 'tree') S.treeReturn = null; },
  toast(kind, text) {
    if (kind === 'error') {
      S.errors.unshift(text);
      S.errors.length = Math.min(S.errors.length, 5);
    }
    const toast = $('toast');
    toast.textContent = text;
    toast.setAttribute('role', kind === 'error' ? 'alert' : 'status');
    if (kind === 'settled') {
      const button = document.createElement('button');
      button.textContent = 'Undo';
      button.onclick = () => { toast.hidden = true; syncLayer(); intent('undo-settle'); };
      toast.appendChild(button);
    }
    toast.className = kind;
    toast.hidden = false;
    syncLayer();
    clearTimeout(timers.toast);
    timers.toast = setTimeout(() => {
      toast.classList.add('leaving');
      timers.toast = setTimeout(() => { toast.hidden = true; syncLayer(); }, 200);
    }, kind === 'settled' ? 6000 : kind === 'error' ? 5000 : 2600);
  },
  toggleFocus() {
    const active = document.activeElement;
    if (document.hasFocus() && active?.classList.contains('node')) {
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
  settleCurrent() {
    if (S.reader || S.palette || S.popover || S.overlay || (S.surface !== 'reading' && current())) {
      this.escape();
    } else if (current()) {
      intent('settle-current');
    }
  },
  /// Closes the topmost thing Escape owns.
  formReturn(open) {
    S.formReturn = open;
    $('form-return').hidden = !open;
    syncLayer();
    if (open) $('form-return-cancel').focus();
  },
  escape() {
    if (S.formReturn) answerFormReturn(false);
    else if (S.reader) closeReader();
    else if (S.palette) closePalette();
    else if (S.popover) closePopover();
    else if (S.list) hideList();
    else if (S.overlay) closeOverlay();
    else if (S.surface !== 'reading' && current()) {
      setSurface(S.frame?.tabs.length ? 'reading' : 'start');
      post({ t: 'focus', target: 'page' });
    }
  },
  paletteResults(seq, { rows, first }) {
    if (!S.palette || seq !== S.palette.seq) return;
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
      + `<button class="action" data-act="source" id="capture-restore-source"${c.parent ? '' : ' disabled'}>Back to source</button>`
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
  /// For tests: what the view shows, without reaching into the DOM. `deferred` names the
  /// calls held back while a button is down.
  state() {
    return {
      start: S.frame?.start, deferred: [...refresh.pending.keys()],
      surface: S.surface, overlay: S.overlay, treeSide: S.treeSide, animating: !!strip.anim, list: S.list?.key ?? null, layer: S.layer, palette: !!S.palette,
      reader: !!S.reader, popover: S.popover?.kind ?? null, reported: S.reported, logins: loginNames(), save: S.save, locked: S.locked,
      dark: document.documentElement.classList.contains('dark'),
      nodes: [...strip.elements.keys()], rows: S.palette?.rows ?? [],
      waiting: S.frame?.waiting.ids ?? [], pins: (S.strip?.pins ?? []).map((p) => p.kind), room: S.strip?.room ?? null,
    };
  },
};

// A release must finish dispatching its click before any pressed element is replaced.
const refresh = { buttons: 0, pending: new Map(), release: null };
function rebuild(kind, apply, args) {
  if (!refresh.buttons) { apply(...args); return; }
  refresh.pending.delete(kind);
  refresh.pending.set(kind, { apply, args });
}
function releaseRefresh() {
  clearTimeout(refresh.release);
  refresh.release = setTimeout(() => {
    refresh.buttons = 0;
    const pending = [...refresh.pending.values()];
    refresh.pending.clear();
    for (const { apply, args } of pending) apply(...args);
  });
}
for (const kind of ['present', 'strip', 'icons', 'start', 'logins', 'loginsLocked', 'saveLogin', 'paletteResults', 'previewResult', 'reader', 'readerList']) {
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
window.addEventListener('blur', () => { releaseRefresh(); hideList(); });
document.addEventListener('focusout', (event) => {
  if (!S.list) return;
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
    openPopover(menu, el, el.dataset.id);
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
$('agent').innerHTML = icon('spark');
$('forest-button').innerHTML = icon('forest') + '<span>Forest</span>';
$('settings-button').innerHTML = icon('settings');
$('tab-overflow').innerHTML = icon('chevron');
$('search-pill').insertAdjacentHTML('afterbegin', icon('search'));

$('tab-row').addEventListener('click', (event) => {
  const close = event.target.closest('[data-settle]');
  if (close) { intent('settle-tree', { id: close.dataset.settle }); return; }
  const tab = event.target.closest('[data-id]');
  if (!tab) return;
  uncover();
  intent('select-tree', { id: tab.dataset.id });
});
$('tab-row').addEventListener('mousedown', (event) => {
  if (event.button === 1 && event.target.closest('[data-id]')) event.preventDefault();
});
$('tab-row').addEventListener('auxclick', (event) => {
  const tab = event.target.closest('[data-id]');
  if (event.button === 1 && tab) intent('settle-tree', { id: tab.dataset.id });
});
$('tab-row').addEventListener('contextmenu', (event) => {
  const tab = event.target.closest('[data-id]');
  if (!tab) return;
  event.preventDefault();
  openPopover('tab', tab, tab.dataset.id);
  post({ t: 'focus', target: 'chrome' });
  $('popover').querySelector('button')?.focus();
});
$('tab-overflow').addEventListener('click', (event) => {
  event.stopPropagation();
  if (S.popover?.kind === 'trees') closePopover();
  else openPopover('trees', $('tab-overflow'));
});
$('new-tree').addEventListener('click', () => window.canopy.newTree());
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
  if (event.relatedTarget?.closest('#strip-nodes')) return;
  schedule('hideList', 220, hideList);
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

post({ t: 'ready' });

function answerFormReturn(proceed, focusPage = true) {
  if (!S.formReturn) return;
  const question = S.formReturn;
  canopy.formReturn(null);
  post({ t: 'form-return', question, proceed });
  if (focusPage) post({ t: 'focus', target: 'page' });
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
  if (!S.formReturn || !['Tab', 'ISO_Left_Tab'].includes(event.key)) return;
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

document.addEventListener('contextmenu', (event) => {
  const field = event.target.closest('input, textarea, [contenteditable="true"]');
  if (field) return;
  if (event.defaultPrevented) return;
  event.preventDefault();
  const el = event.target.closest('#strip-nodes [data-page], #strip-list [data-id], #strip-list [data-page], #tree-nodes [data-id], #tree-list [data-id], #popover.waiting [data-id], #forest-nodes [data-id]');
  const id = el?.dataset.page || el?.dataset.id;
  if (!id || !visit(id)) return;
  openPopover('node', el, id);
  hideList();
});
