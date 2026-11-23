"""S1 placeholders that runs in worktrees of one repository share (pot.claim_placeholders, pot.cleanup_run), at the
function level: interleavings tests/run.sh's barrier can't line up, with bubblewrap's own cleanup and pins' recheck
and place as the backend. Run with the other unit tests (pot-unit); on any system."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import types
import unittest

import bwrap
import pins as P
import pot

GIT = shutil.which("git")
ENV = {"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
       "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}


def git(*args):
    subprocess.run([GIT, *args], check=True, capture_output=True, env={**os.environ, **ENV})


@unittest.skipUnless(GIT, "needs git")
class Shared(unittest.TestCase):
    def setUp(self):
        self.base = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.base, True)
        repo = os.path.join(self.base, "repo")
        git("init", "-q", repo)
        git("-C", repo, "commit", "-q", "--allow-empty", "-m", "x")
        self.w1, self.w2 = os.path.join(self.base, "w1"), os.path.join(self.base, "w2")
        git("-C", repo, "worktree", "add", "-q", self.w1, "-b", "w1")
        git("-C", repo, "worktree", "add", "-q", self.w2, "-b", "w2")
        self.common = os.path.join(repo, ".git")
        self.ph = os.path.join(self.common, "commondir")
        self.host = types.SimpleNamespace(state=os.path.join(self.base, "state"))
        os.makedirs(os.path.join(self.host.state, "runs"))
        cfg = os.path.join(self.base, "cfg.json")
        with open(cfg, "w") as f:
            f.write("{}")
        self.prog = os.path.join(self.base, "backend")
        with open(self.prog, "w") as f:
            f.write(f"#!/bin/sh\nexec {sys.executable} -I {bwrap.__file__} --config {cfg} \"$@\"\n")
        os.chmod(self.prog, 0o755)
        self.fds = []
        self.addCleanup(lambda: [os.close(fd) for fd in self.fds])

    def resolve(self, rid, top):
        """A run's start up to the git directory's lock: its pins resolved, launch.json written."""
        rd = os.path.join(self.host.state, "runs", rid)
        os.makedirs(rd)
        launch = {"pins": [P.resolve(top, self.common, [], "/dev/null")]}
        with open(os.path.join(rd, "launch.json"), "w") as f:
            json.dump(launch, f)
        return rd, launch

    def start(self, rd, launch):
        """The rest of the start, as pot.run does it: under the locks, claim, then the backend's prepare (recheck,
        place, placeholders.json), then note_placed. The run's list stays held (self.fds)."""
        locks = pot.placeholder_locks(self.host, launch)
        try:
            fd = pot.claim_placeholders(self.host, rd, launch)
            self.fds.append(fd)
            d = launch["pins"][0]
            P.recheck(d)
            p = P.place(d)
            with open(os.path.join(rd, "placeholders.json"), "w") as f:
                json.dump(p.placeholders, f)
            pot.note_placed(rd, fd)
            return fd
        finally:
            pot.unlock(locks)

    def end(self, rd, fd):
        pot.cleanup_run(self.host, self.prog, rd)
        self.fds.remove(fd)
        os.close(fd)

    def test_a_start_whose_placeholder_the_last_run_out_removed(self):
        # A runs; B resolves its pins with A's placeholders there (read-only pins, none planned); A ends before B takes
        # the git directory's lock and removes them. B places them again, rather than stopping with "run again"
        a, la = self.resolve("aaaa", self.w1)
        fa = self.start(a, la)
        self.assertTrue(os.path.isfile(self.ph))
        b, lb = self.resolve("bbbb", self.w2)
        self.end(a, fa)
        self.assertFalse(os.path.exists(self.ph))
        fb = self.start(b, lb)
        self.assertTrue(pot.holds_placeholder(self.ph, ".\n"))
        self.assertIn([self.ph, ".\n"], pot.read_list(os.path.join(b, pot.PINS_USED)))
        self.end(b, fb)
        self.assertFalse(os.path.exists(self.ph))

    def test_a_placeholder_listed_only_in_placeholders_json(self):
        # a run started by pot v0.2.0 (placeholders.json, no pins-used.json), lost: a start takes its placeholder on,
        # so the lost run's cleanup (tidy_lost) leaves it while the start still runs with it bound
        a, la = self.resolve("aaaa", self.w1)
        P.place(la["pins"][0])
        with open(os.path.join(a, "placeholders.json"), "w") as f:
            json.dump([[ph["path"], ph["text"]] for ph in la["pins"][0]["placeholders"] if ph["kind"] == "file"], f)
        b, lb = self.resolve("bbbb", self.w2)
        fb = self.start(b, lb)
        pot.cleanup_run(self.host, self.prog, a)
        self.assertTrue(os.path.isfile(self.ph), "a lost run's cleanup removed a placeholder a live run has bound")
        self.end(b, fb)
        self.assertFalse(os.path.exists(self.ph))


    def test_a_submodule_and_its_worktree_share_a_lock(self):
        # a run in a super repository pins (and places placeholders in) its submodule's git directory under
        # .git/modules; a run in a worktree of the submodule has that directory as its common one. They place the same
        # placeholders, so they take the same lock
        sup = os.path.join(self.base, "sup")
        git("init", "-q", sup)
        git("-C", sup, "-c", "protocol.file.allow=always", "submodule", "add", "-q", os.path.join(self.base, "repo"),
            "sub")
        git("-C", sup, "commit", "-q", "-m", "s")
        mod = os.path.join(sup, ".git", "modules", "sub")
        wt = os.path.join(self.base, "subwt")
        git("-C", os.path.join(sup, "sub"), "worktree", "add", "-q", wt, "-b", "x")
        l1 = {"pins": [P.resolve(sup, os.path.join(sup, ".git"), [], "/dev/null")]}
        l2 = {"pins": [P.resolve(wt, mod, [], "/dev/null")]}
        both = {ph["path"] for ph in l1["pins"][0]["placeholders"]} & {ph["path"] for ph in l2["pins"][0]["placeholders"]}
        self.assertTrue(both)

        def inodes(launch):
            fds = pot.placeholder_locks(self.host, launch)
            try:
                return {os.fstat(fd).st_ino for fd in fds}
            finally:
                pot.unlock(fds)
        self.assertTrue(inodes(l1) & inodes(l2), "two runs that place the same placeholders take no lock in common")

    def test_a_lost_cleanup_rechecks_under_the_lock(self):
        # tidy_lost decides a run is lost before it takes the git directory's lock; a start that had created its list
        # but not yet locked it looks lost. Under the lock the run is live again: its cleanup does nothing
        a, la = self.resolve("aaaa", self.w1)
        fa = self.start(a, la)
        pot.cleanup_run(self.host, self.prog, a, lost=True)
        self.assertTrue(os.path.isfile(self.ph))
        self.assertTrue(os.path.exists(os.path.join(a, pot.PINS_USED)))
        self.end(a, fa)
        self.assertFalse(os.path.exists(self.ph))

if __name__ == "__main__":
    unittest.main()
