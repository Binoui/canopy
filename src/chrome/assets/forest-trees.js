function treesItemHtml(i) {
  if (i.kind === 'band') return `<div class="band-head" data-key="${i.key}"><span>${esc(i.title)}</span><span class="hint">${esc(i.detail)}</span></div>`;
  if (i.kind === 'hint') return `<div class="hint-line" data-key="${i.key}">${esc(i.text)}</div>`;
  return treeCard(i, `data-key="${i.key}" data-id="${i.id}" data-forest="card" data-tree-menu aria-current="${i.current}"`);
}
function moveForestCard(card, key) {
  const cards = [...$('forest-trees').querySelectorAll('.tree-card')];
  let next;
  if (key === 'ArrowLeft' || key === 'ArrowRight') next = cards[clamp(cards.indexOf(card) + (key === 'ArrowRight' ? 1 : -1), 0, cards.length - 1)];
  else {
    const a = card.getBoundingClientRect(), down = key === 'ArrowDown';
    next = cards.map(c => [c, c.getBoundingClientRect()])
      .filter(([,r]) => down ? r.top > a.bottom - 1 : r.bottom < a.top + 1)
      .sort(([,r],[,s]) => (down ? r.top - s.top : s.bottom - r.bottom) || Math.abs(r.left-a.left)-Math.abs(s.left-a.left))[0]?.[0];
  }
  next?.focus();
}


function forestTreesClick(e) { const card=e.target.closest('[data-forest="card"]'); if(card) openForestTree(card.dataset.id); }
function forestTreesKey(e) {
  const card=e.target.closest('.tree-card');
  if (card && ['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(e.key)) moveForestCard(card,e.key);
  else if (!forestFindKey(e,'trees-find')) return;
  e.preventDefault();
}
