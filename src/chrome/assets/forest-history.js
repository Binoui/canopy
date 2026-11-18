function renderHistory(h) {
  const list = $('forest-list');
  const inList = list.contains(document.activeElement);
  patchChildren(list, h.items.map(historyItemHtml).join('') + (h.more ? `<button class="forest-more" data-key="more" data-forest="more" data-seen="${h.more.seen}">${esc(h.more.label)}</button>` : ''));
  patchChildren($('forest-preview'), h.preview ? forestPreviewHtml(h.preview) : '');
  $('forest-preview').hidden = !h.preview;
  if (inList) {
    const row = list.querySelector('[aria-selected="true"]');
    row?.focus({ preventScroll: true }); row?.scrollIntoView({ block: 'nearest' });
  }
}
function forestPreviewHtml(p) {
  return `<div class="pv-art" data-key="${p.key}-art">${miniHtml(p.mini)}</div>`
    + `<div class="pv-title" data-key="${p.key}-title"><b>${esc(p.title)}</b><span>${esc(p.meta)}</span></div>`
    + p.actions.map(a => `<button class="action pv-action" data-key="${p.key}-${a.kind}" data-forest="${a.kind}" data-id="${p.id}">${esc(a.label)}</button>`).join('')
    + p.outline.map(r => `<button class="pv-row${r.last ? ' last' : ''}${r.waiting ? ' waiting' : ''}" data-key="${p.key}-${r.id}" data-forest="page" data-id="${r.id}" aria-label="${esc(r.label)}" style="padding-left:${Math.min(r.depth,6)*16}px">${fav(r.url,!r.waiting,14)}<span>${esc(r.name)}</span></button>`).join('')
    + (p.cut ? `<div class="pv-cut" data-key="${p.key}-cut">${esc(p.cut)}</div>` : '');
}
function historyItemHtml(i) {
  if (i.kind === 'day') return `<div class="forest-day" data-key="${i.key}" role="presentation"><b>${esc(i.label)}</b><span>${esc(i.count)}</span></div>`;
  return `<button class="forest-row" role="option" data-key="${i.key}" data-id="${i.id}" data-forest="row" data-tree-menu aria-selected="${i.selected}" tabindex="${i.selected ? 0 : -1}" aria-label="${esc(i.name)}">`
    + `<span class="tm">${esc(i.time)}</span><span class="forest-icons">${i.icons.map(n => fav(n.url, true, 16)).join('')}${i.more ? `<span>${esc(i.more)}</span>` : ''}</span>`
    + `<span class="nm">${esc(i.title)}</span><span class="st">${esc(i.sites)}</span><span class="pg">${esc(i.pages)}</span></button>`;
}
