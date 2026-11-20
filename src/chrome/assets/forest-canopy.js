let mapView = null;
let mapRequest = 0;
function resetCanopy() {
  if (mapView) clearTimeout(mapView.tipTimer);
  mapView = null;
  $('canopy-tip').hidden = true;
}
function acceptCanopy(value) {
  return mapView ? value.place.request === mapView.request : value.place.request === 0;
}
const canopyValue = () => mapView?.value;
const band = level => canopyValue().levels.find(l => l.level === level);
const levelOf = k => canopyValue().levels.find(l => k >= l.min && k < l.max)?.level
  ?? (k < canopyValue().levels[0].min ? 'far' : 'near');
const scaleNow = () => mapView.k / band(mapView.drawn).rest;
function applyTransform() {
  const world = $('forest-canopy').querySelector('.canopy-world');
  world.style.setProperty('--s', scaleNow());
  world.style.setProperty('--px', mapView.pan[0]);
  world.style.setProperty('--py', mapView.pan[1]);
}
function screenOf(node, s = scaleNow()) {
  return [mapView.pan[0] + node.x * s, mapView.pan[1] + node.y * s];
}
function witnessAt(at, focused = false) {
  const view = $('forest-canopy').getBoundingClientRect();
  const el = focused ? canopyFocus() : document.elementFromPoint(at[0] + view.left, at[1] + view.top)?.closest('.canopy-item');
  const node = el && mapView.nodes.get(el.dataset.key);
  const item = node && canopyValue().items[+el.dataset.index];
  if (!item?.go) return { world: at.map((p,i) => (p-mapView.pan[i])/scaleNow()), at };
  const r = el.getBoundingClientRect();
  const retained=mapView.witness?.page;
  const page=retained && item.pages?.includes(retained) ? retained : item.anchorPage ?? item.go;
  return { page, fraction:[(at[0]+view.left-r.left)/r.width,(at[1]+view.top-r.top)/r.height], at };
}
// Over empty space the world point holds while the layout stays, but a new zoom level
// lays the map out again, so the nearest page keeps its screen offset from the pointer.
function nearestPage(at) {
  let best=null;
  for (const node of mapView.nodes.values()) {
    const item=canopyValue().items[+node.el.dataset.index];
    if (!item?.go || !item.pages?.length) continue;
    const p=screenOf(node), d=Math.hypot(p[0]-at[0],p[1]-at[1]);
    if (!best || d<best.d) best={d,page:item.anchorPage ?? item.go,off:[at[0]-p[0],at[1]-p[1]]};
  }
  return best && {page:best.page,off:best.off};
}
function nodeOfPage(page) {
  const item = canopyValue().items.find(i => i.pages?.includes(page));
  return item && mapView.nodes.get(item.k);
}
function witnessScreen(witness) {
  if (!witness?.page) return witness;
  const item = canopyValue().items.find(i => i.pages?.includes(witness.page));
  const node = item && mapView.nodes.get(item.k);
  if (!node) return null;
  const center = screenOf(node), r = node.el.getBoundingClientRect();
  return { ...witness, at:[center[0]+(witness.fraction[0]-.5)*r.width,center[1]+(witness.fraction[1]-.5)*r.height] };
}
function reanchor(witness, relaid = false) {
  if (!witness) return;
  const near = relaid && witness.near && nodeOfPage(witness.near.page);
  if (near) {
    mapView.pan = witness.at.map((p,i) => p-witness.near.off[i]-(i ? near.y : near.x)*scaleNow());
    return;
  }
  if (witness.world) {
    mapView.pan = witness.at.map((p,i) => p-witness.world[i]*scaleNow());
    return;
  }
  const item = canopyValue().items.find(i => i.pages?.includes(witness.page));
  const node = item && mapView.nodes.get(item.k);
  if (!node) return;
  const r = node.el.getBoundingClientRect();
  mapView.pan = [witness.at[0]-node.x*scaleNow()-(witness.fraction[0]-.5)*r.width,
    witness.at[1]-node.y*scaleNow()-(witness.fraction[1]-.5)*r.height];
}
function renderCanopy(value) {
  if (!acceptCanopy(value)) return;
  const first = !mapView;
  if (first) mapView = { k:value.place.k,pan:[...value.place.pan],drawn:value.level,request:0,
    asked:null,nodes:new Map([...$('forest-canopy').querySelector('.canopy-items').children].map(el => [el.dataset.key,{el,sig:null,x:NaN,y:NaN}])),edges:new Map([...$('forest-canopy').querySelector('.canopy-edges g').children].map(el => [el.dataset.key,el])),focus:null,drag:null,moved:false,tipTimer:0,room:null,witness:null,value };
  const changedLayout = !first && canopyValue().layout !== value.layout;
  const active = document.activeElement?.closest('.canopy-item');
  const oldFocus=canopyValue().items.find(i=>i.k===mapView.focus);
  const focusedPage=mapView.focusPage ?? oldFocus?.anchorPage ?? oldFocus?.go;
  const witness = first ? null : witnessScreen(mapView.witness) ?? (active ? witnessAt(focusedPoint(), true) : null);
  if (focusedPage) {
    const next=value.items.find(i=>i.pages?.includes(focusedPage));
    mapView.focus=next?.k ?? homeKey(value);
    mapView.focusPage=next ? focusedPage : null;
  }
  mapView.value = value;
  mapView.drawn = value.level;
  mapView.asked = null;
  const view = $('forest-canopy');
  view.hidden = false;
  view.dataset.level = value.level;
  view.dataset.layout = value.layout;
  for (const [name,px] of Object.entries(value.css)) view.style.setProperty(name, `${px}px`);
  patchItems(value);
  patchEdges(value);
  const tipNode=mapView.nodes.get(mapView.tipKey);
  if (!tipNode) hideTip();
  else if (!$('canopy-tip').hidden) $('canopy-tip').textContent=value.items[+tipNode.el.dataset.index].tip;
  if (changedLayout) { mapView.pan=[...value.place.pan]; mapView.witness=null; }
  else reanchor(witness, true);
  applyTransform();
  if (active && (!active.isConnected || active.dataset.key !== mapView.focus)) canopyFocus()?.focus({preventScroll:true});
  if (changedLayout) { focusItem(homeKey(value)); hideTip(); }
  renderCanopyHeader(value);
}
function patchItems(value) {
  const host = $('forest-canopy').querySelector('.canopy-items'), seen = new Set();
  if (!value.items.some(i => i.k === mapView.focus)) mapView.focus = homeKey(value);
  value.items.forEach((item,index) => {
    seen.add(item.k);
    const {x,y,nav,up,...shown} = item, sig = JSON.stringify(shown);
    let node = mapView.nodes.get(item.k);
    if (!node) {
      const el = document.createElement('button'); el.className = 'canopy-item'; el.dataset.key = item.k;
      host.append(el); node = {el,sig:null,x:NaN,y:NaN}; mapView.nodes.set(item.k,node);
    }
    const el = node.el;
    if (node.sig !== sig) {
      el.dataset.kind = item.kind;
      el.dataset.st = item.st ?? '';
      el.dataset.current = String(!!item.current); el.dataset.previous = String(!!item.previous);
      el.dataset.playing = String(!!item.playing);
      if (item.page) el.dataset.page = item.page;
      if (item.kind === 'tree') {el.dataset.id = item.id; el.dataset.treeMenu = '';}
      el.setAttribute('aria-label',item.tip ?? `${item.t}, ${item.sub}`);
      el.style.width = `${item.w}px`; el.style.height = `${item.h}px`;
      el.innerHTML = itemHtml(item);
      node.sig = sig;
    }
    if (node.x !== x || node.y !== y) {
      el.style.setProperty('--x',x); el.style.setProperty('--y',y); node.x=x; node.y=y;
    }
    if (el.dataset.index !== String(index)) el.dataset.index = index;
    const tabIndex = item.k === mapView.focus ? 0 : -1;
    if (el.tabIndex !== tabIndex) el.tabIndex = tabIndex;
  });
  for (const [key,node] of mapView.nodes) if (!seen.has(key)) {node.el.remove();mapView.nodes.delete(key);}
}
function itemHtml(item) {
  if (item.kind === 'tree') return `<b>${esc(item.t)}</b> <i>${esc(item.sub)}</i>`;
  const tags = (item.tags ?? []).map(t => `<i class="tag${t === 'Back' ? ' back' : ''}">${esc(t)}</i>`).join('');
  const count = item.n ? `<i class="cnt">${esc(item.n)}</i>` : '';
  const lines = item.t ? `<span class="lb"><b>${esc(item.t)}</b>${item.sub ? `<em>${esc(item.sub)}</em>` : ''}</span>` : '';
  return `<span class="tags">${tags}</span><span class="mk">${fav(item.url,item.st !== 'waiting',item.sz)}${playingMark(item.playing)}${count}</span>${lines}`;
}
function patchEdges(value) {
  const host = $('forest-canopy').querySelector('.canopy-edges g'), seen = new Set();
  value.items.forEach(item => {
    if (item.up === null || item.up === undefined) return;
    const a=value.items[item.up], mx=(a.x+item.x)/2;
    const d=value.edge === 'line' ? `M${a.x} ${a.y}L${item.x} ${item.y}` : `M${a.x} ${a.y}C${mx} ${a.y} ${mx} ${item.y} ${item.x} ${item.y}`;
    let path=mapView.edges.get(item.k);
    if (!path) {path=document.createElementNS('http://www.w3.org/2000/svg','path');path.dataset.key=item.k;host.append(path);mapView.edges.set(item.k,path);}
    if (path.getAttribute('d') !== d) path.setAttribute('d',d);
    if (path.dataset.e !== item.e) path.dataset.e=item.e;
    seen.add(item.k);
  });
  for (const [key,path] of mapView.edges) if (!seen.has(key)) {path.remove();mapView.edges.delete(key);}
}
function homeKey(value) {return value.items.find(i => i.current && i.go)?.k ?? value.items[0]?.k;}
function renderCanopyHeader(value) {
  patchChildren($('canopy-layout'),value.layouts.map(l => `<button data-key="${l.layout}" data-layout="${l.layout}" aria-pressed="${l.layout === value.layout}">${esc(l.label)}</button>`).join(''));
  patchChildren($('canopy-zoom'),value.levels.map(l => `<button data-key="${l.level}" data-level="${l.level}" aria-pressed="${l.level === value.level}">${esc(l.label)}</button>`).join(''));
  const back=$('canopy-back'); back.hidden=!value.back;
  if (value.back) {back.dataset.go=value.back.id;patchChildren(back,`<span data-key="b">${esc(value.back.label)} <b>${esc(value.back.title)}</b></span>`);}
  patchChildren($('canopy-legend'),value.legend.map(([st,label]) => `<span data-key="${st}"><i class="legend-mark ${st}"></i>${esc(label)}</span>`).join('')+`<span data-key="help" class="map-help">${esc(value.help)}</span>`);
}
function zoomAround(at,k,focused=false) {
  if (!mapView) return;
  hideTip();
  const v=canopyValue(), witness=witnessAt(at,focused);
  const next=clamp(k,v.levels[0].min,v.levels[v.levels.length-1].max);
  mapView.k=next;
  reanchor(witness);
  applyTransform();
  if (witness.world) witness.near=nearestPage(at);
  mapView.witness=witness;
  const want=levelOf(next);
  // A reversal to the still-drawn level must supersede an outstanding request too.
  if (want === (mapView.asked ?? mapView.drawn)) return;
  mapView.asked=want;
  post({t:'forest',do:'zoom',level:want,request:(mapView.request=++mapRequest),anchor:witness.page ?? null});
}
function centre() {const r=$('forest-canopy').getBoundingClientRect();return [r.width/2,r.height/2];}
function focusedPoint() {const el=canopyFocus(), node=el && mapView.nodes.get(el.dataset.key);return node ? screenOf(node) : centre();}
function viewportPoint(e) {const r=$('forest-canopy').getBoundingClientRect();return [e.clientX-r.left,e.clientY-r.top];}
$('forest-canopy').addEventListener('wheel',e => {
  if (!mapView) return; e.preventDefault();
  const dy=e.deltaY*(e.deltaMode ? 33 : 1),dx=e.deltaX*(e.deltaMode ? 33 : 1);
  if (Math.abs(dy)>=Math.abs(dx)) zoomAround(viewportPoint(e),mapView.k*Math.exp(-dy*.0021));
  else {hideTip();mapView.pan[0]-=dx;applyTransform();}
},{passive:false});
$('canopy-zoom').addEventListener('click',e => {
  const b=e.target.closest('[data-level]'); if (b && mapView) zoomAround(focusedPoint(),band(b.dataset.level).rest,true);
});
$('forest-canopy').addEventListener('pointerdown',e => {
  if (e.button !== 0 || !mapView) return;
  hideTip(); mapView.witness=null;mapView.moved=false;
  mapView.drag={x:e.clientX,y:e.clientY,pan:[...mapView.pan],moved:false,id:e.pointerId};
});
$('forest-canopy').addEventListener('pointermove',e => {
  const d=mapView?.drag; if (!d || d.id !== e.pointerId) return;
  const dx=e.clientX-d.x,dy=e.clientY-d.y;
  if (!d.moved && Math.hypot(dx,dy)<5) return;
  if (!d.moved) {d.moved=true;$('forest-canopy').setPointerCapture(d.id);$('forest-canopy').classList.add('dragging');}
  mapView.pan=[d.pan[0]+dx,d.pan[1]+dy];applyTransform();
});
function endDrag(e) {
  const d=mapView?.drag;if (!d || (e.pointerId !== undefined && d.id !== e.pointerId)) return;
  mapView.moved=d.moved;mapView.drag=null;$('forest-canopy').classList.remove('dragging');
}
for (const name of ['pointerup','pointercancel','lostpointercapture']) $('forest-canopy').addEventListener(name,endDrag);
window.addEventListener('blur',endDrag);
function canopyClick(e) {
  if (mapView?.moved) {mapView.moved=false;return;}
  const el=e.target.closest('.canopy-item'); if (!el || !mapView) return;
  const item=canopyValue().items[+el.dataset.index];
  if (item.kind === 'tree') post({t:'forest',do:'act',tree:item.id,act:'open'});
  else post({t:'forest',do:'go',id:item.go});
}
$('canopy-back').addEventListener('click',() => post({t:'forest',do:'go',id:$('canopy-back').dataset.go}));
function showTip(el) {
  hideTip();if (!mapView) return;
  mapView.tipKey=el.dataset.key;
  mapView.tipTimer=setTimeout(() => {
    const item=canopyValue()?.items[+el.dataset.index];if (!item?.tip || mapView?.drag || !el.isConnected) return;
    const tip=$('canopy-tip'),r=el.getBoundingClientRect(),v=$('forest-canopy').getBoundingClientRect();
    tip.textContent=item.tip;tip.hidden=false;
    tip.style.left=`${clamp(r.left-v.left,8,Math.max(8,v.width-tip.offsetWidth-8))}px`;
    tip.style.top=`${clamp(r.bottom-v.top+8,8,Math.max(8,v.height-tip.offsetHeight-8))}px`;
  },400);
}
function hideTip() {if (mapView) {clearTimeout(mapView.tipTimer);mapView.tipKey=null;}$('canopy-tip').hidden=true;}
$('forest-canopy').addEventListener('pointerover',e => {const el=e.target.closest('.canopy-item');if (el && !el.contains(e.relatedTarget)) showTip(el);});
$('forest-canopy').addEventListener('pointerout',e => {const el=e.target.closest('.canopy-item');if (el && !el.contains(e.relatedTarget)) hideTip();});
$('forest-canopy').addEventListener('focusin',e => {const el=e.target.closest('.canopy-item');if (el) {mapView.focus=el.dataset.key;const item=canopyValue().items[+el.dataset.index];mapView.focusPage=item.anchorPage ?? item.go ?? null;}});
$('forest').addEventListener('keydown',e => {
  if (e.key === 'Tab') queueMicrotask(() => {
    const el=document.activeElement.closest('.canopy-item');
    if (el && mapView) showTip(el);
  });
});
const CANOPY_STEP={ArrowLeft:0,ArrowRight:1,ArrowUp:2,ArrowDown:3};
function canopyKey(e) {
  if (!mapView) return false;
  const el=e.target.closest('.canopy-item'),item=el && canopyValue().items[+el.dataset.index];
  if (item && e.key in CANOPY_STEP) {const next=item.nav[CANOPY_STEP[e.key]];if (next !== null) focusItem(canopyValue().items[next].k);}
  else if (item && e.key === 'Enter') canopyClick({target:el});
  else if (['+','=','Add','-','Subtract'].includes(e.key)) zoomAround(focusedPoint(),mapView.k*(['-','Subtract'].includes(e.key) ? .8 : 1.25),true);
  else return false;
  e.preventDefault();return true;
}
function focusItem(key) {
  const node=mapView.nodes.get(key);if (!node) return;
  for (const [k,n] of mapView.nodes) n.el.tabIndex=k === key ? 0 : -1;
  mapView.focus=key;const item=canopyValue().items[+node.el.dataset.index];mapView.focusPage=item.anchorPage ?? item.go ?? null;mapView.witness=null;node.el.focus({preventScroll:true});
  const r=$('forest-canopy').getBoundingClientRect(),[x,y]=screenOf(node),box=node.el.getBoundingClientRect();
  for (const [i,p,extent,limit] of [[0,x,box.width/2,r.width],[1,y,box.height/2,r.height]])
    mapView.pan[i]+=Math.max(0,extent+8-p)-Math.max(0,p+extent+8-limit);
  applyTransform();
  showTip(node.el);
}
function canopyFocus() {return mapView?.nodes.get(mapView.focus ?? homeKey(canopyValue()))?.el;}
function canopyRoom() {
  const actual=$('forest-canopy').getBoundingClientRect();
  if (actual.width > 0 && actual.height > 0) return {width:Math.round(actual.width),height:Math.round(actual.height)};
  const pop=$('forest').querySelector('.forest-pop').getBoundingClientRect();
  const head=$('forest').querySelector('.forest-toolbar').getBoundingClientRect();
  return {width:Math.max(1,Math.round(pop.width)),height:Math.max(1,Math.round(pop.height-head.height-32))};
}
new ResizeObserver(() => {
  if (!mapView || S.overlay !== 'forest' || S.forest?.view !== 'canopy') return;
  const room=canopyRoom();
  if (mapView.room?.width === room.width && mapView.room?.height === room.height) return;
  mapView.room=room;post({t:'forest',do:'room',...room});
}).observe($('forest-canopy'));

$('canopy-layout').addEventListener('click',e => {
  const b=e.target.closest('[data-layout]');
  if (b && mapView) post({t:'forest',do:'layout',layout:b.dataset.layout});
});
window.canopy.canopyPinch = (factor,x,y) => {
  if (!mapView || S.overlay !== 'forest' || S.forest?.view !== 'canopy') return;
  const r=$('forest-canopy').getBoundingClientRect();
  if (x < r.left || y < r.top || x > r.right || y > r.bottom) return;
  zoomAround([x-r.left,y-r.top],mapView.k*factor);
};
