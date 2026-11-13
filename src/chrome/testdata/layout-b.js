// Copied verbatim from the layout D prototype (tree-ui-prototype/src/app.html).
function chainOf(F, id, max) {
  const ids = [id];
  for (let x = id; ids.length < max;) {
    const ks = F.dkids(x); if (!ks.length) break;
    x = ks[ks.length - 1].id; ids.push(x);
  }
  return {ids, hidden: F.count(id) - (ids.length - 1)};
}
function rowsOf(F, id) {
  const rows = [{ids: [], col0: 0, parent: null}];
  (function walk(nid, col, row) {
    row.ids.push(nid);
    const ks = F.dkids(nid).sort((a, b) => b.at - a.at);
    ks.forEach((c, j) => {
      if (j === 0) walk(c.id, col + 1, row);
      else { const r = {ids: [], col0: col + 1, parent: nid}; rows.push(r); walk(c.id, col + 1, r); }
    });
  })(id, 0, rows[0]);
  return rows;
}

function layoutB(F, W, opt = {}) {
  const open = opt.open !== false, pill = !!opt.pill, down = !!opt.down, restH = opt.rest || 52;
  const H = open ? (down ? restH / 2 + 94 : 112) : restH, Y = down ? restH / 2 : H / 2, PAD = 22;
  const OFF = open ? (down ? [0, 27, 46, 65, 84] : [0, 25, 45]) : [0, restH * .27, restH * .42], MAXB = open && down ? 4 : 2;
  const path = F.path(), k = path.length - 1, cv = F.get(path[k]);
  const pillW = pill ? Math.min(250, 7 + 16 + 8 + Math.max(tw(cv.title), tw(SITE[cv.host] || cv.host, "11px " + PF)) + 16) : 32;
  const extra = pillW - 32;
  const HALF = Math.max(23, Math.min(36, (W - 2 * PAD - 40 - extra) / (2 * k + 10)));
  const nodes = [], edges = [], X = u => PAD + 16 + u * HALF + (u > 2 * k ? extra : 0);
  const bud = (n, d) => open ? n : Object.assign(n, {kind: "dot", s: d, w: d, badge: undefined, cls: n.cls.filter(c => c !== "raw" && c !== "sm")});
  const occ = {};
  const free = (sb, u) => !(occ[sb]?.has(u - 1) || occ[sb]?.has(u));
  const take = (sb, u) => { const s = occ[sb] ??= new Set(); s.add(u - 1); s.add(u); };
  const yOf = (side, band) => side === "mid" ? Y : side === "up" ? Y - OFF[band] : Y + OFF[band];
  const badgeOn = new Map();
  nodes.push({key: "glow", kind: "glow", x: X(2 * k) + (pill ? extra / 2 - pillW / 2 + 16 : 0), y: Y, s: open ? 104 : 80, cls: []});
  path.forEach((id, i) => {
    if (i === k && pill) { nodes.push(mk(F, id, X(2 * k) + extra / 2, Y, open ? 32 : 30, {kind: "pill", w: pillW, cls: ["path"]})); if (i) edges.push({a: path[i - 1], b: id, cls: "path"}); return; }
    nodes.push(mk(F, id, X(2 * i), Y, i === k ? 32 : open ? 28 : 26, {cls: ["path"]}));
    if (i) edges.push({a: path[i - 1], b: id, cls: "path"});
  });
  // next steps from the current page: continuation on the trunk line, the rest fanned around it
  const fwd = [...F.dkids(path[k])].sort((a, b) => ((b.state === "opened" && b.by === "me") - (a.state === "opened" && a.by === "me")) || b.at - a.at);
  const slots = down ? [["mid", 0], ["down", 1], ["down", 2], ["down", 3], ["down", 4]].slice(0, MAXB + 1) : [["mid", 0], ["up", 1], ["down", 1], ["up", 2], ["down", 2]];
  fwd.forEach((c, j) => {
    if (j >= slots.length) return;
    const [side, band] = slots[j], sb = side + band;
    const ch = chainOf(F, c.id, 4);
    ch.ids.forEach((id, r) => {
      const u = 2 * k + 2 + 2 * r;
      take(sb, u);
      const last = r === ch.ids.length - 1;
      nodes.push(bud(mk(F, id, X(u), yOf(side, band), r === 0 ? 22 : 14, {cls: r ? ["raw"] : [], badge: last && ch.hidden ? "+" + ch.hidden : undefined}), side === "mid" && r === 0 ? 10 : 7));
      edges.push({a: r ? ch.ids[r - 1] : path[k], b: id, cls: ecls(F, id)});
    });
  });
  if (fwd.length > slots.length) badgeOn.set(path[k], (badgeOn.get(path[k]) || 0) + fwd.slice(slots.length).reduce((s, c) => s + 1 + F.count(c.id), 0));
  // side branches, nearest to the current page first
  for (let i = k - 1; i >= 0; i--) {
    const alts = F.dkids(path[i]).filter(c => c.id !== path[i + 1]).sort((a, b) => b.at - a.at);
    const u0 = 2 * i + 1;
    alts.forEach((c, j) => {
      const rows = rowsOf(F, c.id), total = 1 + F.count(c.id);
      let best = null;
      const sides = down ? ["down"] : j % 2 ? ["down", "up"] : ["up", "down"];
      for (const side of sides) {
        for (let band = 1; band <= MAXB; band++) {
          for (let R = Math.min(rows.length, MAXB - band + 1); R >= 1; R--) {
            const p = []; let ok = true;
            for (let r = 0; r < R && ok; r++) {
              const row = rows[r], sb = side + (band + r);
              if (r > 0 && !p.some(q => q.row.ids.slice(0, q.n).includes(row.parent))) break;
              let n = 0;
              while (n < row.ids.length && free(sb, u0 + 2 * (row.col0 + n))) n++;
              if (n === 0) { if (r === 0) ok = false; break; }
              p.push({row, sb, side, band: band + r, n});
            }
            if (!ok || !p.length) continue;
            const score = p.reduce((t, q) => t + q.n, 0) * 10 - band - (side === sides[0] ? 0 : .5);
            if (!best || score > best.score) best = {score, p};
          }
        }
      }
      const plan = best && best.p;
      if (!plan) { badgeOn.set(path[i], (badgeOn.get(path[i]) || 0) + total); return; }
      let shown = 0;
      for (const {row, sb, side, band, n} of plan) {
        row.ids.slice(0, n).forEach((id, m) => {
          const col = row.col0 + m, u = u0 + 2 * col;
          take(sb, u);
          shown++;
          nodes.push(bud(mk(F, id, X(u), yOf(side, band), col === 0 ? 20 : 14, {cls: col ? ["raw"] : []}), col === 0 ? 8 : 6));
          const fromTrunk = !m && !row.parent;
          edges.push({a: m ? row.ids[m - 1] : (row.parent || path[i]), b: id, from: fromTrunk ? (side === "up" ? "top" : "bottom") : undefined, cls: ecls(F, id)});
        });
      }
      if (open && total > shown) { const rn = nodes.find(n => n.key === c.id); rn.badge = "+" + (total - shown); }
    });
  }
  if (open) for (const [id, n] of badgeOn) { const pn = nodes.find(x => x.key === id); pn.badge = "+" + n; }
  const width = Math.max(...nodes.map(n => n.x)) + PAD + 20;
  return {nodes, edges, width, height: H, focus: X(2 * k)};
}

