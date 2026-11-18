function treesCardHtml(c) {
  const line = c.kind === 'line';
  const main = `<button class="card-main" data-key="main" data-main tabindex="-1" title="${esc(c.tip ?? c.title)}" aria-label="${esc(c.name)}">`
    + (line ? `<span class="card-icon">${fav(c.icon, true, 16)}</span>` : `<span class="card-art">${miniHtml(c.mini)}</span>`)
    + `<span class="card-title">${esc(c.title)}</span>`
    + (!line && c.page ? `<span class="card-page">${esc(c.page)}</span>` : '')
    + (c.time ? `<span class="card-time">${esc(c.time)}</span>` : '')
    + (!line && c.meta ? `<span class="card-meta">${esc(c.meta)}</span>` : '')
    + (!line && c.waiting ? `<span class="card-waiting count" title="${esc(c.waitingTip)}" aria-label="${esc(c.waitingTip)}">${esc(c.waiting)}</span>` : '') + '</button>';
  const glyph = {pin:icon('pinned'), unpin:icon('pinned'), save:icon('bookmark'), settle:'✓'};
  const buttons = c.buttons.map(b => `<button data-key="${b.act}" data-act="${b.act}" class="card-action" title="${esc(b.label)}" aria-label="${esc(b.label)}">${glyph[b.act] ?? esc(b.label)}</button>`).join('');
  return `<div class="tree-card ${c.kind}" data-key="${c.key}" data-id="${c.id}" data-forest="card" data-tree-menu draggable="true" tabindex="0" role="group" aria-label="${esc(c.name)}" aria-current="${!!c.current}">${main}<span class="card-acts" data-key="acts">${buttons}</span></div>`;
}
function treesLaneHtml(l) {
  return `<section class="forest-lane" data-key="${l.key}" data-lane="${l.lane}" aria-label="${esc(l.title)}">`
    + `<header data-key="head"><b>${esc(l.title)}</b><span>${esc(l.count)}</span></header>`
    + `<div class="lane-cards" data-key="cards">${l.cards.length ? l.cards.map(treesCardHtml).join('') : `<div class="hint-line" data-key="empty">${esc(l.empty)}</div>`}</div>`
    + (l.foot ? `<button class="lane-history" data-key="foot" data-show-view="${l.foot.view}">${esc(l.foot.label)}</button>` : '') + '</section>';
}
function renderForestTrees(value) {
  $('forest-find').hidden = true;
  const focused = document.activeElement.closest('#forest-trees [data-id]')?.dataset.id;
  patchChildren($('forest-trees'), value.lanes.map(treesLaneHtml).join(''));
  if (focused && document.activeElement === document.body) $('forest-trees').querySelector(`[data-id="${focused}"]`)?.focus();
}
function forestTreesFocus() { return $('forest-trees').querySelector('.tree-card[aria-current="true"]') ?? $('forest-trees').querySelector('.tree-card') ?? $('forest-trees').querySelector('[data-show-view]'); }
function forestTreesMenu(id) { return S.forest?.trees?.lanes.flatMap(l => l.cards).find(c => c.id === id)?.menu; }
function forestTreesClick(e) {
  const view = e.target.closest('[data-show-view]');
  if (view) { post({t:'forest',do:'show',view:view.dataset.showView}); return; }
  const card = e.target.closest('#forest-trees [data-id]');
  if (!card) return;
  const act = e.target.closest('[data-act]');
  if (act) treeAct(card.dataset.id, act.dataset.act);
  else if (e.target.closest('[data-main]')) openForestTree(card.dataset.id);
}
function forestTreesKey(e) {
  const card = e.target.closest('#forest-trees [data-id]');
  if (!card || !(e.target === card || e.target.matches('[data-main]'))) return;
  if (e.target === card && e.key === 'Enter') { openForestTree(card.dataset.id); e.preventDefault(); return; }
  const lane = card.closest('[data-lane]');
  let next;
  if (['ArrowUp','ArrowDown'].includes(e.key)) {
    const cards = [...lane.querySelectorAll('[data-id]')];
    next = cards[clamp(cards.indexOf(card) + (e.key === 'ArrowDown' ? 1 : -1), 0, cards.length - 1)];
  } else if (['ArrowLeft','ArrowRight'].includes(e.key)) {
    const lanes = [...$('forest-trees').children];
    const target = lanes[lanes.indexOf(lane) + (e.key === 'ArrowRight' ? 1 : -1)];
    const y = card.getBoundingClientRect().top;
    next = [...target?.querySelectorAll('[data-id]') ?? []].sort((a,b) => Math.abs(a.getBoundingClientRect().top-y)-Math.abs(b.getBoundingClientRect().top-y))[0] ?? target?.querySelector('[data-show-view]');
  } else return;
  next?.focus();
  e.preventDefault();
}

function clearForestDrop() {
  for (const el of $('forest-trees').querySelectorAll('.drop-here,.drop-before,.drop-end')) el.classList.remove('drop-here','drop-before','drop-end');
}
function forestDropAt(event) {
  if (!tabDrag || ![...event.dataTransfer.types].includes(treeDragType)) return null;
  const lane = event.target.closest('#forest-trees [data-lane]');
  const spec = S.forest?.trees?.lanes.find(l => l.lane === lane?.dataset.lane);
  const from = S.forest?.trees?.lanes.find(l => l.cards.some(c => c.id === tabDrag))?.lane
    ?? (S.frame.tabs.some(t => t.id === tabDrag) ? 'tabs' : null);
  if (!spec?.accepts.includes(from)) return null;
  const cards = [...lane.querySelectorAll('[data-forest="card"]')].filter(c => c.dataset.id !== tabDrag);
  const before = spec.ordered ? cards.find(c => { const r=c.getBoundingClientRect(); return event.clientY < r.top+r.height/2; }) : null;
  return {lane, spec, before};
}
function forestDragOver(event) {
  const drop = forestDropAt(event);
  clearForestDrop();
  $('forest-trees').querySelector(`[data-id="${tabDrag}"]`)?.classList.add('dragging');
  if (!drop) return;
  event.preventDefault();
  event.dataTransfer.dropEffect = 'move';
  drop.lane.classList.add('drop-here');
  if (drop.before) drop.before.classList.add('drop-before');
  else drop.lane.classList.add('drop-end');
  const box = drop.lane.querySelector('.lane-cards'), r = box.getBoundingClientRect();
  if (event.clientY < r.top + 28) box.scrollTop -= 12;
  else if (event.clientY > r.bottom - 28) box.scrollTop += 12;
}
$('forest-trees').addEventListener('dragstart', event => {
  const card = event.target.closest('[data-forest="card"]');
  if (!card || event.target.closest('[data-act]')) { event.preventDefault(); return; }
  tabDrag = card.dataset.id;
  event.dataTransfer.setData(treeDragType, tabDrag);
  event.dataTransfer.effectAllowed = 'move';
  requestAnimationFrame(() => { if (tabDrag === card.dataset.id) card.classList.add('dragging'); });
});
$('forest-trees').addEventListener('dragenter', forestDragOver);
$('forest-trees').addEventListener('dragover', forestDragOver);
$('forest-trees').addEventListener('dragleave', event => {
  const lane = event.target.closest('[data-lane]');
  if (!lane) return;
  const r = lane.getBoundingClientRect();
  if (event.clientX < r.left || event.clientX >= r.right || event.clientY < r.top || event.clientY >= r.bottom) clearForestDrop();
});
$('forest-trees').addEventListener('drop', event => {
  const drop = forestDropAt(event), tree = event.dataTransfer.getData(treeDragType);
  if (!drop || tree !== tabDrag) return;
  event.preventDefault();
  const lane = drop.spec.lane, before = drop.before?.dataset.id ?? null;
  finishTabDrag();
  post({t:'forest',do:'drop',tree,lane,before});
});
$('forest-trees').addEventListener('dragend', () => finishTabDrag());
