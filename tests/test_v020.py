"""0.2.0: `access: none` is no workspace at all (compose, explain, why, siblings' narrowing, both backends' start
folder and what they leave unseen); a credential or mcp binding with `via: file` gets its value in a file of the
run's /run/pot/secrets, which its variable names. Run with the other unit tests (pot-unit), on any system: the
backends' runs are tests/run.sh's (Seatbelt's on macOS)."""
import json
import os
import shutil
import stat
import tempfile
import unittest

import bwrap as B
import compose as C
import pot
import seatbelt as S
from test_compose import granted, inp, root, ws  # noqa: F401

BLIND = {"blind": {"extends": "work", "access": "none"}}


class AccessNone(unittest.TestCase):
    def test_policy(self):
        p, _ = C.compose(inp(r=root(contexts=BLIND), context="blind"))
        self.assertEqual(p["access"], "none")
        w = p["workspace"][0]
        self.assertEqual((w["protected"], w["sealed"], w["proposal"], w["clone"]), ([], [], [], None))

    def test_a_clone_is_refused(self):
        # a clone that the run never sees: refused, before pot makes one (pot.py's clone_mode_checks) and here
        r = root(contexts={"blindclone": {"extends": "dispatch", "access": "none"}})
        with self.assertRaisesRegex(C.Refusal, r"access none: the run sees no workspace, so it has no clone"):
            C.compose(inp(r=r, context="blindclone"))
        with self.assertRaisesRegex(C.Refusal, r"access none"):
            C.compose(inp(r=root(contexts=BLIND), context="blind", flags={"mode": "clone"}))

    def test_none_is_narrower_than_read(self):
        blind, _ = C.compose(inp(r=root(contexts=BLIND), context="blind"))
        read, _ = C.compose(inp(flags={"access": "read"}))
        self.assertEqual(C.narrower(blind, read), [])
        self.assertEqual(C.narrower(read, blind), ["read access"])
        # a parent that sees no workspace covers no read inside it
        child = dict(blind, items=blind["items"] + [{"kind": "read", "value": "~/src/r/x", "path": "/home/u/src/r/x",
                                                      "class": "reach", "effective": True, "sources": []}])
        self.assertEqual(C.narrower(child, blind), ["~/src/r/x"])
        self.assertEqual(C.narrower(child, read), [])

    def test_explain(self):
        p, rep = C.compose(inp(r=root(contexts=BLIND), context="blind"))
        t = C.explain_text(p, rep, root(contexts=BLIND))
        self.assertIn("workspace  repo:example.com/o/r (none: not in the run) /home/u/src/r", t)
        self.assertIn("limits     no workspace (access none): the run does not see it, and starts in its home", t)
        self.assertNotIn("the workspace is read-only", t)

    def test_why(self):
        p, _ = C.compose(inp(r=root(contexts=BLIND), context="blind"))
        t, reached = pot.path_answer(p, "/home/u/src/r/x", "~/src/r/x", "pot (blind):", "/home/u", {}, lambda i: "")
        self.assertEqual((t, reached), ("pot (blind): ~/src/r/x is in the workspace repo:example.com/o/r, which this "
                                        "run does not see (access none)", False))


class Backends(unittest.TestCase):
    POL = {"access": "none", "workspace": [{"path": "/w/r", "common": "/w/r/.git"},
                                           {"path": "/w/s", "common": "/g/s.git"}]}

    def test_start_folder(self):
        launch = {"home": "/home/u", "cwd": "/w/r/sub"}
        self.assertEqual(B.start_dir(self.POL, launch), "/home/u")
        self.assertEqual(B.start_dir(dict(self.POL, access="read"), launch), "/w/r/sub")

    def test_unseen(self):
        self.assertEqual(B.unseen(self.POL), ["/w/r", "/w/s", "/g/s.git"])
        self.assertEqual(B.unseen(dict(self.POL, access="read")), [])

    def test_seatbelt_denies_it(self):
        # Seatbelt: the run starts in its own HOME; the workspace and its git directory are denied over every allow
        # (inspect's view reads the whole file system), never allowed
        self.assertEqual(S.start_dir(self.POL, {"cwd": "/w/r/sub"}, "/st/runs/ab12/home"), "/st/runs/ab12/home")
        self.assertEqual(S.start_dir(dict(self.POL, access="write"), {"cwd": "/w/r/sub"}, "/r/home"), "/w/r/sub")
        self.assertEqual(S.unseen(self.POL), [S.real(p) for p in ("/w/r", "/w/s", "/g/s.git")])


class ViaFile(unittest.TestCase):
    POL = {"bindings": [
        {"capability": "credential", "selector": "a", "via": "file", "env": ["TOK"], "from": {"file": "~/k"}},
        {"capability": "credential", "selector": "b", "via": "env", "env": ["PLAIN"], "from": {"file": "~/k2"}},
        {"capability": "mcp", "selector": "m", "via": "file", "env": ["MKEY"], "from": {"file": "~/m.env"},
         "server": "m", "url": "https://m.example/mcp", "header": "Authorization", "scheme": "Bearer"},
        {"capability": "mcp", "selector": "n", "via": "env", "env": ["NKEY"], "from": {},
         "server": "n", "url": "https://n.example/mcp", "header": "X-Key", "scheme": ""}]}

    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.d, ignore_errors=True))

    def test_files_and_values(self):
        B.make_secrets(self.POL, self.d)
        sd = os.path.join(self.d, "secrets")
        self.assertEqual(stat.S_IMODE(os.stat(sd).st_mode), 0o700)
        env = B.deliver(self.POL, self.d, "/run/pot/secrets", {"TOK": "tok-v", "PLAIN": "plain-v", "MKEY": "m-v", "NKEY": "n-v"})
        self.assertEqual(env, {"TOK": "/run/pot/secrets/TOK", "PLAIN": "plain-v", "MKEY": "/run/pot/secrets/MKEY", "NKEY": "n-v"})
        for n, v in (("TOK", "tok-v"), ("MKEY", "m-v")):
            f = os.path.join(sd, n)
            self.assertEqual(open(f).read(), v)
            st = os.lstat(f)
            self.assertEqual((stat.S_IMODE(st.st_mode), st.st_uid), (0o400, os.getuid()))
        self.assertEqual(sorted(os.listdir(sd)), ["MKEY", "TOK"])      # never a via env value

    def test_no_folder_without_a_file_binding(self):
        B.make_secrets({"bindings": [self.POL["bindings"][1]]}, self.d)
        self.assertFalse(os.path.exists(os.path.join(self.d, "secrets")))

    def test_cleanup_removes_them(self):
        B.make_secrets(self.POL, self.d)
        B.deliver(self.POL, self.d, "/run/pot/secrets", {"TOK": "tok-v"})
        B.cleanup(self.d)
        self.assertFalse(os.path.exists(os.path.join(self.d, "secrets")))

    def test_mcp_header_from_the_file(self):
        # via file: the MCP configuration's header helper reads the key from the file its variable names (the value
        # is never in mcp.json); via env: the header names the variable, as in 0.1.0
        servers = B.mcp_servers(self.POL)
        self.assertEqual(servers["n"], {"type": "http", "url": "https://n.example/mcp", "headers": {"X-Key": "${NKEY}"}})
        m = servers["m"]
        self.assertNotIn("headers", m)
        self.assertEqual(m["headersHelper"], """read -r v < "$MKEY"; printf '%s%s%s' '{"Authorization": "Bearer ' "$v" '"}'""")

    def test_how_people_read_it(self):
        self.assertEqual(C.binding_desc("credential", {"via": "file", "from": {"file": "~/k"}}),
                         "file ~/k, as a file in /run/pot/secrets that its variable names (the run holds it)")
        self.assertEqual(C.binding_desc("credential", {"via": "env", "from": {"file": "~/k"}}), "env ~/k (the run holds it)")
        self.assertEqual(C.binding_desc("mcp", {"via": "file", "env": "M", "from": {"file": "~/m.env"}}),
                         "file M from ~/m.env, as a file in /run/pot/secrets that M names (the run holds it)")
        self.assertEqual(C.binding_desc("mcp", {"env": "M"}), "env M from the host's environment (the run holds it)")

    def test_a_key_a_header_cannot_carry(self):
        B.make_secrets(self.POL, self.d)
        with self.assertRaisesRegex(B.Stop, 'the mcp binding m: its key holds a quote, a backslash or a control character'):
            B.deliver(self.POL, self.d, "/run/pot/secrets", {"MKEY": 'a"b'})


if __name__ == "__main__":
    unittest.main()
