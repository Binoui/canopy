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
  const target = S.forest?.view === 'trees' ? forestTreesFocus() : forestHistoryFocus();
  (target ?? $('forest-close')).focus({preventScroll:true});
  target?.scrollIntoView({ block: 'nearest' });
}
function renderForest() {
  const f = S.forest;
  if (!f || S.overlay !== 'forest') return;
  const within = $('forest').contains(document.activeElement);
  $('forest-undo').hidden = !f.undo;
  patchChildren($('forest-undo'), f.undo ? `<span data-key="text">${esc(f.undo.text)}</span><button data-key="undo" data-delete-id="${f.undo.id}">${esc(f.undo.label)}</button>` : '');
  $('forest-count').textContent = f.count;
  if (document.activeElement !== $('forest-find') || !f.query) $('forest-find').value = f.query;
  for (const b of $('forest-views').children) b.setAttribute('aria-pressed', String(b.dataset.view === f.view));
  $('forest-trees').hidden = f.view !== 'trees';
  $('forest-history').hidden = f.view !== 'history';
  if (f.view === 'trees') renderForestTrees(f.trees);
  else renderHistory(f.history);
  const empty = f.view === 'trees' ? f.trees.empty : f.history.empty;
  $('forest-empty').hidden = !empty;
  $('forest-empty').textContent = empty ?? '';
  if (within && (!$('forest').contains(document.activeElement) || !document.activeElement.getClientRects().length)) focusForest();
}
function openForestTree(id) { treeAct(id, 'open'); }
$('forest').addEventListener('click', e => { if (S.forest?.view === 'trees') forestTreesClick(e); else forestHistoryClick(e); });
$('forest').addEventListener('contextmenu', (e) => {
  const el = e.target.closest('[data-tree-menu]');
  if (!el) return;
  e.preventDefault();
  openPopover('tree', el, { source: 'forest', id: el.dataset.id });
  post({ t: 'focus', target: 'chrome' });
  $('popover').querySelector('button')?.focus();
});
$('forest-views').addEventListener('click', (e) => {
  const b = e.target.closest('[data-view]');
  if (b) post({ t: 'forest', do: 'show', view: b.dataset.view });
});
$('forest-find').addEventListener('input', () => post({t:'forest',do:S.forest?.view === 'trees' ? 'trees-find' : 'find',query:$('forest-find').value}));
$('forest-close').addEventListener('click', closeOverlay);
$('forest').querySelector('.scrim').addEventListener('mousedown', closeOverlay);
$('forest').addEventListener('keydown', (e) => {
  if (e.altKey && ['ArrowUp', 'ArrowDown'].includes(e.key)) {
    const card = e.target.closest('[data-id]');
    if (card) { intent('step-tree', {id: card.dataset.id, next: e.key === 'ArrowDown'}); e.preventDefault(); }
    return;
  }
  if (e.altKey && e.key === 'Enter') {
    const card = e.target.closest('[data-id]');
    if (card) { post({t:'forest',do:'act',tree:card.dataset.id,act:'open-keep'}); e.preventDefault(); }
    return;
  }
  if (S.popover || e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return;
  if (S.forest?.view === 'trees') forestTreesKey(e); else forestHistoryKey(e);

});

$('forest-undo').addEventListener('click', e => { const b=e.target.closest('[data-delete-id]'); if(b) post({t:'forest',do:'undo',id:b.dataset.deleteId}); });

function forestFindKey(e, action) {
  const field=e.target===$('forest-find');
  if(!field && [...e.key].length===1 && e.key!==' ') {
    $('forest-find').focus();$('forest-find').value+=e.key;
    post({t:'forest',do:action,query:$('forest-find').value});return true;
  }
  if(field && e.key==='ArrowDown') {focusForest();return true;}
  return false;
}

function forestMenu(id) { return S.forest?.view === 'trees' ? forestTreesMenu(id) : forestHistoryMenu(id); }
