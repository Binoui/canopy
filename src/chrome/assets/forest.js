// ---------- forest view ----------

function toggleForest(view = 'canopy', fromChrome = true) {
  if (S.overlay !== 'forest') {
    S.forestWants = view;
    toggleOverlay('forest');
    if (!fromChrome && S.overlay === 'forest') rememberFocus('overlay', document.body);
  } else if (!S.forest) closeOverlay();
  else post({ t: 'forest', do: 'toggle', view });
}
function focusForest() {
  post({ t: 'focus', target: 'chrome' });
  const target = S.forest?.view === 'canopy' ? canopyFocus() : forestHistoryFocus();
  (target ?? $('forest-close')).focus({preventScroll:true});
  if (S.forest?.view !== 'canopy') target?.scrollIntoView({ block: 'nearest' });
}
function renderForest() {
  const f = S.forest;
  if (!f || S.overlay !== 'forest') return;
  const within = $('forest').contains(document.activeElement);
  $('forest-undo').hidden = !f.undo;
  patchChildren($('forest-undo'), f.undo ? `<span data-key="text">${esc(f.undo.text)}</span><button data-key="undo" data-delete-id="${f.undo.id}">${esc(f.undo.label)}</button>` : '');
  $('forest-count').textContent = f.canopy?.hint ?? f.count;
  if (document.activeElement !== $('forest-find') || !f.query) $('forest-find').value = f.query;
  for (const b of $('forest-views').children) b.setAttribute('aria-pressed', String(b.dataset.view === f.view));
  $('forest-canopy').hidden = f.view !== 'canopy';
  for (const id of ['canopy-layout','canopy-zoom','canopy-legend']) $(id).hidden = f.view !== 'canopy';
  $('canopy-back').hidden = f.view !== 'canopy' || !f.canopy?.back;
  $('forest-find').hidden = f.view !== 'history';
  $('forest-history').hidden = f.view !== 'history';
  if (f.view === 'canopy') renderCanopy(f.canopy);
  else renderHistory(f.history);
  const empty = f.view === 'canopy' ? null : f.history.empty;
  $('forest-empty').hidden = !empty;
  $('forest-empty').textContent = empty ?? '';
  if (within && (!$('forest').contains(document.activeElement) || !document.activeElement.getClientRects().length)) focusForest();
}
function openForestTree(id) { treeAct(id, 'open'); }
$('forest').addEventListener('click', e => { if (S.forest?.view === 'canopy') canopyClick(e); else forestHistoryClick(e); });
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
$('forest-find').addEventListener('input', () => post({t:'forest',do:'find',query:$('forest-find').value}));
$('forest-close').addEventListener('click', closeOverlay);
$('forest').querySelector('.scrim').addEventListener('mousedown', closeOverlay);
$('forest').addEventListener('keydown', (e) => {
  if (S.popover || e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return;
  if (S.forest?.view === 'canopy') canopyKey(e); else forestHistoryKey(e);

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

function forestMenu(id) { return S.forest?.view === 'canopy' ? S.forest.canopy.items.find(i => i.id === id)?.menu : forestHistoryMenu(id); }
