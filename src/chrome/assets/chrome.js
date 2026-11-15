// Canopy's chrome view. Rust sends state with `canopy.present` and friends; this script
// paints it and reports input back with `post`. It decides nothing about the forest: layouts,
// palette results and intents all come from Rust.
'use strict';

const HEADER = 42, REST = 52, OPEN = 120, TRUNK_Y = 26, CARD_WIDTH = 304, ACCESSORY = 22;
const MOVE = 300, RESIZE = 220;
/// A page with more files than this shows one leaf and their count.
const LEAVES = 3;
const MONOGRAM = ['#d95c42', '#3b7ad6', '#2e9e5e', '#9c6bde', '#d9962e', '#3b99ad', '#d1547a', '#6b7380'];
const ICONS = {
  plus: '<path d="M12 5v14M5 12h14" stroke-width="1.9"/>',
  search: '<circle cx="11" cy="11" r="6.5" stroke-width="1.9"/><path d="M20 20l-4.2-4.2" stroke-width="1.9"/>',
  spark: '<path d="M11 2.5l1.9 5.6 5.6 1.9-5.6 1.9L11 17.5l-1.9-5.6L3.5 10l5.6-1.9z" fill="currentColor" stroke="none"/><path d="M18.5 14l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9z" fill="currentColor" stroke="none" opacity=".7"/>',
  forest: '<circle cx="4.5" cy="12" r="2" stroke-width="1.8"/><circle cx="19.5" cy="5.5" r="2" stroke-width="1.8"/><circle cx="19.5" cy="12" r="2" stroke-width="1.8"/><circle cx="19.5" cy="18.5" r="2" stroke-width="1.8"/><path d="M6.5 12h11M10 12c2.5 0 3-6.5 7.5-6.5M10 12c2.5 0 3 6.5 7.5 6.5" stroke-width="1.8"/>',
  chevron: '<path d="M7 10l5 5 5-5" stroke-width="2"/>',
  globe: '<circle cx="12" cy="12" r="8.5" stroke-width="1.7"/><path d="M3.5 12h17M12 3.5c2.6 2.6 2.6 14.4 0 17M12 3.5c-2.6 2.6-2.6 14.4 0 17" stroke-width="1.7"/>',
  close: '<path d="M6 6l12 12M18 6L6 18" stroke-width="1.9"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" stroke-width="1.7"/><path d="M14 3v5h5" stroke-width="1.7"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" stroke-width="1.7"/>',
  settings: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" stroke-width="1.6"/><circle cx="12" cy="12" r="3" stroke-width="1.6"/>',
};

const $ = (id) => document.getElementById(id);
const post = (message) => window.ipc.postMessage(JSON.stringify(message));
/// Intents that load a page also give the keyboard back to it, as the GTK buttons that
/// never took focus did.
const NAVIGATES = new Set(['restore', 'select-tree', 'settle-tree', 'settle-current', 'shortcut', 'path', 'open-citation', 'open-saved', 'branch-queued']);
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
  strip: null,
  rev: 0,
  systemDark: null,
  surface: 'reading',
  open: false,
  layer: 'none',
  hovered: null,
  card: null,
  popover: null,
  palette: null,
  reader: null,
  errors: [],
  manualPan: null,
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
const visitFav = (v, size) => fav(v.url, !v.queued, size);

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

function enterStrip() {
  if (S.layer === 'palette') return;
  cancel('close');
  if (!S.open) schedule('open', 110, () => { if (!S.open && S.layer === 'none') setOpen(true); });
}
function leaveStrip() {
  cancel('open');
  if (S.open) schedule('close', 380, closeStrip);
}
function closeStrip() {
  if (S.open && S.layer !== 'menu') {
    setOpen(false);
    hideCard();
  }
}
function enterNode(id) {
  if (S.layer !== 'none') return;
  S.hovered = id;
  cancel('hideCard');
  schedule('showCard', S.card ? 40 : 260, () => {
    if (S.open && S.layer === 'none' && S.hovered && S.card !== S.hovered) showCard(S.hovered);
  });
}
function leaveNode() {
  S.hovered = null;
  cancel('showCard');
  if (S.card) schedule('hideCard', 160, hideCard);
}
function setLayer(layer) {
  const previous = S.layer;
  S.layer = layer;
  if (layer === 'palette') {
    cancel('open');
    cancel('close');
    if (S.open) setOpen(false);
    hideCard();
  } else if (layer === 'menu') {
    cancel('close');
    hideCard();
  } else if (previous === 'menu' && S.open) {
    schedule('close', 380, closeStrip);
  }
  syncLayer();
}

/// Tells Rust how far down the view must reach (null for the whole window), and whether
/// Escape is ours. Below that, the page gets the pointer.
function syncLayer() {
  const modal = !!(S.reader || S.palette || S.popover || S.card || S.open
    || (S.surface !== 'reading' && current()));
  let height = HEADER + REST;
  if (S.palette || S.reader || S.surface !== 'reading') {
    height = null;
  } else {
    if (S.open || strip.anim) {
      const strips = [strip.shown.height, strip.target?.height ?? REST, strip.anim ? strip.from.height : REST];
      height = Math.max(height, HEADER + Math.max(...strips) + 24);
    }
    for (const el of [S.card && $('card'), S.popover && $('popover')]) {
      if (!el) continue;
      const bottom = el.hidden ? HEADER + OPEN + 260 : el.getBoundingClientRect().bottom;
      height = Math.max(height, bottom + 30);
    }
    height = Math.ceil(height);
  }
  const report = `${height}/${modal}`;
  if (report !== S.reported) {
    S.reported = report;
    post({ t: 'layer', height, modal });
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
      + (root ? visitFav(root, 14) : '')
      + `<span class="tab-title">${esc(tree.title)}</span>`
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
  dots: new Map(),
  badges: new Map(),
  leaves: new Map(),
};

const targetLayout = () => S.strip?.[S.open ? 'open' : 'rest'] ?? null;

function setOpen(open) {
  S.open = open;
  S.manualPan = null;
  syncLayer();
  animateStrip(RESIZE);
}

function animateStrip(duration) {
  const layout = targetLayout();
  if (!layout) return;
  strip.from = strip.shown;
  strip.target = layout;
  buildStrip(layout);
  if (!duration) {
    strip.anim = null;
    step(performance.now(), 1);
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
    for (const [id, el] of strip.elements) {
      if (!strip.target.nodes.some((n) => n.id === id)) {
        el.remove();
        strip.elements.delete(id);
        strip.dots.get(id)?.remove();
        strip.dots.delete(id);
        strip.badges.get(id)?.remove();
        strip.badges.delete(id);
        strip.leaves.get(id)?.remove();
        strip.leaves.delete(id);
      }
    }
    syncLayer();
  }
}

function iconSize(kind, size) {
  if (kind === 'pill') return 16;
  if (size <= 14) return Math.round(size);
  if (size >= 30) return 18;
  if (size >= 24) return 16;
  if (size >= 20) return 13;
  return 12;
}

/// One element per node, kept across frames so hovers and transitions survive a re-layout.
function buildStrip(layout) {
  const nodes = $('strip-nodes');
  const canvas = $('strip-canvas');
  for (const n of layout.nodes) {
    const v = visit(n.id);
    if (!v) continue;
    let el = strip.elements.get(n.id);
    if (!el) {
      el = document.createElement('button');
      el.className = 'node';
      el.dataset.id = n.id;
      el.addEventListener('mouseenter', () => enterNode(el.dataset.id));
      el.addEventListener('mouseleave', leaveNode);
      el.addEventListener('click', () => activate(el.dataset.id));
      el.addEventListener('keydown', walkTrunk);
      nodes.appendChild(el);
      strip.elements.set(n.id, el);
    }
    el.className = `node ${n.kind}` + (n.kind === 'tile' && n.s < 24 ? ' sm' : '') + (v.queued ? ' pending' : '');
    el.setAttribute('aria-label', `${v.label}, ${v.site}`);
    const html = n.kind === 'bud' ? ''
      : n.kind === 'pill'
        ? visitFav(v, 16) + `<span class="pill-text"><span class="pill-title">${esc(v.label)}</span><span class="pill-site">${esc(v.site)}</span></span>`
        : visitFav(v, iconSize(n.kind, n.s));
    if (el.dataset.html !== html) {
      el.innerHTML = html;
      el.dataset.html = html;
    }
    let dot = strip.dots.get(n.id);
    if (v.queued && n.kind !== 'bud') {
      if (!dot) {
        dot = document.createElement('div');
        dot.className = 'pending-dot';
        canvas.appendChild(dot);
        strip.dots.set(n.id, dot);
      }
    } else if (dot) {
      dot.remove();
      strip.dots.delete(n.id);
    }
    let badge = strip.badges.get(n.id);
    if (n.hidden.length) {
      if (!badge) {
        badge = document.createElement('button');
        badge.className = 'badge';
        badge.addEventListener('click', (event) => {
          event.stopPropagation();
          openPopover('hidden', badge, JSON.parse(badge.dataset.hidden));
        });
        canvas.appendChild(badge);
        strip.badges.set(n.id, badge);
      }
      badge.textContent = `+${n.hidden.length}`;
      badge.dataset.hidden = JSON.stringify(n.hidden);
      badge.setAttribute('aria-label', `${n.hidden.length} more pages`);
    } else if (badge) {
      badge.remove();
      strip.badges.delete(n.id);
    }
    // A bud is too small to carry leaves; they show once the strip opens.
    const files = n.kind === 'bud' ? [] : filesOf(n.id);
    let leaves = strip.leaves.get(n.id);
    if (files.length) {
      if (!leaves) {
        leaves = document.createElement('button');
        leaves.dataset.id = n.id;
        leaves.addEventListener('click', (event) => {
          event.stopPropagation();
          const id = leaves.dataset.id;
          if (S.popover?.kind === 'files' && S.popover.data === id) closePopover();
          else openPopover('files', leaves, id, strip.elements.get(id));
        });
        canvas.appendChild(leaves);
        strip.leaves.set(n.id, leaves);
      }
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
  const pan = S.manualPan ?? mix(from.pan, to.pan, k);
  strip.shown = { height: mix(from.height, to.height, k), pan, nodes, edges, width: to.width };
  drawStrip();
}

function drawStrip() {
  const shown = strip.shown;
  const width = window.innerWidth;
  const openness = clamp((shown.height - REST) / (OPEN - REST), 0, 1);
  $('strip').style.height = `${shown.height}px`;
  $('strip-clip').style.height = `${shown.height}px`;
  const bg = $('strip-bg');
  bg.style.height = `${shown.height}px`;
  bg.style.background = `color-mix(in srgb, var(--chrome) ${Math.round(openness * 100)}%, var(--bg))`;
  bg.style.borderRadius = `0 0 ${12 * openness}px ${12 * openness}px`;
  bg.style.boxShadow = openness > 0 ? `0 ${10 * openness}px ${24 * openness}px var(--shadow)` : 'none';
  bg.style.setProperty('--rest-line', String(1 - openness));
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
    const dot = strip.dots.get(id);
    if (dot) {
      const raw = el.classList.contains('raw');
      const size = raw ? 5 : 7, inset = raw ? 2 : 3;
      dot.style.width = dot.style.height = `${size}px`;
      dot.style.left = `${r.x + r.w / 2 + inset - size}px`;
      dot.style.top = `${r.y - r.s / 2 - inset}px`;
      dot.style.opacity = r.o;
    }
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
    const reach = 40 + 12 * openness;
    const x = r.x - r.w / 2 + 16;
    Object.assign(glow.style, { left: `${x - reach}px`, top: `${TRUNK_Y - reach}px`, width: `${2 * reach}px`, height: `${2 * reach}px` });
  }
  const opacity = new Map([...shown.nodes].map(([id, n]) => [id, n.o]));
  $('strip-edges').innerHTML = [...shown.edges.values()].map((e) => {
    const [p0, p1, p2, p3] = e.c;
    const o = e.kind === 'path' ? 1 : (opacity.get(e.to) ?? 1);
    return `<path class="${e.kind}" style="opacity:${o}" d="M${p0} C${p1} ${p2} ${p3}"/>`;
  }).join('');
  const fill = openness > 0.5 ? 'var(--chrome)' : 'var(--bg)';
  const left = $('fade-left'), right = $('fade-right');
  left.hidden = shown.pan >= -1;
  right.hidden = !(shown.width + shown.pan > width + 1);
  for (const [el, side] of [[left, 'left'], [right, 'right']]) {
    el.style.height = `${shown.height - 2}px`;
    el.style[side] = '0';
    el.style.background = `linear-gradient(to ${side === 'left' ? 'right' : 'left'}, ${fill}, transparent)`;
  }
}

function activate(id) {
  hideCard();
  if (id !== current()) {
    setSurface('reading');
    intent('restore', { id });
  }
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

// ---------- card ----------

let previewSeq = 0;
function showCard(id) {
  S.card = id;
  previewSeq += 1;
  post({ t: 'preview', id, seq: previewSeq });
  syncLayer();
}

function hideCard() {
  cancel('showCard');
  cancel('hideCard');
  if (!S.card) return;
  S.card = null;
  $('card').hidden = true;
  syncLayer();
}

function renderCard(preview) {
  const id = preview.id;
  const v = visit(id);
  const r = strip.shown.nodes.get(id);
  if (!v || !r || S.card !== id) return;
  const isCurrent = id === current();
  const tags = [isCurrent && 'Current page', v.queued && 'Pending'].filter(Boolean);
  const source = isCurrent && v.source;
  const card = $('card');
  card.innerHTML = `<div class="card-head">${visitFav(v, 16)}<div class="card-title">${esc(preview.title)}</div></div>`
    + '<div class="card-body">'
    + `<div class="card-meta">${esc(preview.meta)}</div>`
    + (tags.length ? `<div class="card-tags">${tags.map((t) => `<span class="tag">${t}</span>`).join('')}</div>` : '')
    + (preview.snippet ? `<div class="card-snippet">${esc(preview.snippet)}</div>` : '')
    + (source ? `<div class="card-source"><div class="card-meta">${esc(source.text)}</div><div class="card-actions">`
      + `<button class="action" data-act="source"${v.parent ? '' : ' disabled'}>Back to source</button>`
      + `<button class="action" data-act="comment"${source.citation ? '' : ' disabled'}>Open comment</button></div></div>` : '')
    + '<div class="card-actions">'
    + `<button class="action" data-act="open">${isCurrent ? 'Reload' : 'Open'}</button>`
    + '<button class="action" data-act="branch">New branch here</button>'
    + '</div></div>';
  const x = r.x + strip.shown.pan;
  card.style.left = `${clamp(x - CARD_WIDTH / 2, 8, window.innerWidth - CARD_WIDTH - 8)}px`;
  card.style.top = `${HEADER + OPEN + 8}px`;
  card.hidden = false;
  syncLayer();
  card.querySelectorAll('[data-act]').forEach((button) => button.addEventListener('click', () => {
    const act = button.dataset.act;
    hideCard();
    if (act === 'open') {
      setSurface('reading');
      intent('restore', { id });
    } else if (act === 'branch') {
      if (v.queued) intent('branch-queued', { id });
      else openPalette({ scope: id, text: '' });
    } else if (act === 'source') {
      intent('restore', { id: v.parent });
    } else if (act === 'comment') {
      intent('open-citation', { id });
    }
  }));
}

// ---------- popovers: Memory menu, hidden pages, trees that do not fit, a tree's name ----------

/// Opens a menu under `anchor`, or under `below` when the anchor sits on top of something
/// the menu should not cover.
function openPopover(kind, anchor, data, below = anchor) {
  const popover = $('popover');
  S.popover = { kind, anchor, data };
  renderPopover();
  popover.hidden = false;
  const a = anchor.getBoundingClientRect();
  const width = popover.offsetWidth;
  popover.style.left = `${clamp(a.left, 8, window.innerWidth - width - 8)}px`;
  popover.style.top = `${below.getBoundingClientRect().bottom + 6}px`;
  if (S.layer !== 'palette') setLayer('menu');
  syncLayer();
}

function closePopover(returnFocus = true) {
  if (!S.popover) return;
  S.popover = null;
  $('popover').hidden = true;
  if (S.layer === 'menu') setLayer('none');
  if (returnFocus && S.surface === 'reading' && !S.palette && !S.reader) post({ t: 'focus', target: 'page' });
  syncLayer();
}

function visitRow(v) {
  const detail = v.queued ? 'pending' : v.site.split(' · ').slice(1).join(' · ');
  return `<button class="menu-row" data-id="${v.id}">${visitFav(v, 16)}<span class="grow">${esc(v.label)}</span><span class="menu-detail">${esc(detail)}</span></button>`;
}

function renderPopover() {
  const popover = $('popover');
  const { kind, data } = S.popover;
  if (kind === 'hidden') {
    popover.innerHTML = data.map(visit).filter(Boolean).map(visitRow).join('');
    popover.querySelectorAll('[data-id]').forEach((row) => row.addEventListener('click', () => {
      closePopover();
      activate(row.dataset.id);
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
      setSurface('reading');
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
    const files = filesOf(data);
    if (!files.length) {
      closePopover(false);
      return;
    }
    popover.innerHTML = `<div class="menu-heading">${files.length === 1 ? 'Downloaded from this page' : `${files.length} files downloaded from this page`}</div>`
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

function openPalette({ scope, text }) {
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
  input.select();
  queryPalette();
  syncLayer();
}

function renderPaletteContext() {
  if (!S.palette) return;
  const v = S.palette.scope && visit(S.palette.scope);
  $('palette-context').innerHTML = v
    ? `From ${visitFav(v, 12)}<span>${esc(v.label)}</span>`
    : '<span class="tag">New tree</span>';
}

function closePalette(returnFocus = true) {
  if (!S.palette) return;
  clearTimeout(paletteTimer);
  S.palette = null;
  $('palette').hidden = true;
  $('palette-results').innerHTML = '';
  setLayer(S.surface === 'reading' ? 'none' : 'palette');
  if (returnFocus && S.surface === 'reading') post({ t: 'focus', target: 'page' });
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
  if (reading) setSurface('reading');
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

// ---------- surfaces ----------

function setSurface(surface) {
  if (S.surface === surface) return;
  S.surface = surface;
  $('start').hidden = surface !== 'start';
  $('forest').hidden = surface !== 'forest';
  setLayer(surface === 'reading' ? 'none' : 'palette');
  if (surface === 'forest') {
    renderForest();
    fitForest(false);
  }
  syncLayer();
}

function renderStart() {
  const frame = S.frame;
  if (!frame) return;
  const shortcuts = frame.shortcuts.map((s) => `<button class="shortcut-tile" data-shortcut="${s.index}" title="${esc(s.url)}">`
    + `<span class="glyph">${fav(s.url, true, 24)}</span><span class="label">${esc(s.label)}</span></button>`).join('');
  const paths = frame.paths.map((p) => `<button class="menu-row path-row" data-path="${p.index}" title="${esc(`${p.first}\n${p.second}`)}">`
    + `<span class="chain"><span class="glyph">${fav(p.first, true, 14)}</span><i>→</i><span class="glyph">${fav(p.second, true, 14)}</span></span>`
    + `<span class="grow">${esc(p.text)}</span><span class="menu-detail">${esc(p.count)}</span></button>`).join('');
  $('start-content').innerHTML = '<div class="start-title">New tree</div>'
    + `<button id="start-search">${icon('search')}<span>An address, a search, or a page in your forest</span><kbd>Ctrl K</kbd></button>`
    + (shortcuts ? `<div class="tiles">${shortcuts}</div>` : '<div class="empty">Your frequent pages will appear here as your forest grows.</div>')
    + '<div><div class="menu-heading">Usual paths</div>'
    + (paths || '<div class="empty" style="text-align:left;padding:0 8px">Follow links to build paths you can revisit.</div>') + '</div>';
  $('start-search').addEventListener('click', () => openPalette({ scope: null, text: '' }));
  $('start-content').querySelectorAll('[data-shortcut]').forEach((tile) => tile.addEventListener('click', () => {
    setSurface('reading');
    intent('shortcut', { index: Number(tile.dataset.shortcut) });
  }));
  $('start-content').querySelectorAll('[data-path]').forEach((row) => row.addEventListener('click', () => {
    setSurface('reading');
    intent('path', { index: Number(row.dataset.path) });
  }));
}

// ---------- forest view ----------

function forestLayout() {
  return S.frame?.forest[S.forestLabels ? 'labels' : 'icons'];
}

function renderForest() {
  if (S.surface !== 'forest' || !S.frame) {
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
    const cls = `forest-node${n.settled ? ' settled' : ''}${S.forestLabels ? '' : ' icon-only'}${path.has(n.id) ? ' path' : ''}${v.queued ? ' pending' : ''}`;
    return `<button class="${cls}" data-id="${n.id}" title="${esc(v.label)}" style="left:${n.x - n.w / 2}px;top:${n.y - n.s / 2}px;width:${n.w}px;height:${n.s}px">`
      + visitFav(v, 16) + (S.forestLabels ? `<span>${esc(v.label)}</span>` : '') + '</button>';
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
  $('forest-nodes').innerHTML = heads + divider + nodes;
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
    setSurface('reading');
    intent('restore', { id: node.dataset.id });
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
$('forest-close').addEventListener('click', () => setSurface(S.frame?.tabs.length ? 'reading' : 'start'));

// ---------- reader ----------

function showReader(html, list) {
  closePalette(false);
  closePopover(false);
  hideCard();
  const box = $('reader-box');
  box.className = `reader-box${list ? ' list' : ''}`;
  box.innerHTML = html + `<button class="icon-button reader-close" aria-label="Close">${icon('close')}</button>`;
  box.querySelector('.reader-close').addEventListener('click', closeReader);
  S.reader = true;
  $('reader').hidden = false;
  syncLayer();
}

function closeReader() {
  if (!S.reader) return;
  S.reader = null;
  $('reader').hidden = true;
  if (S.surface === 'reading') post({ t: 'focus', target: 'page' });
  syncLayer();
}
$('reader').querySelector('.scrim').addEventListener('mousedown', closeReader);

const when = (at) => new Date(at).toLocaleString(undefined, { day: '2-digit', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });

// ---------- Rust's side ----------

window.canopy = {
  present(frame) {
    const previous = current();
    S.frame = frame;
    S.strip = frame.strip;
    S.manualPan = null;
    applyTheme();
    if (frame.current !== previous && S.surface !== 'reading') setSurface('reading');
    if (!frame.tabs.length && S.surface === 'reading') setSurface('start');
    if (S.popover?.kind === 'rename' && !frame.tabs.some((tree) => tree.id === S.popover.data)) closePopover();
    renderTabs();
    animateStrip(previous === null && strip.shown.nodes.size === 0 ? 0 : MOVE);
    renderStart();
    renderForest();
    if (S.card) {
      if (!visit(S.card)) hideCard();
      else showCard(S.card);
    }
    if (S.popover) renderPopover();
    renderPaletteContext();
    syncLayer();
  },
  strip(layouts) {
    S.strip = layouts;
    S.manualPan = null;
    animateStrip(0);
    fitTabs();
  },
  icons(revision) {
    S.rev = revision;
    if (!S.frame) return;
    renderTabs();
    buildStrip(strip.target ?? targetLayout());
    renderStart();
    renderForest();
  },
  openPalette(options) { openPalette(options); },
  newTree() {
    intent('cancel');
    closePalette(false);
    setSurface('start');
  },
  toggleForest() {
    intent('cancel');
    closePalette(false);
    setSurface(S.surface === 'forest' ? (S.frame?.tabs.length ? 'reading' : 'start') : 'forest');
  },
  toast(kind, text) {
    if (kind === 'error') {
      S.errors.unshift(text);
      S.errors.length = Math.min(S.errors.length, 5);
    }
    const toast = $('toast');
    toast.textContent = text;
    toast.className = kind;
    toast.hidden = false;
    clearTimeout(timers.toast);
    timers.toast = setTimeout(() => {
      toast.classList.add('leaving');
      timers.toast = setTimeout(() => { toast.hidden = true; }, 200);
    }, kind === 'error' ? 5000 : 2600);
  },
  toggleFocus() {
    const active = document.activeElement;
    if (document.hasFocus() && active?.classList.contains('node')) {
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
    if (S.reader || S.palette || S.popover || (S.surface !== 'reading' && current())) {
      this.escape();
    } else if (current()) {
      intent('settle-current');
    }
  },
  /// Closes the topmost thing Escape owns.
  escape() {
    if (S.reader) closeReader();
    else if (S.palette) closePalette();
    else if (S.popover) closePopover();
    else if (S.card) hideCard();
    else if (S.surface === 'forest' || (S.surface !== 'reading' && current())) {
      setSurface(S.frame?.tabs.length ? 'reading' : 'start');
      post({ t: 'focus', target: 'page' });
    } else if (S.open) {
      setOpen(false);
    }
  },
  paletteResults(seq, { rows, first }) {
    if (!S.palette || seq !== S.palette.seq) return;
    S.palette.rows = rows;
    S.palette.selected = Math.max(rows.findIndex((row) => row.item === first), 0);
    renderPalette();
  },
  previewResult(seq, preview) {
    if (seq === previewSeq) renderCard(preview);
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
      else if (act === 'source') intent('restore', { id: c.parent });
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
  /// For tests: what the view shows, without reaching into the DOM.
  state() {
    return {
      surface: S.surface, open: S.open, animating: !!strip.anim, card: S.card, layer: S.layer, palette: !!S.palette,
      reader: !!S.reader, popover: S.popover?.kind ?? null, reported: S.reported,
      dark: document.documentElement.classList.contains('dark'),
      nodes: [...strip.elements.keys()], rows: S.palette?.rows ?? [],
    };
  },
};

// ---------- static wiring ----------

$('new-tree').innerHTML = icon('plus');
$('agent').innerHTML = icon('spark');
$('forest-button').innerHTML = icon('forest');
$('settings-button').innerHTML = icon('settings');
$('tab-overflow').innerHTML = icon('chevron');
$('search-pill').insertAdjacentHTML('afterbegin', icon('search'));

$('tab-row').addEventListener('click', (event) => {
  const close = event.target.closest('[data-settle]');
  if (close) { intent('settle-tree', { id: close.dataset.settle }); return; }
  const tab = event.target.closest('[data-id]');
  if (!tab) return;
  setSurface('reading');
  intent('select-tree', { id: tab.dataset.id });
});
$('tab-row').addEventListener('mousedown', (event) => {
  if (event.button === 1 && event.target.closest('[data-id]')) event.preventDefault();
});
$('tab-row').addEventListener('auxclick', (event) => {
  const tab = event.target.closest('[data-id]');
  if (event.button === 1 && tab) intent('settle-tree', { id: tab.dataset.id });
});
// A right click on a tab names its tree.
$('tab-row').addEventListener('contextmenu', (event) => {
  const tab = event.target.closest('[data-id]');
  if (!tab) return;
  event.preventDefault();
  openPopover('rename', tab, tab.dataset.id);
  post({ t: 'focus', target: 'chrome' });
  S.popover.field.focus();
  S.popover.field.select();
});
$('tab-overflow').addEventListener('click', (event) => {
  event.stopPropagation();
  if (S.popover?.kind === 'trees') closePopover();
  else openPopover('trees', $('tab-overflow'));
});
$('new-tree').addEventListener('click', () => window.canopy.newTree());
$('forest-button').addEventListener('click', () => window.canopy.toggleForest());
$('search-pill').addEventListener('click', () => openPalette({ scope: S.surface === 'start' ? null : current(), text: '' }));
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

const stripEl = $('strip');
stripEl.addEventListener('mouseenter', enterStrip);
stripEl.addEventListener('mouseleave', leaveStrip);
stripEl.addEventListener('wheel', (event) => {
  const layout = strip.target;
  if (!layout) return;
  event.preventDefault();
  const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
  const min = Math.min(window.innerWidth - layout.width, 0);
  S.manualPan = clamp((S.manualPan ?? strip.shown.pan) - delta, min, 0);
  strip.shown.pan = S.manualPan;
  drawStrip();
}, { passive: false });

const cardEl = $('card');
cardEl.addEventListener('mouseenter', () => { cancel('hideCard'); cancel('close'); });
cardEl.addEventListener('mouseleave', () => {
  if (S.card) schedule('hideCard', 120, hideCard);
  if (S.open) schedule('close', 380, closeStrip);
});

// A press outside an open popover closes it.
document.addEventListener('mousedown', (event) => {
  if (S.popover && !$('popover').contains(event.target) && event.target !== S.popover.anchor) closePopover();
});

// Over the transparent part of the window, the view only stays while something is open;
// a press there closes the open strip and card.
document.addEventListener('mousedown', (event) => {
  if (event.target === document.documentElement || event.target === document.body) {
    hideCard();
    if (S.open) setOpen(false);
  }
});

new ResizeObserver(() => fitTabs()).observe($('tabs'));
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', applyTheme);

post({ t: 'ready' });
