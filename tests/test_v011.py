"""0.1.1: `access: none` is no workspace at all (compose, explain, why, siblings' narrowing, both backends' start
folder and what they leave unseen). Run with the other unit tests (pot-unit), on any system: the backends' runs
are tests/run.sh's (Seatbelt's on macOS)."""
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


if __name__ == "__main__":
    unittest.main()
