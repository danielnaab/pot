"""0.3.0: the workspace's pins as data (core/pins.py). pot's core resolves them once, at the run's start, and passes
them to the backend in launch.json's `pins`; bubblewrap and the container backend bind what it lists. The expected
lists below are what bwrap's Pins.compute produced on the same workspaces before the change (pot 0.2.0), so the data
must equal what bubblewrap bound then. Run with the other unit tests (pot-unit)."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

import bwrap as B
import pins as P

PROTECT = ["**/flake.nix", "**/.envrc", "AGENTS.md", "missing.md", ".git/hooks"]
GIT_ENV = {"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1", "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@e",
           "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@e", "GIT_CONFIG_COUNT": "0"}


def git(*a, cwd):
    subprocess.run(["git", *a], cwd=cwd, check=True, capture_output=True, env={**os.environ, **GIT_ENV})


def make(d):
    """A repository like tests/run.sh's sbc: flake.nix at the top and below, a protected AGENTS.md, an .envrc, a
    scion's git directory under .git/modules (its config, no hooks) and its work tree's .git file; and a worktree
    beside it, whose common directory is outside its top."""
    top = os.path.join(d, "ws")
    os.makedirs(top)
    git("init", "-q", "-b", "main", cwd=top)
    for p, t in [("flake.nix", "{}"), ("sub/deep/flake.nix", "{}"), ("AGENTS.md", "a"), ("README", "r"), (".envrc", "x")]:
        os.makedirs(os.path.dirname(os.path.join(top, p)), exist_ok=True)
        with open(os.path.join(top, p), "w") as f:
            f.write(t)
    git("add", "-A", cwd=top)
    git("commit", "-qm", "x", cwd=top)
    m = os.path.join(top, ".git", "modules", "scions", "sbk")
    os.makedirs(os.path.join(m, "refs"))
    with open(os.path.join(m, "HEAD"), "w") as f:
        f.write("ref: refs/heads/main\n")
    with open(os.path.join(m, "config"), "w") as f:
        f.write("[core]\n")
    os.makedirs(os.path.join(top, "scions", "sbk"))
    with open(os.path.join(top, "scions", "sbk", ".git"), "w") as f:
        f.write("gitdir: ../../.git/modules/scions/sbk\n")
    git("worktree", "add", "-q", os.path.join(d, "wt"), "-b", "w", cwd=top)
    return top, os.path.join(d, "wt")


GITDIRS = [".git", ".git/worktrees", ".git/worktrees/wt", ".git/modules", ".git/modules/scions", ".git/modules/scions/sbk"]
GIT_RO = [".git/config", ".git/config.worktree", ".git/hooks", ".git/info", ".git/commondir", ".git/gitdir",
          ".git/worktrees/wt/config.worktree", ".git/worktrees/wt/commondir", ".git/worktrees/wt/gitdir",
          ".git/modules/scions/sbk/config", ".git/modules/scions/sbk/config.worktree", ".git/modules/scions/sbk/hooks",
          ".git/modules/scions/sbk/commondir", ".git/modules/scions/sbk/gitdir"]
GIT_PLACED = [(".git/commondir", ".\n"), (".git/config.worktree", ""), (".git/gitdir", ""), (".git/worktrees/wt/config.worktree", ""),
              (".git/modules/scions/sbk/commondir", ".\n"), (".git/modules/scions/sbk/config.worktree", ""),
              (".git/modules/scions/sbk/gitdir", "")]
# what Pins.compute bound before 0.3.0 (pot 0.2.0's bwrap.py, on these workspaces), relative to the test's folder
TOP = {"dirs": ["ws/" + g for g in GITDIRS] + ["ws/scions", "ws/scions/sbk", "ws/sub", "ws/sub/deep"],
       "ro": ["ws/" + r for r in GIT_RO] + ["ws/scions/sbk/.git", "ws/.envrc", "ws/flake.nix", "ws/sub/deep/flake.nix",
                                             "ws/AGENTS.md"],
       "placed": [("ws/" + p, t) for p, t in GIT_PLACED],
       "dotgits": ["ws/.git", "ws/scions/sbk/.git"], "named": ["ws/.envrc", "ws/flake.nix", "ws/sub/deep/flake.nix"]}
WORKTREE = {"dirs": ["ws/" + g for g in GITDIRS] + ["wt/sub", "wt/sub/deep"],
            "ro": ["ws/" + r for r in GIT_RO] + ["wt/.git", "wt/.envrc", "wt/flake.nix", "wt/sub/deep/flake.nix", "wt/AGENTS.md"],
            "placed": [("ws/" + p, t) for p, t in GIT_PLACED],
            "dotgits": ["wt/.git"], "named": ["wt/.envrc", "wt/flake.nix", "wt/sub/deep/flake.nix"]}
LINK = ("sub/flake.nix is a symbolic link, so the sandbox can't make it read-only (a run must not write what decides "
        "the sandbox or what the host runs): replace the link with what it points to, or remove it, then run again")
HARD = ("AGENTS.md has another hard link, so a run could write it through that link (a run must not write what decides "
        "the sandbox or what the host runs): replace it with a copy of its own (cp -p F F.copy && mv F.copy F), then run "
        "again")


class Resolve(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.d, ignore_errors=True))
        self.top, self.wt = make(self.d)
        self.common = os.path.join(self.top, ".git")
        del P.PLACED[:]

    def rel(self, ps):
        return [p[len(self.d) + 1:] for p in ps]

    def check(self, data, want):
        self.assertEqual(self.rel(data["dirs"]), want["dirs"])
        self.assertEqual(self.rel(r["path"] for r in data["ro"]), want["ro"])
        files = [(p["path"], p["text"]) for p in data["placeholders"] if p["kind"] == "file"]
        self.assertEqual([(p[len(self.d) + 1:], t) for p, t in files], want["placed"])
        self.assertEqual(self.rel(data["dotgits"]), want["dotgits"])
        self.assertEqual(self.rel(data["named"]), want["named"])
        self.assertNotIn("refused", data)

    def test_the_data_is_what_pins_bound_and_nothing_is_written(self):
        before = sorted(os.listdir(self.common))
        data = P.resolve(self.top, self.common, PROTECT, "/dev/null")
        self.check(data, TOP)
        self.assertEqual(sorted(os.listdir(self.common)), before)      # resolving creates no placeholder
        self.assertFalse(os.path.exists(os.path.join(self.common, "modules/scions/sbk/hooks")))
        # the scion's git directory has a config and no hooks: a folder placeholder, before its files
        self.assertEqual([self.rel([p["path"]])[0] for p in data["placeholders"] if p["kind"] == "folder"],
                         ["ws/.git/modules/scions/sbk/hooks"])
        kinds = {self.rel([r["path"]])[0]: r["kind"] for r in data["ro"]}
        self.assertEqual(kinds["ws/.git/hooks"], "folder")
        self.assertEqual(kinds["ws/.git/modules/scions/sbk/hooks"], "folder")   # a placeholder folder
        self.assertEqual(kinds["ws/.git/commondir"], "file")                    # a placeholder file
        self.assertEqual({k for r, k in kinds.items() if k == "folder" and r not in
                          ("ws/.git/hooks", "ws/.git/modules/scions/sbk/hooks", "ws/.git/info")}, set())
        self.assertEqual(data["top"], self.top)
        self.assertEqual(data["common"], self.common)
        json.dumps(data)

    def test_a_worktree_pins_its_common_directory(self):
        self.check(P.resolve(self.wt, self.common, PROTECT, "/dev/null"), WORKTREE)

    def test_compute_still_places_and_binds_the_same(self):
        p = P.Pins(self.top, self.common, PROTECT, {})
        p.compute("/dev/null")
        self.assertEqual(self.rel(p.dirs), TOP["dirs"])
        self.assertEqual(self.rel(p.ro), TOP["ro"])
        self.assertEqual([(f[len(self.d) + 1:], t) for f, t in p.placeholders], TOP["placed"])
        self.assertEqual([(f[len(self.d) + 1:], t) for f, t in P.PLACED], TOP["placed"])
        with open(os.path.join(self.common, "commondir")) as f:
            self.assertEqual(f.read(), ".\n")
        self.assertTrue(os.path.isdir(os.path.join(self.common, "modules/scions/sbk/hooks")))
        self.assertIs(B.Pins, P.Pins)
        self.assertIs(B.Stop, P.Stop)

    def test_place_creates_the_placeholders_once(self):
        data = P.resolve(self.top, self.common, PROTECT, "/dev/null")
        P.place(data)
        self.assertEqual([(f[len(self.d) + 1:], t) for f, t in P.PLACED], TOP["placed"])
        with open(os.path.join(self.common, "modules/scions/sbk/commondir")) as f:
            self.assertEqual(f.read(), ".\n")
        self.assertTrue(os.path.isdir(os.path.join(self.common, "modules/scions/sbk/hooks")))
        # the same data as Pins.compute on the same workspace, and the same before-look fingerprint
        q = P.Pins(self.top, self.common, PROTECT, {})
        q.compute("/dev/null")
        self.assertEqual(q.placeholders, [])
        cfg = {"git": "git"}
        self.assertEqual(P.fingerprint(cfg, P.Pins.from_data(data, PROTECT)), P.fingerprint(cfg, q))

    def test_place_never_writes_what_appeared_since(self):
        data = P.resolve(self.top, self.common, PROTECT, "/dev/null")
        target = os.path.join(self.d, "target")
        os.symlink(target, os.path.join(self.common, "gitdir"))
        with self.assertRaisesRegex(P.Stop, r"^\.git/gitdir appeared after pot resolved the workspace's pins"):
            P.place(data)
        self.assertFalse(os.path.lexists(target))

    def test_a_link_is_refused_and_nothing_placed(self):
        os.symlink("flake.nix", os.path.join(self.top, "sub", "flake.nix"))
        before = sorted(os.listdir(self.common))
        data = P.resolve(self.top, self.common, PROTECT, "/dev/null")
        self.assertEqual(data["refused"], {"why": "link", "path": os.path.join(self.top, "sub/flake.nix"), "message": LINK})
        self.assertEqual((data["dirs"], data["ro"], data["placeholders"]), ([], [], []))
        self.assertEqual(sorted(os.listdir(self.common)), before)
        with self.assertRaisesRegex(P.Stop, "^sub/flake.nix is a symbolic link"):
            P.Pins(self.top, self.common, PROTECT, {}).compute("/dev/null")

    def test_a_link_above_a_protected_path_is_refused(self):
        os.rename(os.path.join(self.top, "sub"), os.path.join(self.top, "real"))
        os.symlink("real", os.path.join(self.top, "sub"))
        data = P.resolve(self.top, self.common, PROTECT + ["sub/x"], "/dev/null")
        self.assertEqual(data["refused"]["why"], "link")
        self.assertEqual(data["refused"]["path"], os.path.join(self.top, "sub"))
        self.assertTrue(data["refused"]["message"].startswith("sub is a symbolic link, so the sandbox can't pin sub/x, below it"))

    def test_a_hard_link_is_refused(self):
        os.link(os.path.join(self.top, "AGENTS.md"), os.path.join(self.top, "agents-link"))
        data = P.resolve(self.top, self.common, PROTECT, "/dev/null")
        self.assertEqual(data["refused"], {"why": "hard-link", "path": os.path.join(self.top, "AGENTS.md"), "message": HARD})
        self.assertFalse(os.path.exists(os.path.join(self.common, "commondir")))

    def test_resolve_workspace_adds_the_sealed_folders(self):
        os.makedirs(os.path.join(self.top, "a", ".pi"))
        with open(os.path.join(self.top, "b.pi"), "w"):
            pass
        w = {"path": self.top, "common": self.common, "protected": PROTECT, "sealed": ["**/.pi"], "clone": None}
        data = P.resolve_workspace("git", w)
        self.assertEqual(data["sealed"], [os.path.join(self.top, "a", ".pi")])
        self.check(data, TOP)
        os.symlink("flake.nix", os.path.join(self.top, "sub", "flake.nix"))
        self.assertEqual(P.resolve_workspace("git", w)["sealed"], [])


class Core(unittest.TestCase):
    """pot's core resolves the pins for an in-place run with write access, and only there."""

    def test_only_in_place_write(self):
        import pot
        d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(d, ignore_errors=True))
        top, _ = make(d)
        w = {"id": "repo:x", "path": top, "common": os.path.join(top, ".git"), "protected": PROTECT, "sealed": [],
             "proposal": [], "clone": None}
        got = pot.workspace_pins("git", {"access": "write", "workspace": [w]})
        self.assertEqual(got, [P.resolve_workspace("git", w)])
        self.assertIsNone(pot.workspace_pins("git", {"access": "read", "workspace": [w]}))
        self.assertIsNone(pot.workspace_pins("git", {"access": "write", "workspace": [dict(w, clone="/c")]}))


class Bwrap(unittest.TestCase):
    """bubblewrap binds the pins launch.json holds (one resolution, pot's core's), and resolves them itself only for a
    launch.json written without them."""

    def prepare(self, pins=None):
        d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(d, ignore_errors=True))
        top, _ = make(os.path.join(d, "w"))
        rd = os.path.join(d, "run")
        os.makedirs(os.path.join(d, "home"))
        os.makedirs(rd)
        w = {"id": "repo:x", "path": top, "common": os.path.join(top, ".git"), "protected": PROTECT, "sealed": [],
             "proposal": [], "clone": None}
        policy = {"version": 1, "run": "abcd", "context": "work", "mode": "in-place", "access": "write",
                  "backend": {"name": "bubblewrap", "requires": [], "proved": []}, "workspace": [w], "items": [],
                  "mounts": {"state": [], "run": "/run/pot"}, "bindings": [], "children": {}, "grants": []}
        launch = {"argv": ["true"], "cwd": top, "home": os.path.join(d, "home"), "user": "u", "uid": 1, "gid": 1,
                  "group": "u", "closure": [], "git": {}, "potBin": None}
        if pins is not None:
            launch["pins"] = pins(w)
        with open(os.path.join(rd, "policy.json"), "w") as f:
            json.dump(policy, f)
        with open(os.path.join(rd, "launch.json"), "w") as f:
            json.dump(launch, f)
        cfg = {"baseClosure": os.devnull, "netClosure": os.devnull, "sh": "/bin/sh", "env": "/usr/bin/env",
               "cacert": "/etc/hosts", "fd": "true", "find": "true", "git": "git", "shell": "/bin/sh",
               "python": "python3", "shim": "/x", "bwrap": "bwrap"}
        del P.PLACED[:]
        B.prepare(cfg, rd)
        with open(os.path.join(rd, "bwrap.args"), "rb") as f:
            return [a.decode() for a in f.read().split(b"\0")], top

    def test_it_binds_launch_jsons_pins(self):
        def pins(w):
            data = P.resolve_workspace("git", w)
            data["ro"].append({"path": os.path.join(w["path"], "README"), "kind": "file"})    # only the data says so
            return [data]
        args, top = self.prepare(pins)
        i = args.index(os.path.join(top, "README"))
        self.assertEqual(args[i - 1:i + 2], ["--ro-bind", os.path.join(top, "README"), os.path.join(top, "README")])
        self.assertTrue(os.path.isfile(os.path.join(top, ".git", "commondir")))      # placed by bwrap's prepare
        self.assertEqual(len(P.PLACED), len(TOP["placed"]))

    def test_without_them_it_resolves_them_itself(self):
        args, top = self.prepare()
        self.assertNotIn(os.path.join(top, "README"), args)
        a = os.path.join(top, "AGENTS.md")
        i = args.index(a)
        self.assertEqual(args[i - 1:i + 2], ["--ro-bind", a, a])

    def test_a_refusal_in_the_data_stops_prepare(self):
        def pins(w):
            return [{"top": w["path"], "common": w["common"], "dirs": [], "ro": [], "placeholders": [], "dotgits": [],
                     "named": [], "sealed": [], "refused": {"why": "link", "path": w["path"] + "/x", "message": "x is a link"}}]
        with self.assertRaisesRegex(B.Stop, "^x is a link$"):
            self.prepare(pins)


if __name__ == "__main__":
    unittest.main()


class Look(unittest.TestCase):
    """The host's look before and after an in-place run, as `python3 -I -S pins.py before|after GIT RUNDIR` (the
    container backend's): bwrap's before.json and the in-place part of bwrap's after event, from launch.json's pins."""

    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.d, ignore_errors=True))
        self.top, _ = make(self.d)
        self.rd = os.path.join(self.d, "run")
        os.makedirs(self.rd)
        with open(os.path.join(self.top, "hid"), "w") as f:
            f.write("tracked")
        git("add", "-A", cwd=self.top)
        git("commit", "-qm", "hid", cwd=self.top)
        self.w = {"id": "repo:x", "path": self.top, "common": os.path.join(self.top, ".git"), "protected": PROTECT,
                  "sealed": [], "proposal": [], "clone": None}
        policy = {"version": 1, "run": "abcd", "context": "work", "mode": "in-place", "access": "write",
                  "backend": {"name": "container", "requires": [], "proved": []}, "workspace": [self.w],
                  "items": [{"kind": "hidden", "value": "x/hid", "class": "limits", "sources": [], "effective": True,
                             "paths": [os.path.join(self.top, "hid")]}],
                  "mounts": {"state": [], "run": "/run/pot"}, "bindings": [], "children": {}, "grants": []}
        self.data = P.resolve_workspace("git", self.w)
        P.place(self.data)
        launch = {"argv": ["true"], "cwd": self.top, "home": "/home/nobody-pot-test", "pins": [self.data],
                  "repoKitPaths": {self.top: ["flake.nix", "flake.lock"]}}
        for n, v in (("policy.json", policy), ("launch.json", launch)):
            with open(os.path.join(self.rd, n), "w") as f:
                json.dump(v, f)

    def look(self, verb):
        here = os.path.dirname(os.path.abspath(P.__file__))
        r = subprocess.run([sys.executable, "-I", "-S", os.path.join(here, "pins.py"), verb, "git", self.rd],
                           capture_output=True, text=True, env={**os.environ, **GIT_ENV})
        self.assertEqual(r.returncode, 0, r.stderr)
        return json.loads(r.stdout)

    def test_before_is_bwraps(self):
        out = self.look("before")
        self.assertEqual(out["notes"], [f"pot: {self.top}/hid is hidden here, but git tracks it: the run sees it empty, and "
                                        "git's history still holds it; don't commit it from this run"])
        with open(os.path.join(self.rd, "before.json")) as f:
            before = json.load(f)
        cfg = {"git": "git"}
        want = P.before_look(cfg, [P.Pins.from_data(self.data, PROTECT)])
        self.assertEqual({k: before[k] for k in ("pins", "hostrun", "refs")}, want)
        self.assertEqual(before["hiddenTracked"], {self.top: ["hid"]})

    def test_after_reports_what_the_run_left(self):
        self.look("before")
        with open(os.path.join(self.top, "flake.nix"), "w") as f:
            f.write("changed")
        os.makedirs(os.path.join(self.top, "new", ".git"))
        with open(os.path.join(self.top, "hid"), "w"):
            pass
        git("commit", "-qam", "blanked", cwd=self.top)
        ev = self.look("after")
        self.assertEqual(len(ev["commits"]), 1)
        self.assertEqual(ev["touches_host_run"], ["flake.nix"])
        self.assertEqual(ev["repo_kit_change"], ["flake.nix"])
        self.assertEqual(ev["new_git_metadata"], ["new/.git"])
        self.assertEqual(ev["hidden_blanked"], ["hid"])
        self.assertEqual(ev["unsigned"], 1)
        self.assertEqual(sorted(ev), ["base", "commits", "hidden_blanked", "new_git_metadata", "repo_kit_change",
                                      "sealed_left", "touches_host_run", "unsigned"])

    def test_after_lists_what_a_sealed_folder_got(self):
        base = os.path.join(self.rd, "sealed", "0")
        os.makedirs(os.path.join(base, "keep"))
        with open(os.path.join(base, "new.json"), "w") as f:
            f.write("{}")
        with open(os.path.join(self.rd, "sealed.json"), "w") as f:
            json.dump([{"dest": self.top + "/.pi", "base": base, "entries": ["keep"], "rel": ".pi"}], f)
        self.look("before")
        self.assertEqual(self.look("after")["sealed_left"], [{"path": ".pi/new.json", "type": "file", "size": 2}])

    def test_it_never_reads_through_a_link(self):
        self.look("before")
        os.rename(os.path.join(self.rd, "before.json"), os.path.join(self.d, "elsewhere.json"))
        os.symlink(os.path.join(self.d, "elsewhere.json"), os.path.join(self.rd, "before.json"))
        here = os.path.dirname(os.path.abspath(P.__file__))
        r = subprocess.run([sys.executable, "-I", "-S", os.path.join(here, "pins.py"), "after", "git", self.rd],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("before.json", r.stderr)
