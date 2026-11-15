"""Tests the Bitwarden helper against a stand-in for bw.

The stand-in answers with the shapes bw 2026.10.1 gave with `--response`, recorded against
a real account on a test server.
"""

import base64
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).with_name("canopy-login-bitwarden")

# A bw that keeps its vault in vault.json next to itself. Its session key is "key".
FAKE_BW = r'''#!/usr/bin/env python3
import base64, json, os, sys
home = os.path.dirname(os.path.abspath(__file__))
vault_path = os.path.join(home, "vault.json")
vault = json.load(open(vault_path))
arguments = [a for a in sys.argv[1:] if a not in ("--response", "--nointeraction")]
open(os.path.join(home, "calls.log"), "a").write(" ".join(arguments) + "\n")

def answer(success, **rest):
    print(json.dumps({"success": success, **rest}))
    sys.exit(0 if success else 1)

signed_in = vault["signed_in"]
unlocked = os.environ.get("BW_SESSION") == "key"
if arguments[0] == "status":
    status = "unauthenticated" if not signed_in else "unlocked" if unlocked else "locked"
    answer(True, data={"object": "template", "template": {"userEmail": "probe@example.test", "status": status}})
if not signed_in:
    answer(False, message="You are not logged in.")
if arguments[0] == "unlock":
    if os.environ.get(arguments[2]) != "master":
        answer(False, message="Cryptography error, The decryption operation failed")
    answer(True, data={"object": "message", "title": "Your vault is now unlocked!", "raw": "key"})
if arguments[0] == "sync":
    answer(True, data={"object": "message", "title": "Syncing complete."})
if not unlocked:
    answer(False, message="Vault is locked.")
if arguments[:2] == ["list", "items"]:
    host = arguments[3].split("//")[1].split("/")[0].split(":")[0]
    found = [i for i in vault["items"] if any(host.endswith(u["uri"].split("//")[1]) for u in i["login"]["uris"])]
    answer(True, data={"object": "list", "data": found})
item = json.loads(base64.b64decode(sys.stdin.read()))
if arguments[:2] == ["create", "item"]:
    item["id"] = "new-%d" % len(vault["items"])
    vault["items"].append(item)
elif arguments[:2] == ["edit", "item"]:
    vault["items"] = [item if i["id"] == arguments[2] else i for i in vault["items"]]
json.dump(vault, open(vault_path, "w"))
answer(True, data=item)
'''

FAKE_DIALOG = "#!/bin/sh\n[ \"$FAKE_MASTER\" = cancel ] && exit 1\nprintf '%s\\n' \"$FAKE_MASTER\"\n"


def item(identifier, username, password, uri="https://example.test", **extra):
    login = {"username": username, "password": password, "uris": [{"match": None, "uri": uri}]}
    return {"id": identifier, "type": 1, "name": "example.test", "reprompt": 0, "login": login, **extra}


class HelperTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.home = Path(directory.name)
        for name, text in (("bw", FAKE_BW), ("zenity", FAKE_DIALOG)):
            program = self.home / name
            program.write_text(text)
            program.chmod(program.stat().st_mode | stat.S_IXUSR)
        self.vault(items=[item("a", "yan", "fake-yan"), item("b", "zoe", "fake-zoe", reprompt=1)])

    def vault(self, signed_in=True, items=()):
        (self.home / "vault.json").write_text(json.dumps({"signed_in": signed_in, "items": list(items)}))

    def items(self):
        return json.loads((self.home / "vault.json").read_text())["items"]

    def ask(self, verb, master="master", **request):
        request.setdefault("state", "key")
        environment = dict(os.environ, PATH=f"{self.home}:{os.environ['PATH']}", FAKE_MASTER=master,
                           XDG_CURRENT_DESKTOP="GNOME")
        result = subprocess.run([sys.executable, str(HELPER), verb], input=json.dumps(request),
                                capture_output=True, text=True, env=environment, check=True)
        return json.loads(result.stdout)

    def page(self, **extra):
        return {"url": "https://example.test/login", "origin": "https://example.test", **extra}

    def test_get_answers_the_matching_logins_and_leaves_out_reprompt_ones(self):
        self.assertEqual(self.ask("get", **self.page()),
                         {"logins": [{"username": "yan", "password": "fake-yan"}]})
        other = self.ask("get", url="https://other.test/", origin="https://other.test")
        self.assertEqual(other, {"logins": []})

    def test_a_missing_or_stale_session_is_locked(self):
        self.assertEqual(self.ask("get", **self.page(state=None)), {"locked": True})
        self.assertEqual(self.ask("get", **self.page(state="old")), {"locked": True})
        self.assertEqual(self.ask("store", **self.page(state=None, username="u", password="p")), {"locked": True})
        self.assertFalse((self.home / "calls.log").read_text().count("create"))

    def test_unlock_answers_the_session_or_why_not(self):
        self.assertEqual(self.ask("unlock", state=None), {"state": "key"})
        self.assertEqual(self.ask("unlock", master="cancel", state=None), {"cancelled": True})
        self.assertEqual(self.ask("unlock", master="wrong", state=None),
                         {"error": "Bitwarden did not accept that master password."})
        self.vault(signed_in=False)
        self.assertIn("bw login", self.ask("unlock", state=None)["error"])
        self.assertIn("bw login", self.ask("get", **self.page())["error"])

    def test_store_creates_a_login_for_the_origin(self):
        self.assertEqual(self.ask("store", **self.page(username="new", password="fake-new")), {})
        created = self.items()[-1]
        self.assertEqual((created["type"], created["name"]), (1, "example.test"))
        self.assertEqual(created["login"]["username"], "new")
        self.assertEqual(created["login"]["password"], "fake-new")
        self.assertEqual(created["login"]["uris"], [{"match": None, "uri": "https://example.test"}])
        self.assertEqual(len(self.items()), 3)

    def test_store_changes_only_the_password_of_a_saved_account(self):
        self.assertEqual(self.ask("store", **self.page(username="yan", password="fake-changed")), {})
        items = self.items()
        self.assertEqual(len(items), 2)
        self.assertEqual(items[0], item("a", "yan", "fake-changed"))
        self.assertEqual(items[1]["login"]["password"], "fake-zoe")

    def test_store_refuses_a_login_that_asks_for_the_master_password_again(self):
        answer = self.ask("store", **self.page(username="zoe", password="fake-changed"))
        self.assertIn("master password", answer["error"])
        self.assertEqual(self.items()[1]["login"]["password"], "fake-zoe")

    def test_the_password_never_rides_on_a_command_line(self):
        self.ask("unlock", state=None)
        self.ask("store", **self.page(username="new", password="fake-new"))
        calls = (self.home / "calls.log").read_text()
        self.assertNotIn("fake-new", calls)
        self.assertNotIn(base64.b64encode(b"fake-new").decode()[:8], calls)
        self.assertNotIn("master", calls.replace("CANOPY_BW_PASSWORD", ""))

    def test_an_unknown_question_is_an_error(self):
        self.assertIn("get, store or unlock", self.ask("explode")["error"])


if __name__ == "__main__":
    unittest.main()
