"""0.6.0: pin moves to signed release tags. With the root's releaseSigners (an allowed_signers file), a kit whose
source changed only by moving pins (flake.lock inputs, flake.nix's release tag literals, gitlinks) to release tags
that a release signer signed keeps its grant; anything else asks, as before, and so does every change with no
releaseSigners. The tags come from each input's URL into pot's state; a fetch that fails asks. Throwaway SSH keys,
made here. Run with the other unit tests (pot-unit), on any system with git and ssh-keygen."""
import copy
import json
import os
import shutil
import subprocess
import tempfile
import types
import unittest
from unittest import mock

import compose as C
import pot
import releases as RL
import test_output as T
from test_v2210 import GIT, ENV, Env, git

KEYGEN = shutil.which("ssh-keygen")


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def node(url, rev, ref, narhash="sha256-x", inputs=None):
    n = {"locked": {"type": "git", "url": url, "rev": rev, "narHash": narhash},
         "original": {"type": "git", "url": url, "ref": ref}}
    if inputs:
        n["inputs"] = inputs
    return n


def lock(nodes, root_inputs):
    return json.dumps({"nodes": {**nodes, "root": {"inputs": root_inputs}}, "root": "root", "version": 7}, indent=2) + "\n"


NIXPKGS = lambda rev: {"locked": {"type": "github", "owner": "NixOS", "repo": "nixpkgs", "rev": rev, "narHash": "sha256-" + rev[:4]},  # noqa: E731
                       "original": {"type": "github", "owner": "NixOS", "repo": "nixpkgs", "ref": "nixos-25.05"}}


class Pure(unittest.TestCase):
    def test_flake_nix_release_literals_alone(self):
        a = b'inputs.t.url = "github:o/t/v0.3.0";\ninputs.p.url = "github:o/p/v1.2.3";\n'
        self.assertTrue(RL.flake_nix_same(a, a.replace(b"v0.3.0", b"v0.4.0").replace(b"v1.2.3", b"v1.10.0")))
        self.assertFalse(RL.flake_nix_same(a, a + b"# a comment\n"))
        self.assertFalse(RL.flake_nix_same(a, a.replace(b"github:o/t", b"github:evil/t")))
        self.assertFalse(RL.flake_nix_same(a, a.replace(b"v0.3.0", b"main")))
        # a version that is not a tag literal (no v) is code, not a pin
        self.assertFalse(RL.flake_nix_same(b'x = "0.3.0";', b'x = "0.4.0";'))

    def test_lock_url(self):
        self.assertEqual(RL.lock_url({"locked": {"type": "github", "owner": "o", "repo": "r", "rev": "a"}}),
                         "https://github.com/o/r.git")
        self.assertEqual(RL.lock_url({"locked": {"type": "github", "owner": "o", "repo": "r", "rev": "a", "host": "g.example"}}),
                         "https://g.example/o/r.git")
        self.assertEqual(RL.lock_url({"locked": {"type": "git", "url": "ssh://h/r.git", "rev": "a"}}), "ssh://h/r.git")
        self.assertIsNone(RL.lock_url({"locked": {"type": "tarball", "url": "https://x/y.tgz"}}))
        self.assertIsNone(RL.lock_url({"locked": {"type": "github", "owner": "o", "repo": "r"}}))
        self.assertIsNone(RL.lock_url("x"))

    def test_lock_changes(self):
        old = lock({"t": node("u", "a" * 40, "v0.1.0"), "nixpkgs": NIXPKGS("1" * 40)}, {"t": "t", "nixpkgs": "nixpkgs"})
        new = lock({"t": node("u", "b" * 40, "v0.2.0"), "nixpkgs": NIXPKGS("1" * 40)}, {"t": "t", "nixpkgs": "nixpkgs"})
        self.assertEqual([n for n, _ in RL.lock_changes(old, new)], ["t"])
        self.assertEqual(RL.lock_changes(old, old), [])
        # the root node, the version, a lock that does not parse: not a change of nodes alone
        self.assertIsNone(RL.lock_changes(old, lock({"t": node("u", "a" * 40, "v0.1.0")}, {"t": "t"})))
        self.assertIsNone(RL.lock_changes(old, new.replace('"version": 7', '"version": 8')))
        self.assertIsNone(RL.lock_changes(old, "{"))
        # a removed node is no new code; a new one is a change
        gone = lock({"t": node("u", "a" * 40, "v0.1.0"), "nixpkgs": NIXPKGS("1" * 40), "x": NIXPKGS("2" * 40)},
                    {"t": "t", "nixpkgs": "nixpkgs"})
        self.assertEqual(RL.lock_changes(gone, old), [])
        self.assertEqual([n for n, _ in RL.lock_changes(old, gone)], ["x"])

    def test_move_text(self):
        self.assertEqual(C.release_move_text({"url": "https://github.com/danielnaab/tuteur.git", "tag": "v0.3.0",
                                              "signer": "daniel@person"}),
                         "release move to tuteur v0.3.0, signed by daniel@person")


@unittest.skipUnless(GIT and KEYGEN, "needs git and ssh-keygen")
class Fixture(unittest.TestCase):
    """A provider repository with release tags: v0.1.0 and v0.2.0 signed by the person key (the release signer),
    v0.3.0 by another key, v0.4.0 lightweight, and its own flake.lock at each."""

    def setUp(self):
        Env(self, GIT_INDEX_FILE=None, GIT_DIR=None, GIT_WORK_TREE=None)
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.d, ignore_errors=True))
        for k in ("person", "other"):
            subprocess.run([KEYGEN, "-q", "-t", "ed25519", "-N", "", "-C", k, "-f", os.path.join(self.d, k)], check=True)
        with open(os.path.join(self.d, "person.pub")) as f:
            pub = " ".join(f.read().split()[:2])
        self.signers = os.path.join(self.d, "release_signers")
        write(self.signers, f'daniel@person namespaces="git" {pub}\n')
        self.prov = os.path.join(self.d, "prov")
        git(self.d, "init", "-q", "prov")
        git(self.prov, "config", "commit.gpgsign", "false")
        self.url = "file://" + self.prov
        self.rev = {}
        for i, (tag, key) in enumerate([("v0.1.0", "person"), ("v0.2.0", "person"), ("v0.3.0", "other"), ("v0.4.0", None)], 1):
            write(os.path.join(self.prov, "flake.lock"), lock({"nixpkgs": NIXPKGS(str(i) * 40)}, {"nixpkgs": "nixpkgs"}))
            git(self.prov, "add", ".")
            git(self.prov, "commit", "-qm", tag)
            self.rev[tag] = git(self.prov, "rev-parse", "HEAD")
            if key:
                git(self.prov, "-c", "gpg.format=ssh", "-c", f"user.signingkey={os.path.join(self.d, key)}",
                    "tag", "-s", tag, "-m", tag)
            else:
                git(self.prov, "tag", tag)
        self.state = os.path.join(self.d, "state")
        os.makedirs(self.state)
        self.releases = RL.Releases(GIT, self.state, KEYGEN, self.signers)


class Signed(Fixture):
    def test_signed_by_a_release_signer(self):
        got = self.releases.signed(self.url, self.rev["v0.2.0"], "v0.2.0", "flake.lock")
        self.assertEqual((got["tag"], got["signer"]), ("v0.2.0", "daniel@person"))
        self.assertIn(b"2222222222", got["lock"])

    def test_the_tag_found_by_its_commit(self):
        self.assertEqual(self.releases.signed(self.url, self.rev["v0.1.0"])["tag"], "v0.1.0")

    def test_another_key(self):
        self.assertIn("not signed by a release signer", self.releases.signed(self.url, self.rev["v0.3.0"], "v0.3.0")["why"])

    def test_lightweight_tag(self):
        self.assertIn("why", self.releases.signed(self.url, self.rev["v0.4.0"], "v0.4.0"))
        self.assertIn("why", self.releases.signed(self.url, self.rev["v0.4.0"]))

    def test_tag_of_another_commit(self):
        self.assertIn("why", self.releases.signed(self.url, self.rev["v0.1.0"], "v0.2.0"))

    def test_not_a_release_tag(self):
        self.assertIn("not a release tag", self.releases.signed(self.url, self.rev["v0.1.0"], "main")["why"])

    def test_unreachable_fails_closed(self):
        got = self.releases.signed("file://" + os.path.join(self.d, "gone"), self.rev["v0.1.0"], "v0.1.0")
        self.assertIn("why", got)
        got = self.releases.signed("file://" + os.path.join(self.d, "gone"), self.rev["v0.1.0"])
        self.assertIn("did not answer", got["why"])

    def test_cached_offline(self):
        self.releases.signed(self.url, self.rev["v0.2.0"], "v0.2.0")
        os.rename(self.prov, self.prov + ".away")
        fresh = RL.Releases(GIT, self.state, KEYGEN, self.signers)
        self.assertEqual(fresh.signed(self.url, self.rev["v0.2.0"], "v0.2.0")["signer"], "daniel@person")
        # the signers are read each time: a key taken out of releaseSigners no longer verifies a cached tag
        write(self.signers, "")
        self.assertIn("why", RL.Releases(GIT, self.state, KEYGEN, self.signers).signed(self.url, self.rev["v0.2.0"], "v0.2.0"))

    def test_another_namespace(self):
        # a signature over the same bytes in another namespace (a file signature) is not a git one
        obj = git(self.prov, "cat-file", "tag", "v0.2.0").encode() + b"\n"
        fields, (payload, _) = RL.split_tag(obj)
        p = os.path.join(self.d, "payload")
        with open(p, "wb") as f:
            f.write(payload)
        subprocess.run([KEYGEN, "-Y", "sign", "-q", "-n", "file", "-f", os.path.join(self.d, "person"), p], check=True,
                       capture_output=True)
        with open(p + ".sig", "rb") as f:
            sig = f.read()
        self.assertIsNone(RL.verify_signature(KEYGEN, self.signers, payload, sig, self.d))
        self.assertEqual(fields["tag"], "v0.2.0")


class RepoKit(Fixture):
    """A consumer whose flake directory ws/ takes the provider as input tool (and nixpkgs as its own), granted at
    v0.1.0."""

    def setUp(self):
        super().setUp()
        self.top = os.path.join(self.d, "r")
        git(self.d, "init", "-q", "r")
        git(self.top, "config", "commit.gpgsign", "false")
        self.flake = 'inputs.tool.url = "git+{url}?ref=v0.1.0";\ninputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";\n'
        write(os.path.join(self.top, "ws", "flake.nix"), self.flake.format(url=self.url))
        self.set_lock("v0.1.0")
        write(os.path.join(self.top, "ws", "other.nix"), "{ }\n")
        os.makedirs(os.path.join(self.top, "ws", "sub"))
        git(self.top, "add", ".")
        git(self.top, "update-index", "--add", "--cacheinfo", f"160000,{self.rev['v0.1.0']},ws/sub")
        git(self.top, "commit", "-qm", "init")
        self.host = types.SimpleNamespace(git=GIT, state=self.state, machine="m", home=self.d,
                                          root={"releaseSigners": self.signers, "sshKeygen": KEYGEN})
        self.old = pot.source_tree(self.host, self.top, "ws")
        self.grants = {"grants": {"repo:x": {"requests": ["capability nix"], "digest": C.digest(["capability nix"]),
                                             "source": {"tree": self.old}}}, "machines": {}}

    def set_lock(self, tag, nixpkgs="9", own=None):
        """flake.lock with tool at TAG, tool's nixpkgs as TAG's own lock has it (or OWN), the root's nixpkgs NIXPKGS."""
        i = {"v0.1.0": "1", "v0.2.0": "2", "v0.3.0": "3", "v0.4.0": "4"}[tag]
        write(os.path.join(self.top, "ws", "flake.lock"),
              lock({"tool": node(self.url, self.rev[tag], tag, inputs={"nixpkgs": "nixpkgs_2"}),
                    "nixpkgs": NIXPKGS(nixpkgs * 40), "nixpkgs_2": NIXPKGS((own or i) * 40)},
                   {"tool": "tool", "nixpkgs": "nixpkgs"}))

    def move(self, tag="v0.2.0", flake_extra="", **kw):
        write(os.path.join(self.top, "ws", "flake.nix"), self.flake.format(url=self.url).replace("v0.1.0", tag) + flake_extra)
        self.set_lock(tag, **kw)

    def w(self, scions=()):
        w = {"id": "repo:x", "top": self.top, "flakeDir": "ws", "repoKit": {"manifest": {}},
             "source": pot.source_tree(self.host, self.top, "ws"), "scions": list(scions)}
        pot.narrowed(self.host, w, self.grants)
        return w

    def test_a_signed_release_move_is_accepted(self):
        self.move()
        w = self.w()
        self.assertEqual(w["sourceAccepts"], [{"tree": self.old}])
        self.assertEqual(w["releaseMoves"], [{"input": "tool", "url": self.url, "commit": self.rev["v0.2.0"], "tag": "v0.2.0",
                                              "signer": "daniel@person"}])

    def test_without_release_signers_it_asks(self):
        self.move()
        self.host.root = {}
        w = self.w()
        self.assertEqual(w["sourceAccepts"], [])
        self.assertNotIn("release", w["sourceWhy"])

    def test_another_lock_move_asks(self):
        self.move(nixpkgs="8")
        w = self.w()
        self.assertEqual(w["sourceAccepts"], [])
        self.assertIn("flake.lock input nixpkgs moved, and is no signed release's", w["sourceWhy"]["release"])

    def test_a_transitive_input_not_the_release_lock_asks(self):
        self.move(own="7")
        self.assertIn("nixpkgs_2", self.w()["sourceWhy"]["release"])

    def test_a_tag_by_another_key_asks(self):
        self.move("v0.3.0")
        w = self.w()
        self.assertEqual(w["sourceAccepts"], [])
        self.assertIn("not signed by a release signer", w["sourceWhy"]["release"])

    def test_a_branch_or_commit_pin_asks(self):
        self.move()
        with open(os.path.join(self.top, "ws", "flake.lock")) as f:
            text = f.read()
        # main is not a release tag, though v0.2.0 names the same commit: a branch pin asks
        write(os.path.join(self.top, "ws", "flake.lock"), text.replace('"ref": "v0.2.0"', '"ref": "main"'))
        self.assertIn("moved, and is no signed release's", self.w()["sourceWhy"]["release"])
        git(self.prov, "commit", "-q", "--allow-empty", "-m", "unreleased")
        write(os.path.join(self.top, "ws", "flake.lock"), text.replace(self.rev["v0.2.0"], git(self.prov, "rev-parse", "HEAD")))
        self.assertIn("flake.lock input tool", self.w()["sourceWhy"]["release"])

    def test_flake_nix_beyond_tags_asks(self):
        self.move(flake_extra="# more\n")
        self.assertIn("flake.nix", self.w()["sourceWhy"]["release"])

    def test_another_file_asks(self):
        self.move()
        write(os.path.join(self.top, "ws", "other.nix"), "{ x = 1; }\n")
        self.assertIn("other.nix", self.w()["sourceWhy"]["release"])

    def test_offline_and_uncached_asks(self):
        self.move()
        os.rename(self.prov, self.prov + ".away")
        self.assertEqual(self.w()["sourceAccepts"], [])

    def test_a_gitlink_release_move(self):
        git(self.top, "update-index", "--cacheinfo", f"160000,{self.rev['v0.2.0']},ws/sub")
        git(self.top, "commit", "-qm", "sub")
        sc = {"path": "ws/sub", "url": self.url, "commit": self.rev["v0.2.0"], "tag": "v0.2.0", "verified": True,
              "identity": "scion:x/sub"}
        w = self.w([sc])
        self.assertEqual([m.get("submodule") for m in w["releaseMoves"]], ["ws/sub"])
        # to an unsigned tag: asks
        git(self.top, "update-index", "--cacheinfo", f"160000,{self.rev['v0.4.0']},ws/sub")
        git(self.top, "commit", "-qm", "sub")
        self.assertEqual(self.w([dict(sc, commit=self.rev["v0.4.0"], tag="v0.4.0")])["sourceAccepts"], [])

    def test_a_submodule_kit(self):
        self.grants["grants"]["scion:x/sub"] = {"requests": ["capability nix"], "digest": C.digest(["capability nix"]),
                                                "source": {"commit": self.rev["v0.1.0"], "tag": "v0.1.0"}}
        sc = {"path": "ws/sub", "url": self.url, "commit": self.rev["v0.2.0"], "tag": "v0.2.0", "verified": True,
              "identity": "scion:x/sub"}
        self.w([sc])
        self.assertEqual(sc["sourceAccepts"], [{"commit": self.rev["v0.1.0"], "tag": "v0.1.0"}])
        self.assertEqual(sc["releaseMoves"][0]["signer"], "daniel@person")
        sc3 = dict(sc, commit=self.rev["v0.3.0"], tag="v0.3.0")
        self.w([sc3])
        self.assertEqual(sc3["sourceAccepts"], [])


class Composed(unittest.TestCase):
    """compose: a source held by release moves is a match with its moves (the run record's start kits and pot
    explain); requests that grow still ask."""

    MOVE = {"input": "graft", "url": "https://github.com/danielnaab/tuteur.git", "commit": "c" * 40, "tag": "v0.3.0",
            "signer": "daniel@person"}

    def report(self, accepts, moves, extra_request=False):
        kit = copy.deepcopy(T.REPO_KIT)
        if extra_request:
            kit["requests"]["network"] = ["more.example"]
        real = C.compose

        def patched(inp):
            inp["workspace"][0].update({"sourceAccepts": accepts, "releaseMoves": moves, "source": "t2"})
            return real(inp)
        with mock.patch.object(T, "REPO_KIT", kit), mock.patch.object(C, "compose", patched):
            g = T.granted("root:claude-code@flexion-bedrock", "root:daniel")
            C.write_grant(g, "repo:platform-vm:2222/daniel/flexion-assistant", ["capability nix"], {"tree": "t"}, "m1", "today", False)
            policy, rep = T.compose("flexion-bedrock", g)
        return policy, rep, next(k for k in rep["kits"] if k["kind"] == "repo")

    def test_accepted(self):
        policy, rep, st = self.report([{"tree": "t"}], [self.MOVE])
        self.assertEqual((st["status"], st["accepted"]), ("match", [self.MOVE]))
        self.assertIn("accepted   release move to tuteur v0.3.0, signed by daniel@person (flexion-assistant)",
                      C.explain_text(policy, rep, T.ROOT))

    def test_not_accepted(self):
        _, _, st = self.report([], [])
        self.assertEqual(st["status"], "source")
        self.assertNotIn("accepted", st)

    def test_grown_still_asks(self):
        _, _, st = self.report([{"tree": "t"}], [self.MOVE], extra_request=True)
        self.assertEqual(st["status"], "grown")
        self.assertNotIn("accepted", st)


if __name__ == "__main__":
    unittest.main()
