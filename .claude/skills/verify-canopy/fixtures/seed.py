#!/usr/bin/env python3
"""Seeds a fresh Canopy data folder with stress trees for verify-canopy runs.

Usage: seed.py <forest.sqlite3> <port>
Every page lives on a *.localhost host served by server.py, so nothing leaves this machine.
"""
import os
import sqlite3
import sys
import time

DB, PORT = sys.argv[1], int(sys.argv[2])
ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'
now = int(time.time() * 1000)
clock = [now - 6 * 3600 * 1000]
serial = [0]


def ulid(at):
    serial[0] += 1
    value = (at << 80) | (int.from_bytes(os.urandom(8), 'big') << 16) | (serial[0] & 0xFFFF)
    return ''.join(ALPHABET[(value >> (5 * i)) & 31] for i in reversed(range(26)))


db = sqlite3.connect(DB, timeout=5)
db.execute('PRAGMA foreign_keys = ON')
trees = []


def page(parent, host, title, queued=False, step=40_000):
    clock[0] += step
    at = clock[0]
    vid = ulid(at)
    url = f'http://{host}.localhost:{PORT}/p/{title.replace(" ", "_")}'
    db.execute('INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?,?,?,?,?,?)',
               (vid, parent, url, title, at, None if queued else at))
    if not queued:
        db.execute("INSERT INTO visit_events(visit_id,from_visit_id,kind,at) VALUES(?,?,'new',?)", (vid, parent, at))
    return vid


def tree(root, last, name=None, settled=False, pin=None):
    place = 'tab' if not settled else ('shelf' if pin else 'history')
    position = None if place == 'history' else db.execute('SELECT COALESCE(MAX(position),-1)+1 FROM tree_state WHERE place=?', (place,)).fetchone()[0]
    db.execute('INSERT INTO tree_state(root_id,last_visit_id,name,place,position,pinned,placed_at) VALUES(?,?,?,?,?,?,?)',
               (root, last, name, place, position, int(place == 'tab' and pin is not None), clock[0]))
    trees.append((root, last))


# ---- A. The typical tree: one site, eight pages deep, one small side branch ----
a = [page(None, 'docs', 'Tokio tutorial · Getting started')]
for t in ['Hello Tokio', 'Spawning', 'Shared state', 'Channels', 'I/O', 'Framing', 'Async in depth']:
    a.append(page(a[-1], 'docs', f'Tokio tutorial · {t}'))
side = page(a[3], 'docs', 'Mutex in tokio::sync')
page(side, 'docs', 'RwLock in tokio::sync')
tree(a[0], a[-1])

# ---- B. Research fan-out: a search, twelve results, deep sub-branches, waiting pages ----
b0 = page(None, 'search', 'best 75% mechanical keyboard 2026 at Search')
r1 = page(b0, 'reddit', 'r/MechanicalKeyboards · Best 75% boards this year?')
r1a = page(r1, 'keychron', 'Keychron Q1 Max · Product page')
r1a1 = page(r1a, 'keychron', 'Keychron Q1 Max · Switch options')
r1a2 = page(r1a1, 'keychron', 'Keychron Q1 Max · Reviews')
r1b = page(r1, 'wobkey', 'Wobkey Rainy75 Pro')
page(r1b, 'youtube', 'Rainy75 sound test · 12 switches')
page(r1, 'imgur', 'My Rainy75 build album', queued=True)
page(r1, 'reddit', 'r/MechanicalKeyboards · Rainy75 long-term review', queued=True)
r2 = page(b0, 'rtings', 'The 6 Best 75% Keyboards · RTINGS')
r2a = page(r2, 'rtings', 'Keychron Q1 Max review · RTINGS')
r2b = page(r2a, 'rtings', 'Keychron Q1 Max vs NuPhy Halo75 V2')
r2c = page(r2b, 'nuphy', 'NuPhy Halo75 V2')
page(r2c, 'nuphy', 'NuPhy Halo75 V2 · Keycap profiles', queued=True)
page(r2c, 'nuphy', 'NuPhy Halo75 V2 · Firmware', queued=True)
page(r2b, 'rtings', 'Latency test methodology', queued=True)
page(r2, 'rtings', 'Wobkey Rainy75 review · RTINGS')
page(r2, 'rtings', 'Lemokey P1 Pro review · RTINGS', queued=True)
r3 = page(b0, 'youtube', 'Top 5 75% keyboards (2026) · Switch and Click')
page(r3, 'youtube', 'Why I stopped using gasket mounts')
r4 = page(b0, 'geekhack', 'Interest check · Zoom75 TIGA')
page(r4, 'geekhack', 'Zoom75 TIGA · Group buy thread')
page(b0, 'theverge', 'The best mechanical keyboards to buy right now', queued=True)
page(b0, 'wirecutter', 'The best compact mechanical keyboards', queued=True)
page(b0, 'tomshardware', 'Best gaming keyboards 2026', queued=True)
r5 = page(b0, 'amazon', 'Amazon · 75 percent keyboard')
page(r5, 'amazon', 'Epomaker x Aula F75')
page(r5, 'amazon', 'Royal Kludge R75')
page(b0, 'kbdfans', 'KBDfans · Tofu75 2.0')
page(b0, 'divinikey', 'Divinikey · In stock 75% kits')
tree(b0, r2c)

# ---- C. A deep trail: 34 pages, three sites, a side branch every fifth page ----
hosts = ['wikipedia', 'wikipedia', 'wikipedia', 'archive', 'jstor']
topics = ['Bonsai', 'Penjing', 'Tang dynasty', 'Silk Road', 'Sogdia', 'Samarkand', 'Timurid Empire', 'Ulugh Beg',
          'Ulugh Beg Observatory', 'Zij-i Sultani', 'Star catalogue', 'Hipparchus', 'Precession', 'Axial tilt',
          'Milankovitch cycles', 'Ice age', 'Last Glacial Maximum', 'Doggerland', 'Storegga Slide', 'Tsunami',
          'Lisbon earthquake 1755', 'Candide', 'Voltaire', 'Ferney', 'Lake Geneva', 'Rhone', 'Camargue', 'Flamingo',
          'Carotenoid', 'Lycopene', 'Tomato', 'Columbian exchange', 'Potato', 'Great Famine']
c = []
for i, topic in enumerate(topics):
    c.append(page(c[-1] if c else None, hosts[i % len(hosts)], f'{topic} · Wikipedia' if hosts[i % 5] == 'wikipedia' else topic))
    if i % 5 == 4 and i < 30:
        s = page(c[-1], 'wikipedia', f'See also · {topic}')
        page(s, 'wikipedia', f'Talk · {topic}', queued=(i % 10 == 9))
tree(c[0], c[-1])

# ---- D. A thread, read by middle-clicking: 14 waiting pages and 4 opened ones ----
d0 = page(None, 'news', 'Ask HN · What are you working on this month?')
for n, t in enumerate(['A Rust browser that keeps a tree of visits', 'Self-hosted photo library', 'CAD kernel in Zig',
                       'Offline maps for hikers']):
    o = page(d0, ['github', 'photos', 'ziglang', 'maps'][n], t)
    if n < 2:
        page(o, ['github', 'photos'][n], f'{t} · README')
for t in ['Solar forecast for balconies', 'A font for dyslexic readers', 'Tiny SAT solver', 'Bus tracker for Lyon',
          'Postgres on a Game Boy', 'Paper-first todo app', 'Keyboard layout analyzer', 'Mushroom identifier',
          'WASM audio workstation', 'Local-first spreadsheet', 'Cellular automata toy', 'Recipe scaler',
          'Sourdough logbook', 'Ham radio waterfall']:
    page(d0, t.split()[0].lower().strip('-'), t, queued=True, step=9_000)
tree(d0, d0, pin=1)

# ---- Filler trees, so the tab row overflows ----
for i, (host, title) in enumerate([
        ('mail', 'Inbox'), ('calendar', 'Week 47'), ('github', 'canopy · Pull requests'),
        ('crates', 'wry 0.48 · crates.io'), ('weather', 'Lyon · 10 day forecast'),
        ('maps', 'Train to Annecy'), ('wikipedia', 'Canopy (biology) · Wikipedia'),
        ('news', 'Hacker News'), ('bank', 'Accounts'), ('music', 'Now playing · Radio'),
        ('recipes', 'Tarte tatin, the slow way'), ('docs', 'GTK 3 reference · GtkFixed')]):
    root = page(None, host, title, step=600_000)
    last = page(root, host, f'{title} · detail') if i % 3 == 0 else root
    tree(root, last, pin=2 if i == 2 else None)
for host, title in [('shop', 'Desk lamp search'), ('travel', 'Kyoto in November'), ('forum', 'Repotting the juniper')]:
    root = page(None, host, title, step=900_000)
    tree(root, page(root, host, f'{title} · notes'), settled=True)

db.execute('UPDATE session SET current_id=? WHERE singleton=1', (trees[0][1],))
db.commit()
print(db.execute('SELECT count(*) FROM visits').fetchone()[0], 'visits,', len(trees), 'trees')
