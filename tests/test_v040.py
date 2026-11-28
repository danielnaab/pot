"""0.4.0: a repo kit's source is the tree the commit will have (the index with the flake directory's tracked changes,
as `git commit -a` makes it; in a commit's hook, the index git gives the hook), so a grant made before committing
holds in the commit's hook. 0.5.0: plain `pot grant` grants the updated kits on one answer, then asks for the rest,
in one write; a refusal names the command with its workspace. Run with the other unit tests (pot-unit), on any
system."""
import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import types
import unittest

import compose as C
import pot
from test_v2210 import GIT, Env, git

HERE = os.path.dirname(os.path.abspath(pot.__file__))


def read(path, mode="r"):
    with open(path, mode) as f:
        return f.read()


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


@unittest.skipUnless(GIT, "needs git")
class SourceTree(unittest.TestCase):
    """A scratch repository with its flake directory at ws/ (as potting-shed's nix/workspace)."""

    def setUp(self):
        Env(self, GIT_INDEX_FILE=None, GIT_DIR=None, GIT_WORK_TREE=None)
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.d, ignore_errors=True))
        self.top = os.path.join(self.d, "r")
        self.state = os.path.join(self.d, "state")
        os.makedirs(self.state)
        self.host = types.SimpleNamespace(git=GIT, state=self.state)
        git(self.d, "init", "-q", "r")
        git(self.top, "config", "commit.gpgsign", "false")
        write(os.path.join(self.top, "ws", "flake.lock"), "a\n")
        write(os.path.join(self.top, "ws", "flake.nix"), "{ }\n")
        write(os.path.join(self.top, "notes.md"), "n\n")
        git(self.top, "add", ".")
        git(self.top, "commit", "-qm", "init")

    def source(self):
        return pot.source_tree(self.host, self.top, "ws")

    def head(self):
        return git(self.top, "rev-parse", "HEAD:ws")

    def hook(self):
        """A pre-commit hook that records the source pot computes there, as the hook command's start does."""
        out = os.path.join(self.d, "hook-tree")
        code = (f"import sys, types; sys.path.insert(0, {HERE!r}); import pot; "
                f"open({out!r}, 'w').write(pot.source_tree(types.SimpleNamespace(git={GIT!r}, state={self.state!r}), "
                f"{self.top!r}, 'ws'))")
        write(os.path.join(self.top, ".git", "hooks", "pre-commit"),
              f"#!/bin/sh\nexec {sys.executable} -I -c \"{code}\"\n")
        os.chmod(os.path.join(self.top, ".git", "hooks", "pre-commit"), 0o755)
        return out

    def test_clean(self):
        self.assertEqual(self.source(), self.head())

    def test_unstaged_change_is_in_the_source(self):
        # the 2026-11-27 case: a pin moved (flake.lock changed), not staged; granted, then git commit -a
        objs = git(self.top, "count-objects", "-v")
        index = read(os.path.join(self.top, ".git", "index"), "rb")
        write(os.path.join(self.top, "ws", "flake.lock"), "b\n")
        granted = self.source()
        self.assertNotEqual(granted, self.head())
        self.assertEqual(git(self.top, "count-objects", "-v"), objs, "no object written into the workspace")
        self.assertEqual(read(os.path.join(self.top, ".git", "index"), "rb"), index, "the index is untouched")
        out = self.hook()
        git(self.top, "commit", "-qam", "pins")
        self.assertEqual(read(out), granted, "the hook sees the tree the grant recorded")
        self.assertEqual(self.head(), granted)

    def test_commit_path(self):
        write(os.path.join(self.top, "ws", "flake.lock"), "b\n")
        write(os.path.join(self.top, "notes.md"), "changed, not committed\n")
        granted = self.source()
        out = self.hook()
        git(self.top, "commit", "-qm", "pins", "--", "ws/flake.lock")
        self.assertEqual(read(out), granted)
        self.assertEqual(self.head(), granted)

    def test_staged_then_plain_commit(self):
        write(os.path.join(self.top, "ws", "flake.lock"), "b\n")
        git(self.top, "add", "ws/flake.lock")
        granted = self.source()
        out = self.hook()
        git(self.top, "commit", "-qm", "pins")
        self.assertEqual(read(out), granted)

    def test_a_deleted_file(self):
        os.remove(os.path.join(self.top, "ws", "flake.nix"))
        granted = self.source()
        git(self.top, "commit", "-qam", "gone")
        self.assertEqual(self.head(), granted)

    def test_untracked_files_are_not_the_source(self):
        write(os.path.join(self.top, "ws", "new.nix"), "x\n")
        self.assertEqual(self.source(), self.head())

    def test_the_hook_checks_the_commit_not_the_working_tree(self):
        # partly staged: the commit (and so its hook) holds the staged flake.lock, not the working tree's
        write(os.path.join(self.top, "ws", "flake.lock"), "b\n")
        git(self.top, "add", "ws/flake.lock")
        write(os.path.join(self.top, "ws", "flake.lock"), "c\n")
        out = self.hook()
        git(self.top, "commit", "-qm", "staged part")
        self.assertEqual(read(out), self.head())

    def test_another_repositorys_index_is_not_this_ones(self):
        # GIT_INDEX_FILE from a hook in repository A: B's source is B's own (a --repo workspace)
        other = os.path.join(self.d, "a")
        git(self.d, "init", "-q", "a")
        write(os.path.join(self.top, "ws", "flake.lock"), "b\n")
        want = self.source()
        os.environ["GIT_INDEX_FILE"] = os.path.join(other, ".git", "index")
        self.assertEqual(self.source(), want)

    def test_a_worktree(self):
        wt = os.path.join(self.d, "wt")
        git(self.top, "worktree", "add", "-q", wt, "-b", "topic")
        write(os.path.join(wt, "ws", "flake.lock"), "b\n")
        granted = pot.source_tree(self.host, wt, "ws")
        git(wt, "commit", "-qam", "pins")
        self.assertEqual(git(wt, "rev-parse", "HEAD:ws"), granted)
        self.assertEqual(self.source(), self.head(), "the main checkout's source is its own")


def st(name, status, requests=("capability nix",), entry=None, kind="repo", top=None):
    e = entry if entry is not None else {"requests": list(requests), "source": {"tree": "a" * 40}}
    return {"kit": f"repo:example.com/o/{name}", "name": name, "kind": kind, "status": status,
            "requests": sorted(requests), "powerful": [r for r in requests if r.startswith("capability")],
            "entry": e, "source": {"tree": "b" * 40}, "was": e.get("source"), "now": {"tree": "b" * 40},
            "added": [], "top": top}


class FakeTty:
    def __init__(self, answers):
        self.answers = list(answers)
        self.out = []

    def write(self, s):
        self.out.append(s)

    def flush(self):
        pass

    def readline(self):
        return self.answers.pop(0) + "\n" if self.answers else ""

    def close(self):
        pass

    def text(self):
        return "".join(self.out)


class Grouped(unittest.TestCase):
    """0.5.0: plain pot grant grants the updated kits (a changed source, asking for no more) together on one answer,
    then prompts the rest one by one, in one write of the grants file; --pending is gone."""

    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.d, ignore_errors=True))
        self.host = types.SimpleNamespace(git=GIT or "git", state=os.path.join(self.d, "state"), machine="m",
                                          home=self.d, grants_path=os.path.join(self.d, "grants", "grants.json"))
        Env(self, CLAUDECODE=None, POT=None)
        self.saved, self.commits = [], []
        self.t = None
        real = (pot.tty, pot.save_grants, pot.commit_grants)

        def undo():
            pot.tty, pot.save_grants, pot.commit_grants = real
        self.addCleanup(undo)
        pot.tty = lambda: self.t
        pot.save_grants = lambda host, g: self.saved.append(json.loads(C.grants_dump(g)))
        pot.commit_grants = lambda host, names, verb="pot grant": self.commits.append(list(names)) or "committed"

    def flow(self, sts, answers, grants=None, commit=False):
        self.t = FakeTty(answers)
        batch, rest = pot.batch_split(sts)
        pot.grant_flow(self.host, {}, grants or {"grants": {}, "machines": {}}, rest, commit=commit, batch=batch)
        return self.t.text()

    def grown(self, name):
        return st(name, "grown", requests=("capability nix", "network more.example:443"),
                  entry={"requests": ["capability nix"], "source": {"tree": "a" * 40}})

    def test_split(self):
        sts = [st("a", "source"), st("b", "grown"), st("d", "never"), st("e", "source"), st("f", "unverified")]
        batch, rest = pot.batch_split(sts)
        self.assertEqual([s["name"] for s in batch], ["a", "e"])
        self.assertEqual([s["name"] for s in rest], ["b", "d", "f"])

    def test_one_answer_then_one_by_one_in_one_write(self):
        sts = [st("a", "source"), self.grown("c"),
               st("b", "source", requests=("capability nix", "network x.example:443"))]
        text = self.flow(sts, ["y", "y"], commit=True)
        self.assertEqual(len(self.saved), 1, "one grants.json write")
        self.assertEqual(sorted(self.saved[0]["grants"]),
                         ["repo:example.com/o/a", "repo:example.com/o/b", "repo:example.com/o/c"])
        self.assertEqual(self.saved[0]["grants"]["repo:example.com/o/a"]["source"], {"tree": "b" * 40})
        self.assertEqual(self.saved[0]["grants"]["repo:example.com/o/c"]["requests"],
                         ["capability nix", "network more.example:443"])
        self.assertEqual(self.commits, [["a", "b", "c"]], "one commit, naming every kit granted")
        # the batch first, its one question, then the kit that asks for more, alone
        q = text.index("grant 2 updated kits? [y/N]")
        self.assertLess(text.index("a   repo kit (repo:example.com/o/a)"), q)
        self.assertLess(text.index("b   repo kit (repo:example.com/o/b)"), q)
        self.assertIn("  powerful: capability nix", text[:q])
        self.assertLess(q, text.index("granted a (changed source)"))
        self.assertLess(q, text.index("c   repo kit (repo:example.com/o/c), for every run in this repository   asks for more"))
        self.assertIn("    + network more.example:443", text[q:])
        self.assertIn("  grant these? [y/N]", text[q:])

    def test_one_updated_kit(self):
        text = self.flow([st("a", "source")], ["y"])
        self.assertIn("grant 1 updated kit? [y/N]", text)
        self.assertEqual(len(self.saved), 1)

    def test_narrower_requests_are_granted_as_asked(self):
        s = st("a", "source", requests=("capability nix",),
               entry={"requests": ["capability nix", "network x.example:443"], "source": {"tree": "a" * 40}})
        old = dict(s["entry"], digest=C.digest(s["entry"]["requests"]), granted="2026-11-01 m")
        text = self.flow([s], ["y"], {"grants": {s["kit"]: old}, "machines": {}})
        self.assertIn("  no longer asked: - network x.example:443", text)
        # a narrower set with a new source: this machine's section, as pot grant KIT writes it
        e = self.saved[0]["machines"]["m"]["repo:example.com/o/a"]
        self.assertEqual((e["requests"], e["source"]), (["capability nix"], {"tree": "b" * 40}))

    def test_no_to_the_batch_still_asks_the_rest(self):
        text = self.flow([st("a", "source"), self.grown("c")], ["n", "y"])
        self.assertIn("not granted: a", text)
        self.assertEqual(len(self.saved), 1)
        self.assertEqual(sorted(self.saved[0]["grants"]), ["repo:example.com/o/c"])

    def test_no_to_everything_writes_nothing(self):
        self.flow([st("a", "source"), st("n", "never", entry={"requests": []})], ["n", "n"], commit=True)
        self.assertEqual((self.saved, self.commits), ([], []))

    def test_the_stopped_run_and_the_named_workspace_are_shown(self):
        a, b = st("a", "source"), st("b", "source")
        a["stoppedRun"], a["stoppedIn"] = "8b3f", os.path.join(self.d, "src", "a")
        b["workspace"] = os.path.join(self.d, "src", "b")
        text = self.flow([a, b], ["n"])
        self.assertIn("changed source   (stopped run 8b3f, in ~/src/a)", text)
        self.assertIn("changed source   (in ~/src/b)", text)

    def run_here(self, here, stopped, answers):
        self.t = FakeTty(answers)
        real = (pot.workspace_status, pot.stopped_kits)
        pot.workspace_status = lambda host, root, context, flags: here
        pot.stopped_kits = lambda host, root, context, flags: stopped
        buf = io.StringIO()
        try:
            with contextlib.redirect_stdout(buf):
                pot.grant_here(self.host, {}, {"grants": {}, "machines": {}}, "work", {})
        finally:
            pot.workspace_status, pot.stopped_kits = real
        out = buf.getvalue().splitlines()
        return self.t.text(), out

    def test_plain_grant_groups_and_asks_a_stopped_runs_new_kit_alone(self):
        never = st("n", "never", entry=None)
        never["entry"], never["stoppedRun"] = None, "9427"
        text, out = self.run_here([st("a", "source"), self.grown("c"), st("x", "never", entry=None),
                                   st("m", "match")], [never], ["y", "y", "y"])
        q = text.index("grant 1 updated kit? [y/N]")
        self.assertLess(q, text.index("n   repo kit (repo:example.com/o/n)"))
        self.assertLess(text.index("n   repo kit"), text.index("c   repo kit"), "a stopped run's kit first")
        self.assertEqual(len(self.saved), 1)
        self.assertEqual(sorted(self.saved[0]["grants"]),
                         ["repo:example.com/o/a", "repo:example.com/o/c", "repo:example.com/o/n"])
        self.assertEqual(out, ["pot: also not granted: x (pot grant NAME)"])

    def test_nothing_to_grant(self):
        text, out = self.run_here([st("m", "match")], [], [])
        self.assertEqual(out, ["pot: nothing to grant"])
        self.assertEqual(self.saved, [])

    def test_refused_under_an_agent(self):
        os.environ["CLAUDECODE"] = "1"
        with self.assertRaisesRegex(C.Refusal, "pot grant is for people: CLAUDECODE is set"):
            pot.grant(self.host, {"agentMarkers": ["CLAUDECODE"]}, {}, [])

    def test_pending_is_gone(self):
        with self.assertRaisesRegex(pot.Usage, "unknown option --pending"):
            pot.parse(["grant", "--pending"], {})
        _, _, flags, _ = pot.parse(["grant", "--workspace", "a", "--workspace", "b", "--commit"], {})
        self.assertEqual((flags["workspace"], flags["commit"]), (["a", "b"], True))


class RefusalCommand(unittest.TestCase):
    """0.5.0: a refusal for a changed source (or a kit that asks for more) names the command, with the workspace."""

    def test_changed_source(self):
        text = C.stale_message("work", st("a", "source"), "~/src/potting-shed")
        self.assertIn("a (repo:example.com/o/a) changed source since its grant:", text)
        self.assertTrue(text.endswith("\n  grant it: cd ~/src/potting-shed && pot grant"), text)

    def test_asks_for_more(self):
        s = st("a", "grown")
        s["added"] = ["network more.example:443"]
        self.assertTrue(C.stale_message("work", s, "/srv/a b").endswith("  grant it: cd '/srv/a b' && pot grant"))

    def test_never_granted_and_no_workspace(self):
        self.assertTrue(C.stale_message("work", st("a", "never"), "~/x").endswith("  grant it: pot grant a"))
        self.assertTrue(C.stale_message("work", st("a", "source")).endswith("  grant it: pot grant a"))

    def test_shell_path(self):
        self.assertEqual([C.shell_path(p) for p in ("~", "~/src/pot", "~/my src/p", "/tmp/x")],
                         ["~", "~/src/pot", "~/'my src/p'", "/tmp/x"])

    def test_grant_workspace(self):
        ws = [{"id": "repo:a", "top": "/w/a"}, {"id": "repo:b", "top": "/w/b"}]
        self.assertEqual(pot.grant_workspace(ws, {"kit": "repo:b"}), "/w/b")
        self.assertEqual(pot.grant_workspace(ws, {"kit": "root:daniel"}), "/w/a")


if __name__ == "__main__":
    unittest.main()
