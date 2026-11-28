"""0.4.0: a repo kit's source is the tree the commit will have (the index with the flake directory's tracked changes,
as `git commit -a` makes it; in a commit's hook, the index git gives the hook), so a grant made before committing
holds in the commit's hook; and `pot grant --pending`, every changed source in one prompt and one write. Run with the
other unit tests (pot-unit), on any system."""
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


class Pending(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.d, ignore_errors=True))
        self.host = types.SimpleNamespace(git=GIT or "git", state=os.path.join(self.d, "state"), machine="m",
                                          home=self.d, grants_path=os.path.join(self.d, "grants", "grants.json"))
        Env(self, CLAUDECODE=None, POT=None)

    def run_pending(self, sts, answers, grants=None):
        t = FakeTty(answers)
        saved = []
        real_tty, real_save = pot.tty, pot.save_grants
        pot.tty = lambda: t
        pot.save_grants = lambda host, g: saved.append(json.loads(C.grants_dump(g)))
        try:
            pot.grant_pending(self.host, {}, grants or {"grants": {}, "machines": {}}, sts)
        finally:
            pot.tty, pot.save_grants = real_tty, real_save
        return t.text(), saved

    def test_split(self):
        sts = [st("a", "source"), st("b", "grown"), st("c", "match"), st("d", "never"), st("e", "source")]
        todo, skipped = pot.pending_split(sts)
        self.assertEqual([s["name"] for s in todo], ["a", "e"])
        self.assertEqual([s["name"] for s in skipped], ["b"])

    def test_grants_every_changed_source_in_one_write(self):
        sts = [st("a", "source"), st("b", "source", requests=("capability nix", "network x.example:443")),
               st("c", "grown")]
        sts[2]["added"] = ["network more.example:443"]
        text, saved = self.run_pending(sts, ["y"])
        self.assertEqual(len(saved), 1, "one grants.json write")
        self.assertEqual(sorted(saved[0]["grants"]), ["repo:example.com/o/a", "repo:example.com/o/b"])
        self.assertEqual(saved[0]["grants"]["repo:example.com/o/a"]["source"], {"tree": "b" * 40})
        self.assertEqual(saved[0]["grants"]["repo:example.com/o/b"]["requests"],
                         ["capability nix", "network x.example:443"])
        self.assertIn("grant 2 changed sources? [y/N]", text)
        self.assertIn("skipped c: asks for more (1 more): pot grant c", text)
        self.assertIn("granted a (changed source)", text)
        self.assertIn("granted b (changed source)", text)

    def test_narrower_requests_are_granted_as_asked(self):
        s = st("a", "source", requests=("capability nix",),
               entry={"requests": ["capability nix", "network x.example:443"], "source": {"tree": "a" * 40}})
        old = dict(s["entry"], digest=C.digest(s["entry"]["requests"]), granted="2026-11-01 m")
        _, saved = self.run_pending([s], ["y"], {"grants": {s["kit"]: old}, "machines": {}})
        # a narrower set with a new source: this machine's section, as pot grant KIT writes it
        e = saved[0]["machines"]["m"]["repo:example.com/o/a"]
        self.assertEqual((e["requests"], e["source"]), (["capability nix"], {"tree": "b" * 40}))

    def test_no_is_nothing(self):
        text, saved = self.run_pending([st("a", "source")], ["n"])
        self.assertEqual(saved, [])
        self.assertIn("nothing granted", text)

    def test_nothing_pending(self):
        s = st("c", "grown")
        s["added"] = ["network more.example:443"]
        text, saved = self.run_pending([st("b", "match"), s], [])
        self.assertEqual(saved, [])
        self.assertIn("pot grant --pending: no changed source to grant", text)
        self.assertIn("skipped c: asks for more (1 more): pot grant c", text)

    def test_another_workspace_names_it(self):
        s = st("c", "grown", top=os.path.join(self.d, "src", "c"))
        s["added"] = ["network more.example:443"]
        s["workspace"] = os.path.join(self.d, "src", "c")
        text, _ = self.run_pending([s], [])
        self.assertIn("pot grant c --workspace ~/src/c", text)

    def test_refused_under_an_agent(self):
        os.environ["CLAUDECODE"] = "1"
        with self.assertRaisesRegex(C.Refusal, "pot grant is for people: CLAUDECODE is set"):
            pot.grant(self.host, {"agentMarkers": ["CLAUDECODE"]}, {"pending": True}, [])

    def test_usage(self):
        for flags, kits in (({"pending": True, "diff": True}, []), ({"pending": True, "revoke": ["x"]}, []),
                            ({"pending": True}, ["x"])):
            with self.assertRaises(pot.Usage):
                pot.grant(self.host, {}, flags, kits)

    def test_parse(self):
        _, _, flags, _ = pot.parse(["grant", "--pending", "--workspace", "a", "--workspace", "b", "--commit"], {})
        self.assertEqual((flags["pending"], flags["workspace"], flags["commit"]), (True, ["a", "b"], True))


if __name__ == "__main__":
    unittest.main()
