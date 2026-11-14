#!/usr/bin/env python3
"""Capture one Chrome profile into a private, standalone research dataset."""

import argparse
import contextlib
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import time
from urllib.parse import urlsplit
from zoneinfo import ZoneInfo


EPOCH = datetime(1601, 1, 1, tzinfo=timezone.utc)


def chrome_time(value):
    if not value or int(value) <= 0:
        return None
    try:
        return EPOCH + timedelta(microseconds=int(value))
    except (ValueError, OverflowError):
        return None


def iso(value):
    return value.isoformat(timespec="microseconds").replace("+00:00", "Z") if value else None


def signatures(paths):
    result = {}
    for path in paths:
        try:
            stat = path.stat()
            result[path.name] = (stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns)
        except FileNotFoundError:
            pass
    return result


@contextlib.contextmanager
def snapshot_history(profile, work):
    source = profile / "History"
    target = work / "History"
    method = "sqlite_online_backup"
    with contextlib.closing(sqlite3.connect(source.as_uri() + "?mode=ro", uri=True, timeout=0.3)) as original:
        try:
            original.execute("SELECT name FROM sqlite_master LIMIT 1").fetchone()
            deadline = time.monotonic() + 5

            def progress(*_):
                if time.monotonic() > deadline:
                    raise TimeoutError("Chrome history remained busy during backup")

            with contextlib.closing(sqlite3.connect(target)) as destination:
                original.backup(destination, pages=512, progress=progress, sleep=0.05)
        except (sqlite3.OperationalError, TimeoutError) as error:
            if not isinstance(error, TimeoutError) and "locked" not in str(error).lower() and "busy" not in str(error).lower():
                raise
            method = "stable_file_bundle_with_sqlite_recovery"
            paths = [source, profile / "History-wal", profile / "History-journal"]
            for attempt in range(8):
                for name in ("History", "History-wal", "History-journal", "History-shm"):
                    (work / name).unlink(missing_ok=True)
                before = signatures(paths)
                for path in paths:
                    if path.name in before:
                        shutil.copyfile(path, work / path.name)
                if before == signatures(paths):
                    break
                time.sleep(0.1 * (attempt + 1))
            else:
                raise RuntimeError("Chrome kept writing its history. Retry when browsing is idle.")
    with contextlib.closing(sqlite3.connect(target)) as copied:
        if copied.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
            raise RuntimeError("The copied Chrome history failed its integrity check")
        yield copied, method


def stable_bookmarks(profile):
    path = profile / "Bookmarks"
    if not path.exists():
        return None
    for _ in range(5):
        before = signatures([path])
        raw = path.read_bytes()
        if before == signatures([path]):
            return json.loads(raw)
    raise RuntimeError("Chrome kept changing its bookmarks. Retry when browsing is idle.")


def create_schema(db):
    db.executescript("""
        PRAGMA foreign_keys=ON;
        CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE urls(
            id INTEGER PRIMARY KEY, url TEXT NOT NULL, title TEXT NOT NULL,
            visit_count INTEGER, typed_count INTEGER, last_visit_time INTEGER,
            hidden INTEGER, hostname TEXT, scheme TEXT
        );
        CREATE TABLE visits(
            id INTEGER PRIMARY KEY, url INTEGER NOT NULL REFERENCES urls(id),
            visit_time INTEGER, from_visit INTEGER, transition INTEGER,
            visit_duration INTEGER, opener_visit INTEGER, source INTEGER,
            visited_at_utc TEXT, local_date TEXT, local_hour INTEGER, local_weekday INTEGER
        );
        CREATE INDEX visits_url ON visits(url);
        CREATE INDEX visits_time ON visits(visit_time);
        CREATE INDEX visits_referrer ON visits(from_visit);
        CREATE TABLE bookmarks(
            id TEXT PRIMARY KEY, parent_id TEXT, type TEXT NOT NULL,
            title TEXT NOT NULL, url TEXT, folder_path TEXT NOT NULL,
            date_added INTEGER, added_at_utc TEXT
        );
        CREATE VIEW habit_visits AS
            SELECT v.*,u.url AS page_url,u.title,u.hostname
            FROM visits v JOIN urls u ON u.id=v.url
            WHERE u.scheme IN ('http','https') AND u.hostname IS NOT NULL
              AND v.visited_at_utc IS NOT NULL
              AND (v.transition & 255) NOT IN (3,4)
              AND (v.source IS NULL OR v.source=1);
        CREATE VIEW habit_domains AS
            SELECT hostname,COUNT(*) AS visits,COUNT(DISTINCT url) AS distinct_pages,
                COUNT(DISTINCT local_date) AS active_days,
                SUM((transition & 255)=1) AS explicit_navigation_visits,
                MIN(visited_at_utc) AS first_visit_utc,MAX(visited_at_utc) AS last_visit_utc
            FROM habit_visits GROUP BY hostname;
        CREATE VIEW habit_pages AS
            SELECT url AS url_id,page_url,title,hostname,COUNT(*) AS visits,
                COUNT(DISTINCT local_date) AS active_days,
                MAX(visited_at_utc) AS last_visit_utc
            FROM habit_visits GROUP BY url;
        CREATE VIEW habit_hours AS
            SELECT local_hour,COUNT(*) AS visits FROM habit_visits GROUP BY local_hour;
        CREATE VIEW habit_weekdays AS
            SELECT local_weekday,COUNT(*) AS visits FROM habit_visits GROUP BY local_weekday;
        CREATE VIEW observed_referrer_pairs AS
            SELECT p.hostname AS from_hostname,c.hostname AS to_hostname,COUNT(*) AS visits
            FROM habit_visits c JOIN habit_visits p ON p.id=c.from_visit
            WHERE c.id<>p.id GROUP BY p.hostname,c.hostname;
    """)


def import_history(source, destination, local_zone):
    for row in source.execute("SELECT id,url,title,visit_count,typed_count,last_visit_time,hidden FROM urls"):
        identifier, url, title, visit_count, typed_count, last_visit, hidden = row
        try:
            parsed = urlsplit(url)
            hostname, scheme = parsed.hostname, parsed.scheme.lower()
        except ValueError:
            hostname, scheme = None, ""
        destination.execute("INSERT INTO urls VALUES(?,?,?,?,?,?,?,?,?)", (
            identifier, url, title or "", visit_count, typed_count, last_visit, hidden, hostname, scheme,
        ))
    fields = {row[1] for row in source.execute("PRAGMA table_info(visits)")}
    tables = {row[0] for row in source.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    optional = [f"v.{field}" if field in fields else "0" for field in ("visit_duration", "opener_visit")]
    origin = "s.source" if "visit_source" in tables else "NULL"
    join = "LEFT JOIN visit_source s ON s.id=v.id" if "visit_source" in tables else ""
    query = f"SELECT v.id,v.url,v.visit_time,v.from_visit,v.transition,{','.join(optional)},{origin} FROM visits v {join}"
    for row in source.execute(query):
        value = chrome_time(row[2])
        local = value.astimezone(local_zone) if value else None
        destination.execute("INSERT INTO visits VALUES(?,?,?,?,?,?,?,?,?,?,?,?)", (*row,
            iso(value), local.date().isoformat() if local else None,
            local.hour if local else None, local.weekday() if local else None,
        ))


def import_bookmarks(payload, db):
    if payload is None:
        return
    stack = [(node, None, [], "root:" + name) for name, node in payload["roots"].items()]
    while stack:
        node, parent_id, path, fallback_id = stack.pop()
        identifier = str(node.get("id", fallback_id))
        kind = node.get("type", "folder")
        title = node.get("name", "")
        added = int(node.get("date_added") or 0)
        db.execute("INSERT INTO bookmarks VALUES(?,?,?,?,?,?,?,?)", (
            identifier, parent_id, kind, title, node.get("url"),
            json.dumps(path, ensure_ascii=False), added, iso(chrome_time(added)),
        ))
        for index, child in reversed(list(enumerate(node.get("children", [])))):
            stack.append((child, identifier, path + [title], f"{identifier}:{index}"))


def rows(db, query):
    cursor = db.execute(query)
    names = [item[0] for item in cursor.description]
    return [dict(zip(names, row)) for row in cursor]


def summarize(db, profile, captured, method, zone, bookmarks_available):
    counts = {name: db.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] for name, table in (
        ("urls", "urls"), ("visits", "visits"), ("habit_eligible_visits", "habit_visits"),
    )}
    counts["bookmarks"] = db.execute("SELECT COUNT(*) FROM bookmarks WHERE type='url'").fetchone()[0]
    counts["bookmark_folders"] = db.execute("SELECT COUNT(*) FROM bookmarks WHERE type='folder'").fetchone()[0]
    counts["retained_referrer_links"] = db.execute("SELECT COUNT(*) FROM visits v JOIN visits p ON p.id=v.from_visit WHERE v.id<>p.id").fetchone()[0]
    counts["missing_referrer_targets"] = db.execute("SELECT COUNT(*) FROM visits v LEFT JOIN visits p ON p.id=v.from_visit WHERE v.from_visit<>0 AND p.id IS NULL").fetchone()[0]
    first, last = db.execute("SELECT MIN(visited_at_utc),MAX(visited_at_utc) FROM visits").fetchone()
    hours = dict(db.execute("SELECT local_hour,visits FROM habit_hours"))
    weekdays = dict(db.execute("SELECT local_weekday,visits FROM habit_weekdays"))
    return {
        "profile": profile.name, "source_profile": str(profile), "captured_at_utc": captured,
        "history_capture": method, "timezone": zone, "counts": counts,
        "date_range_utc": {"first": first, "last": last},
        "bookmarks_available": bookmarks_available,
        "top_domains": rows(db, "SELECT * FROM habit_domains ORDER BY visits DESC,hostname LIMIT 30"),
        "frequent_pages": rows(db, "SELECT * FROM habit_pages ORDER BY visits DESC,url_id LIMIT 30"),
        "hourly_visits": [{"hour": hour, "visits": hours.get(hour, 0)} for hour in range(24)],
        "weekday_visits": [{"weekday": day, "visits": weekdays.get(day, 0)} for day in range(7)],
        "referrer_pairs": rows(db, "SELECT * FROM observed_referrer_pairs ORDER BY visits DESC,from_hostname,to_hostname LIMIT 30"),
        "limitations": [
            "Only records currently retained in this profile are available.",
            "Habits use HTTP(S) visits, excluding subframes and explicitly synced, extension, or imported visit sources.",
            "A visit with no visit_source row is included, matching Chrome's ordinary browsing default.",
            "Visit counts do not measure attention or time spent reading. Raw recorded durations are retained without interpreting them.",
            "Referrer links are recorded metadata, not a reconstructed tab history or a claim about research intent.",
            "URLs, titles, and bookmark folder names are private data and can include sensitive query strings.",
        ],
        "format_sources": [
            "https://raw.githubusercontent.com/chromium/chromium/main/base/time/time.h",
            "https://raw.githubusercontent.com/chromium/chromium/main/components/history/core/browser/history_types.h",
            "https://raw.githubusercontent.com/chromium/chromium/main/components/history/core/browser/visit_database.cc",
            "https://raw.githubusercontent.com/chromium/chromium/main/ui/base/page_transition_types.h",
            "https://www.sqlite.org/backup.html",
            "https://www.sqlite.org/howtocorrupt.html",
        ],
    }


def extract(profile, output, zone_name="Europe/Paris"):
    profile, output = Path(profile).expanduser().resolve(), Path(output).expanduser().resolve()
    if output.is_relative_to(profile):
        raise ValueError("The output directory must be outside the Chrome profile")
    if not (profile / "History").is_file():
        raise FileNotFoundError("The selected profile has no History file")
    if output.exists():
        raise FileExistsError("Choose a new output directory; existing captures are never overwritten")
    zone = ZoneInfo(zone_name)
    captured = iso(datetime.now(timezone.utc))
    output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.TemporaryDirectory(prefix=".chrome-import-", dir=output.parent) as staging:
        staging = Path(staging)
        with tempfile.TemporaryDirectory(prefix="snapshot-", dir=staging) as work:
            payload = stable_bookmarks(profile)
            with snapshot_history(profile, Path(work)) as (source, method):
                with contextlib.closing(sqlite3.connect(staging / "dataset.sqlite3")) as db:
                    create_schema(db)
                    with db:
                        import_history(source, db, zone)
                        import_bookmarks(payload, db)
                        for table in ("urls", "visits"):
                            expected = source.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                            actual = db.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                            if actual != expected:
                                raise RuntimeError(f"The {table} count differs from the source snapshot")
                        db.executemany("INSERT INTO metadata VALUES(?,?)", [
                            ("profile", profile.name), ("source_profile", str(profile)),
                            ("captured_at_utc", captured), ("timezone", zone_name), ("history_capture", method),
                        ])
                    if db.execute("PRAGMA foreign_key_check").fetchall():
                        raise RuntimeError("The imported visits have missing URL records")
                    if db.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
                        raise RuntimeError("The imported dataset failed its integrity check")
                    summary = summarize(db, profile, captured, method, zone_name, payload is not None)
        (staging / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
        counts = summary["counts"]
        (staging / "README.md").write_text(
            f"# Chrome dataset\n\nCaptured {captured} from {profile.name}.\n\n"
            f"The capture contains {counts['visits']:,} visits, {counts['urls']:,} URLs, "
            f"{counts['bookmarks']:,} bookmarks, and {counts['bookmark_folders']:,} bookmark folders.\n\n"
            "`dataset.sqlite3` contains the normalized records and SQL views for habits. "
            "`summary.json` contains counts, top domains, frequent pages, local-hour and weekday distributions, "
            "and recorded referrer pairs. Weekdays run from Monday=0 to Sunday=6.\n\n"
            "The capture is separate from Canopy's live browsing database. No imported URL was opened or fetched. "
            "The input consisted only of Chrome's History database and Bookmarks file. "
            "No cookies, credentials, saved form data, or page bodies were imported.\n\n"
            "The snapshot method and interpretation limits are recorded in summary.json. "
            "Keep this directory local; its URLs and titles can contain private data.\n"
        )
        for path in staging.iterdir():
            path.chmod(0o600)
        if output.exists():
            raise FileExistsError("The output directory appeared during import")
        staging.rename(output)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timezone", default="Europe/Paris")
    args = parser.parse_args()
    os.umask(0o077)
    summary = extract(args.profile, args.output, args.timezone)
    print(json.dumps({"output": str(args.output), "profile": summary["profile"],
                      "counts": summary["counts"], "date_range_utc": summary["date_range_utc"],
                      "history_capture": summary["history_capture"]}, indent=2))


if __name__ == "__main__":
    main()
