import hashlib
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

from import_chrome import EPOCH, extract
from datetime import datetime, timezone


def timestamp(value):
    delta = datetime.fromisoformat(value).astimezone(timezone.utc) - EPOCH
    return (delta.days * 86400 + delta.seconds) * 1_000_000 + delta.microseconds


class ImportTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.profile = self.root / "Profile 1"
        self.profile.mkdir()
        with closing(sqlite3.connect(self.profile / "History")) as db, db:
            db.executescript("""
                CREATE TABLE urls(id INTEGER PRIMARY KEY,url TEXT,title TEXT,visit_count INTEGER,
                    typed_count INTEGER,last_visit_time INTEGER,hidden INTEGER);
                CREATE TABLE visits(id INTEGER PRIMARY KEY,url INTEGER,visit_time INTEGER,
                    from_visit INTEGER,transition INTEGER,visit_duration INTEGER,opener_visit INTEGER);
                CREATE TABLE visit_source(id INTEGER PRIMARY KEY,source INTEGER);
            """)
            db.executemany("INSERT INTO urls VALUES(?,?,?,?,?,?,?)", [
                (1,"https://a.example/","A",2,1,0,0),
                (2,"https://b.example/","B",2,0,0,0),
                (3,"chrome://newtab/","New tab",0,0,0,0),
            ])
            db.executemany("INSERT INTO visits VALUES(?,?,?,?,?,?,?)", [
                (1,1,timestamp("2026-01-01T12:00:00+00:00"),0,1,0,0),
                (2,2,timestamp("2026-07-01T12:00:00+00:00"),1,0,0,0),
                (3,1,timestamp("2026-07-01T13:00:00+00:00"),2,3,0,0),
                (4,2,timestamp("2026-07-01T14:00:00+00:00"),0,0,0,0),
            ])
            db.execute("INSERT INTO visit_source VALUES(4,0)")
        payload = {"roots":{"bookmark_bar":{
            "id":"1","name":"Bookmarks bar","type":"folder","children":[{
                "id":"2","name":"Read later","type":"folder","children":[{
                    "id":"3","name":"Saved A","type":"url","url":"https://a.example/",
                    "date_added":str(timestamp("2026-01-01T12:00:00+00:00")),
                },{"id":"4","name":"Bookmarklet","type":"url","url":"javascript:void(0)"}],
            }],
        }}}
        (self.profile / "Bookmarks").write_text(json.dumps(payload))

    def test_capture_preserves_history_folders_dst_and_source_files(self):
        before = {name:hashlib.sha256((self.profile/name).read_bytes()).hexdigest()
                  for name in ("History","Bookmarks")}
        output = self.root / "capture"
        summary = extract(self.profile, output)
        self.assertEqual(summary["counts"]["visits"],4)
        self.assertEqual(summary["counts"]["habit_eligible_visits"],2)
        self.assertEqual(summary["counts"]["bookmarks"],2)
        self.assertEqual(summary["counts"]["bookmark_folders"],2)
        with closing(sqlite3.connect(output/"dataset.sqlite3")) as db:
            self.assertEqual(db.execute("SELECT local_hour FROM visits WHERE id IN (1,2) ORDER BY id").fetchall(),[(13,),(14,)])
            folder = db.execute("SELECT folder_path FROM bookmarks WHERE id='3'").fetchone()[0]
            self.assertEqual(json.loads(folder),["Bookmarks bar","Read later"])
            self.assertEqual(db.execute("SELECT * FROM observed_referrer_pairs").fetchall(),[("a.example","b.example",1)])
            self.assertEqual(db.execute("PRAGMA foreign_key_check").fetchall(),[])
        for name,digest in before.items():
            self.assertEqual(hashlib.sha256((self.profile/name).read_bytes()).hexdigest(),digest)
        self.assertEqual(output.stat().st_mode & 0o777,0o700)
        self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in output.iterdir()))

    def test_existing_capture_and_chrome_output_are_rejected(self):
        output = self.root / "capture"
        extract(self.profile,output)
        with self.assertRaises(FileExistsError):
            extract(self.profile,output)
        with self.assertRaises(ValueError):
            extract(self.profile,self.profile/"capture")

    def test_locked_history_recovers_only_committed_data(self):
        code = """import sqlite3,sys
db=sqlite3.connect(sys.argv[1])
db.execute('PRAGMA locking_mode=EXCLUSIVE')
db.execute('PRAGMA cache_size=1')
db.execute('BEGIN EXCLUSIVE')
db.execute(\"UPDATE urls SET title=? WHERE id=1\",('uncommitted'*2000,))
print('ready',flush=True)
sys.stdin.readline()
db.rollback()
db.close()
"""
        writer = subprocess.Popen([sys.executable,"-c",code,str(self.profile/"History")],
                                  stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        try:
            self.assertEqual(writer.stdout.readline().strip(),"ready")
            output = self.root/"locked-capture"
            summary = extract(self.profile,output)
            self.assertEqual(summary["history_capture"],"stable_file_bundle_with_sqlite_recovery")
            with closing(sqlite3.connect(output/"dataset.sqlite3")) as db:
                self.assertEqual(db.execute("SELECT title FROM urls WHERE id=1").fetchone()[0],"A")
                self.assertEqual(db.execute("SELECT COUNT(*) FROM visits").fetchone()[0],4)
        finally:
            writer.communicate("done\n",timeout=5)
        self.assertEqual(writer.returncode,0)


if __name__ == "__main__":
    unittest.main()
