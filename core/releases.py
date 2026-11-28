"""0.6.0: pin moves to signed release tags (the root's releaseSigners).

A changed source whose only changes move pins to release tags that a signer the root names signed needs no new
grant: the decision was made at the tag. This module holds the pieces that need no host facts beyond git and
ssh-keygen: what a flake.lock change moves, whether a flake.nix change is release tags alone, and whether a tag
object at a URL is signed by a release signer (fetched into pot's state, never into a workspace). Every failure is
an answer of None, so the caller asks for a grant as before: it fails closed.
"""
import fcntl
import hashlib
import json
import os
import re
import subprocess
import tempfile

RELEASE = re.compile(r"v?[0-9]+\.[0-9]+\.[0-9]+")
TAG_LITERAL = re.compile(rb"(?<![0-9A-Za-z.])v[0-9]+\.[0-9]+\.[0-9]+(?![0-9A-Za-z])")
SIG_START = b"-----BEGIN SSH SIGNATURE-----"
FETCH_TIMEOUT = 60
# a commit hook's git variables (pot runs in one) never reach the cache's git
GIT_VARS = ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_COMMON_DIR", "GIT_NAMESPACE")


def git_env(**extra):
    return {**{k: v for k, v in os.environ.items() if k not in GIT_VARS}, "GIT_TERMINAL_PROMPT": "0", **extra}


def flake_nix_same(old, new):
    """Whether two flake.nix texts (bytes) differ in release tag literals (vX.Y.Z) alone: an input's URL moved from
    one release to another, as `github:o/r/v0.3.0` to `github:o/r/v0.4.0`, and nothing else."""
    return TAG_LITERAL.sub(b"vX.Y.Z", old) == TAG_LITERAL.sub(b"vX.Y.Z", new)


def lock_url(node):
    """The git URL an input's locked node was fetched from: github (with its host) and git only; else None."""
    lk = node.get("locked") if isinstance(node, dict) else None
    if not isinstance(lk, dict) or not isinstance(lk.get("rev"), str):
        return None
    if lk.get("type") == "github" and isinstance(lk.get("owner"), str) and isinstance(lk.get("repo"), str):
        return f"https://{lk.get('host') or 'github.com'}/{lk['owner']}/{lk['repo']}.git"
    if lk.get("type") == "git" and isinstance(lk.get("url"), str):
        return lk["url"]
    return None


def lock_changes(old, new):
    """The nodes a flake.lock change adds or changes, as [(name, node)], or None when the change is not one of nodes
    alone (a lock that does not parse, another version or root, a changed root node). Removed nodes are no new code."""
    try:
        a, b = json.loads(old), json.loads(new)
    except (ValueError, UnicodeDecodeError):
        return None
    if not (isinstance(a, dict) and isinstance(b, dict) and isinstance(a.get("nodes"), dict) and isinstance(b.get("nodes"), dict)):
        return None
    if {k: v for k, v in a.items() if k != "nodes"} != {k: v for k, v in b.items() if k != "nodes"}:
        return None
    root = b.get("root", "root")
    if a["nodes"].get(root) != b["nodes"].get(root):
        return None
    out = [(n, v) for n, v in sorted(b["nodes"].items()) if n != root and a["nodes"].get(n) != v]
    if not all(isinstance(v, dict) and isinstance(v.get("original", {}), dict) for _, v in out):
        return None
    return out


def locked_nodes(text):
    """A lock's locked attribute sets, as canonical JSON: what a signed release's own flake.lock names."""
    try:
        d = json.loads(text)
        return {json.dumps(v["locked"], sort_keys=True) for v in d["nodes"].values()
                if isinstance(v, dict) and isinstance(v.get("locked"), dict)}
    except (ValueError, KeyError, TypeError, AttributeError):
        return set()


def split_tag(obj):
    """A tag object's header fields and (payload, signature), or None when it holds no SSH signature."""
    head, _, _ = obj.partition(b"\n\n")
    fields = {}
    for line in head.split(b"\n"):
        k, _, v = line.partition(b" ")
        fields.setdefault(k.decode("ascii", "replace"), v.decode("utf-8", "replace"))
    i = obj.rfind(b"\n" + SIG_START)
    if i < 0:
        return fields, None
    return fields, (obj[:i + 1], obj[i + 1:])


def verify_signature(keygen, signers, payload, sig, scratch):
    """The principal of RELEASE SIGNERS (an allowed_signers file) whose key made SIG over PAYLOAD in the git
    namespace (ssh-keygen -Y find-principals, then -Y verify), else None."""
    fd, sf = tempfile.mkstemp(dir=scratch, prefix=".sig-")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(sig)
        r = subprocess.run([keygen, "-Y", "find-principals", "-f", signers, "-s", sf], capture_output=True, text=True)
        if r.returncode != 0:
            return None
        for p in [l.strip() for l in r.stdout.split("\n") if l.strip()]:
            v = subprocess.run([keygen, "-Y", "verify", "-f", signers, "-I", p, "-n", "git", "-s", sf],
                               input=payload, capture_output=True)
            if v.returncode == 0:
                return p
        return None
    finally:
        os.unlink(sf)


class Releases:
    """Tags fetched from release URLs, kept in pot's state (one bare repository per URL, under a lock), and their
    signatures checked against the root's releaseSigners."""

    def __init__(self, git, state, keygen, signers, timeout=FETCH_TIMEOUT):
        self.git, self.keygen, self.signers, self.timeout = git, keygen, signers, timeout
        self.dir = os.path.join(state, "cache", "releases")
        self.memo = {}

    def _g(self, repo, *args, timeout=None):
        return subprocess.run([self.git, "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null", "-C", repo] + list(args),
                              capture_output=True, timeout=timeout, cwd="/",
                              env=git_env(GIT_DIR=repo))

    def _repo(self, url):
        d = os.path.join(self.dir, hashlib.sha256(url.encode()).hexdigest()[:16] + ".git")
        if not os.path.isdir(d):
            os.makedirs(self.dir, exist_ok=True)
            subprocess.run([self.git, "init", "-q", "--bare", d], capture_output=True, cwd="/", env=git_env())
        return d

    def _local_tag(self, repo, rev, tag):
        """A tag object in the cache naming REV: TAG's, or with TAG None any release tag's."""
        r = self._g(repo, "for-each-ref", "--format=%(refname:strip=2) %(objecttype) %(*objectname)", "refs/tags")
        for line in r.stdout.decode(errors="replace").split("\n"):
            parts = line.split()
            if len(parts) == 3 and parts[1] == "tag" and parts[2] == rev and (parts[0] == tag if tag else RELEASE.fullmatch(parts[0])):
                return parts[0]
        return None

    def _remote_tags(self, url, rev):
        try:
            r = subprocess.run([self.git, "ls-remote", "--tags", url], capture_output=True, text=True, timeout=self.timeout,
                               cwd="/", env=git_env())
        except subprocess.TimeoutExpired:
            return None
        if r.returncode != 0:
            return None
        return sorted({ref[len("refs/tags/"):].removesuffix("^{}") for c, _, ref in (l.partition("\t") for l in r.stdout.split("\n"))
                       if c == rev and ref.endswith("^{}") and RELEASE.fullmatch(ref[len("refs/tags/"):].removesuffix("^{}"))})

    def _fetch(self, repo, url, tag):
        try:
            r = self._g(repo, "fetch", "-q", "--no-tags", "--no-write-fetch-head", "--depth=1", url,
                        f"+refs/tags/{tag}:refs/tags/{tag}", timeout=self.timeout)
        except subprocess.TimeoutExpired:
            return False
        return r.returncode == 0

    def signed(self, url, rev, tag=None, lock_path=None):
        """REV as a release tag at URL that a release signer signed: {"tag", "signer", "lock"} (the tagged commit's
        LOCK_PATH, when asked and present), or {"why": "..."}. TAG None: any release tag naming REV. Fetches only
        what the cache lacks; a fetch that fails (offline, the URL gone) is a "why", so the caller asks."""
        key = (url, rev, tag, lock_path)
        if key not in self.memo:
            self.memo[key] = self._signed(url, rev, tag, lock_path)
        return self.memo[key]

    def _signed(self, url, rev, tag, lock_path):
        if tag is not None and not RELEASE.fullmatch(tag):
            return {"why": f"{tag} is not a release tag"}
        repo = self._repo(url)
        with open(repo + ".lock", "a") as lk:
            fcntl.flock(lk, fcntl.LOCK_EX)
            found = self._local_tag(repo, rev, tag)
            if found is None:
                names = [tag] if tag else self._remote_tags(url, rev)
                if names is None:
                    return {"why": f"{url} did not answer"}
                for t in names:
                    if self._fetch(repo, url, t) and self._local_tag(repo, rev, t):
                        found = t
                        break
                if found is None:
                    return {"why": f"no annotated release tag at {url} names {rev[:12]}" if names else
                            f"no release tag at {url} names {rev[:12]}"}
            obj = self._g(repo, "cat-file", "tag", f"refs/tags/{found}").stdout
            fields, signed = split_tag(obj)
            if fields.get("object") != rev or fields.get("type") != "commit" or fields.get("tag") != found:
                return {"why": f"{found} at {url}: its tag object does not name {rev[:12]}"}
            if signed is None:
                return {"why": f"{found} at {url} has no SSH signature"}
            who = verify_signature(self.keygen, self.signers, signed[0], signed[1], self.dir)
            if who is None:
                return {"why": f"{found} at {url} is not signed by a release signer"}
            out = {"tag": found, "signer": who, "lock": None}
            if lock_path:
                b = self._g(repo, "cat-file", "blob", f"{rev}:{lock_path}")
                out["lock"] = b.stdout if b.returncode == 0 else None
            return out
