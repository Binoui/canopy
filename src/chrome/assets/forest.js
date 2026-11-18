// ---------- forest view ----------

function toggleForest(view = 'trees', fromChrome = true) {
  if (S.overlay !== 'forest') {
    S.forestWants = view;
    toggleOverlay('forest');
    if (!fromChrome && S.overlay === 'forest') rememberFocus('overlay', document.body);
  } else if (!S.forest) closeOverlay();
  else post({ t: 'forest', do: 'toggle', view });
}
function focusForest() {
  post({ t: 'focus', target: 'chrome' });
  const target = S.forest?.view === 'trees'
    ? $('forest-trees').querySelector('.tree-card[aria-current="true"]') ?? $('forest-trees').querySelector('.tree-card')
    : $('forest-list').querySelector('[aria-selected="true"]');
  (target ?? $('forest-find')).focus({ preventScroll: true });
  target?.scrollIntoView({ block: 'nearest' });
}
function renderForest() {
  const f = S.forest;
  if (!f || S.overlay !== 'forest') return;
  const within = $('forest').contains(document.activeElement);
  $('forest-count').textContent = f.count;
  if (document.activeElement !== $('forest-find') || !f.query) $('forest-find').value = f.query;
  for (const b of $('forest-views').children) b.setAttribute('aria-pressed', String(b.dataset.view === f.view));
  $('forest-trees').hidden = f.view !== 'trees';
  $('forest-history').hidden = f.view !== 'history';
  if (f.view === 'trees') patchChildren($('forest-trees'), f.trees.items.map(treesItemHtml).join(''));
  else renderHistory(f.history);
  const empty = f.view === 'trees' ? f.trees.empty : f.history.empty;
  $('forest-empty').hidden = !empty;
  $('forest-empty').textContent = empty ?? '';
  if (within && (!$('forest').contains(document.activeElement) || !document.activeElement.getClientRects().length)) focusForest();
}
function openForestTree(id) { uncover(); intent('select-tree', { id }); }
$('forest').addEventListener('click', (e) => {
  const el = e.target.closest('[data-forest]');
  if (!el) return;
  const id = el.dataset.id;
  if (el.dataset.forest === 'card') openForestTree(id);
  else if (el.dataset.forest === 'row') post({ t: 'forest', do: 'select', tree: id });
  else if (el.dataset.forest === 'reopen') openForestTree(id);
  else if (el.dataset.forest === 'more') post({ t: 'forest', do: 'more', seen: Number(el.dataset.seen) });
  else if (el.dataset.forest === 'save') intent('pin-tree', { id, pinned: true });
  else if (el.dataset.forest === 'page') { uncover(); intent('restore', { id }); }
});
$('forest').addEventListener('contextmenu', (e) => {
  const el = e.target.closest('[data-tree-menu]');
  if (!el) return;
  e.preventDefault();
  openPopover('tree', el, { source: 'forest', id: el.dataset.id });
  post({ t: 'focus', target: 'chrome' });
  $('popover').querySelector('button')?.focus();
});
$('forest').addEventListener('dblclick', (e) => {
  const row = e.target.closest('[data-forest="row"]');
  if (row) openForestTree(row.dataset.id);
});
$('forest-views').addEventListener('click', (e) => {
  const b = e.target.closest('[data-view]');
  if (b) post({ t: 'forest', do: 'show', view: b.dataset.view });
});
$('forest-find').addEventListener('input', () => post({ t: 'forest', do: 'find', query: $('forest-find').value }));
$('forest-close').addEventListener('click', closeOverlay);
$('forest').querySelector('.scrim').addEventListener('mousedown', closeOverlay);
$('forest').addEventListener('keydown', (e) => {
  if (S.popover || e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return;
  const field = e.target === $('forest-find');
  const row = e.target.closest('.forest-row'), card = e.target.closest('.tree-card');
  if (!field && [...e.key].length === 1 && e.key !== ' ') {
    $('forest-find').focus(); $('forest-find').value += e.key;
    post({ t: 'forest', do: 'find', query: $('forest-find').value });
  } else if (field && e.key === 'ArrowDown') focusForest();
  else if (row && ['ArrowUp', 'ArrowDown'].includes(e.key)) post({ t: 'forest', do: 'step', by: e.key === 'ArrowUp' ? -1 : 1 });
  else if (row && e.key === 'Enter') openForestTree(row.dataset.id);
  else if (card && ['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(e.key)) moveForestCard(card, e.key);
  else return;
  e.preventDefault();
});
