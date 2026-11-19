#!/usr/bin/env python3
"""Copy a timing forest and spread its settled trees across eighteen months."""
import argparse
import datetime as dt
import pathlib
import sqlite3

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("source", type=pathlib.Path)
parser.add_argument("destination", type=pathlib.Path)
parser.add_argument("--today", type=dt.date.fromisoformat, default=dt.date.today())
args = parser.parse_args()
if args.source.resolve() == args.destination.resolve() or args.destination.exists():
    parser.error("destination must be a new copy")
args.destination.parent.mkdir(parents=True, exist_ok=True)
with sqlite3.connect(f"file:{args.source.resolve()}?mode=ro", uri=True) as source:
    with sqlite3.connect(args.destination) as copied:
        source.backup(copied)
with sqlite3.connect(args.destination) as db:
    columns = {row[1] for row in db.execute("PRAGMA table_info(tree_state)")}
    column, predicate = ("placed_at", "place='history'") if "place" in columns else ("settled_at", "settled_at IS NOT NULL AND pin_order IS NULL")
    trees = list(db.execute(f"SELECT root_id FROM tree_state WHERE {predicate} ORDER BY {column} DESC, root_id"))
    for i, (tree,) in enumerate(trees):
        day = args.today - dt.timedelta(days=i * 548 // max(1, len(trees) - 1))
        at = dt.datetime.combine(day, dt.time(12, i % 60)).astimezone()
        db.execute(f"UPDATE tree_state SET {column}=? WHERE root_id=?", (int(at.timestamp() * 1000), tree))
    db.commit()
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
print(f"Spread {len(trees)} settled trees through {args.today} in {args.destination}")
