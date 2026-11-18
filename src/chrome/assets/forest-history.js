function historyRuns(runs) { return runs.map(r => r.m ? `<mark>${esc(r.t)}</mark>` : esc(r.t)).join(''); }
function historyEdge(edge, key) {
  return edge ? `<button class="forest-more" data-key="${key}" data-forest="more" data-edge="${edge.edge}" data-before="${edge.before}">${esc(edge.label)}</button>` : '';
}
function historySites(sites) {
  return sites.map(s => `<button data-key="site-${esc(s.site)}" data-forest="site" data-site="${esc(s.site)}" aria-pressed="${s.on}">${fav(s.icon,true,14)}<span>${esc(s.site)}</span><em>${s.count}</em></button>`).join('');
}
function renderHistory(h) {
  if (!$('history-main')) {
    $('forest-preview')?.remove();
    const rail = document.createElement('aside'); rail.id = 'history-rail';
    const main = document.createElement('div'); main.id = 'history-main';
    const top = document.createElement('div'); top.id = 'history-top';
    const chips = document.createElement('div'); chips.id = 'history-chips';
    const count = document.createElement('div'); count.id = 'history-count';
    $('forest-history').prepend(rail, main);
    main.append(top, $('forest-list'));
    top.append($('forest-find'), chips, count);
    $('forest-find').placeholder = h.field.placeholder;
    $('forest-find').setAttribute('aria-label', h.field.label);
    $('forest-list').setAttribute('role','list');
    $('forest-list').addEventListener('scroll', historyLoadEdge);
  }
  $('forest-find').hidden = false;
  $('history-count').textContent = h.count;
  patchChildren($('history-chips'), historySites(h.rail.sites));
  patchChildren($('history-rail'), `<h4 data-key="when">${esc(h.rail.whenLabel)}</h4>` + h.rail.when.map(b => `<button data-key="${b.key}" data-forest="jump" data-bucket="${b.key}" ${b.disabled ? 'disabled' : ''}><span>${esc(b.label)}</span><em>${b.count}</em></button>`).join('') + `<h4 data-key="sites">${esc(h.rail.sitesLabel)}</h4>` + historySites(h.rail.sites));
  const list = $('forest-list');
  const focused = list.contains(document.activeElement);
  const focusKey = focused ? document.activeElement.dataset.key : null;
  const reveal = h.reveal && JSON.stringify(h.reveal);
  const jumping = reveal && reveal !== list.dataset.reveal;
  const witness = [...list.querySelectorAll('.history-item')].find(el => el.getBoundingClientRect().bottom > list.getBoundingClientRect().top);
  const witnessKey = witness?.dataset.key;
  const offset = witness?.getBoundingClientRect().top;
  const oldMinis = new Map([...list.querySelectorAll('.forest-row')].map(el=>[el.dataset.id,el.querySelector('.history-mini')]));
  const entries = [];
  if(h.before) entries.push(`<div class="history-edge" data-key="before">${historyEdge(h.before,'before-button')}</div>`);
  for (const i of h.items) {
    const cls=i.kind === 'day' ? 'forest-day' : `history-item${i.selected ? ' selected' : ''}${i.unfolded ? ' unfolded' : ''}`;
    let content;
    if(i.kind === 'day') {
      content = `<b data-key="label">${esc(i.label)}</b><span data-key="count">${esc(i.count)}</span>` + (i.delete ? `<button data-key="delete" data-forest="delete-day" data-day="${i.delete.day}" aria-label="${esc(i.delete.name)}">${esc(i.delete.label)}</button>` : '');
    } else {
      const shape=JSON.stringify([i.mini,i.mini.nodes.map(n=>S.revisions.get(iconSite(n.url))??0)]);
      const oldMini=oldMinis.get(i.id);
      const mini=oldMini?.dataset.shape===shape ? oldMini.innerHTML : miniHtml(i.mini);
      const row = `<button class="forest-row" data-key="row" data-id="${i.id}" data-forest="row" data-tree-menu aria-selected="${i.selected}" aria-expanded="${i.unfolded}" tabindex="${i.selected ? 0 : -1}" aria-label="${esc(i.name)}"><span data-key="time" class="tm">${esc(i.time)}</span><span data-key="mini" class="history-mini" data-shape="${esc(shape)}">${mini}</span><span data-key="names" class="history-names"><b>${historyRuns(i.titleRuns)}</b><span>${i.trail.map(historyRuns).join(' · ')}${i.more ? ` · ${esc(i.more)}` : ''}</span></span><span data-key="meta" class="history-meta">${esc(i.meta)}</span></button>`;
      const actions = i.buttons.map(a=>`<button class="history-action ${a.act}" data-key="act-${a.act}" data-forest="act" data-act="${a.act}" data-id="${i.id}" aria-label="${esc(a.label)}" title="${esc(a.label)}">${a.act==='open'?esc(a.label):a.act==='save'?icon('bookmark'):icon('close')}</button>`).join('');
      const outline = i.outline ? i.outline.rows.map(r=>`<button class="history-page${r.last?' last':''}${r.waiting?' waiting':''}" data-key="page-${r.id}" data-forest="page" data-id="${r.id}" aria-label="${esc(r.label)}" style="padding-left:${Math.min(r.depth,12)*16+10}px">${fav(r.url,!r.waiting,14)}<span>${esc(r.name)}</span></button>`).join('') + (i.outline.cut?`<div class="history-cut" data-key="cut">${esc(i.outline.cut)}</div>`:'') : '';
      content=row+actions+outline;
    }
    entries.push(`<div class="${cls}" data-key="${i.key}">${content}</div>`);
  }
  if(h.more) entries.push(`<div class="history-edge" data-key="more">${historyEdge(h.more,'more-button')}</div>`);
  patchChildren(list,entries.join(''));
  if(jumping) {
    const row=list.querySelector(`.forest-row[data-id="${h.reveal.id}"]`);
    (row?.closest('.history-item')?.previousElementSibling ?? row)?.scrollIntoView({block:'start'});
  } else if(witnessKey) {
    const kept = [...list.children].find(el=>el.dataset.key===witnessKey);
    if(kept) list.scrollTop += kept.getBoundingClientRect().top-offset;
  }
  list.dataset.reveal = reveal || '';
  if(focused) {
    const selected = list.querySelector('.forest-row[aria-selected="true"]');
    const retained = document.activeElement !== document.body && document.activeElement.dataset.key===focusKey && document.activeElement.isConnected;
    if(!retained || focusKey==='row') {selected?.focus({preventScroll:true});selected?.scrollIntoView({block:'nearest'});}
  }
}
function historyLoadEdge() {
  const h=S.forest?.history, list=$('forest-list');
  if(!h || S.forest.view!=='history')return;
  if(list.scrollTop<32 && h.before) post({t:'forest',do:'more',edge:h.before.edge,before:true});
  else if(list.scrollTop+list.clientHeight>=list.scrollHeight-80 && h.more) post({t:'forest',do:'more',edge:h.more.edge,before:false});
}
function forestHistoryClick(e) {
  const el=e.target.closest('[data-forest]');if(!el)return;
  const id=el.dataset.id;
  switch(el.dataset.forest) {
    case 'row': post({t:'forest',do:'select',tree:id});break;
    case 'act': treeAct(id,el.dataset.act);break;
    case 'page': post({t:'forest',do:'page',id});break;
    case 'more': post({t:'forest',do:'more',edge:el.dataset.edge,before:el.dataset.before==='true'});break;
    case 'site': post({t:'forest',do:'site',site:el.dataset.site});break;
    case 'jump': post({t:'forest',do:'jump',bucket:el.dataset.bucket});break;
    case 'delete-day': post({t:'forest',do:'delete-day',day:el.dataset.day});break;
  }
}
$('forest-history').addEventListener('dblclick', e=>{const row=e.target.closest('.forest-row');if(row)openForestTree(row.dataset.id);});
function forestHistoryKey(e) {
  if(e.target!==$('forest-find')) {
    if(!e.target.closest('.forest-row') && ['Enter',' '].includes(e.key))return;
    switch(e.key) {
      case 'ArrowUp': case 'ArrowDown': post({t:'forest',do:'step',by:e.key==='ArrowUp'?-1:1});break;
      case 'ArrowRight': case ' ': post({t:'forest',do:'fold',open:true});break;
      case 'ArrowLeft': post({t:'forest',do:'fold',open:false});break;
      case 'Enter': if(e.target.closest('.forest-row'))post({t:'forest',do:'act',act:'open'});else return;break;
      case 's': case 'S': post({t:'forest',do:'act',act:'save'});break;
      case 'Delete': post({t:'forest',do:'act',act:'delete'});break;
      default: if(!forestFindKey(e,'find'))return;
    }
  } else if(!forestFindKey(e,'find'))return;
  e.preventDefault();
}
function forestHistoryFocus() { return $('forest-list').querySelector('.forest-row[aria-selected="true"]') ?? $('forest-find'); }
function forestHistoryMenu(id) { return S.forest?.history?.items?.find(i=>i.id===id)?.menu; }
