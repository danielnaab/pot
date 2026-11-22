"""pot: the workspace's pins, and the host's look at an in-place run (0.3.0).

pot's core resolves each workspace's pins once, at the run's start, and passes them to the backend as data
(launch.json's `pins`, contracts/launch-1.md): the folders bound writable onto themselves (mount points: not renamed,
removed or replaced), the paths bound read-only onto themselves with their kinds, the placeholders the backend
creates first, the sealed folders, and a refusal (a link on a protected path, a protected file with a second hard
link) that stops the run. bubblewrap (bwrap.py) and Seatbelt import this module; the container backend, built apart,
binds what the data lists and runs this file for the host's look before and after a run:

    python3 -I -S pins.py before|after GIT RUNDIR

`before` writes RUNDIR/before.json (what the after event compares: bwrap's) and prints {"notes": [...]}; `after`
prints the in-place part of the after event (bwrap's). Both read RUNDIR/policy.json and launch.json, never through
a link. The standard library only.
"""
import hashlib
import json
import os
import stat
import subprocess
import sys

GITFILES = ["config", "config.worktree", "hooks", "info", "commondir", "gitdir"]
# S1: names host git reads in a git directory that a run could create when they are absent (commondir would point
# host git at a run-written config). pot puts a neutral placeholder at each absent one, read-only in the run, and
# removes it after: commondir "." (the directory itself), the others empty (host git verified to work with each).
PLACEHOLDERS = {"commondir": ".\n", "config.worktree": "", "gitdir": ""}
PLACED = []   # every placeholder this backend made, written to <run dir>/placeholders.json even when prepare stops
# §9: changed host-run files the after event reports, beyond the protect list
HOST_RUN_EXTRA = [".gitmodules", ".lfsconfig", ".gitattributes", ".github/workflows", ".forgejo/workflows",
                  ".gitlab-ci.yml"]
EMPTY_BLOB = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"     # git's empty blob (SHA-1)
SEAL_MAX = 256    # entries a sealed folder takes (3 bwrap arguments each; bwrap's limit is 9000 in all)


class Stop(Exception):
    pass


class Refused(Stop):
    """A pin the sandbox can't make: WHY is `link` or `hard-link`, PATH the path at fault."""

    def __init__(self, message, why, path):
        super().__init__(message)
        self.why, self.path = why, path


def visible(s, lines=False):
    """Run-chosen text, safe for the host's terminal: C0 and C1 controls as \\xNN, Unicode format characters (Cf:
    bidi overrides and isolates, zero-width ones) as \\uNNNN (R8), a byte that is not UTF-8 as \\xNN (G2). LINES keeps
    a newline (a message's own lines).
    pot.py and pins.py hold the same function (pins.py's, bwrap's, never keeps lines)."""
    import unicodedata
    out = []
    for c in s:
        o = ord(c)
        if c == "\n" and lines:
            out.append(c)
        elif o < 0x20 or 0x7f <= o <= 0x9f:
            out.append(f"\\x{o:02x}")
        elif 0xdc80 <= o <= 0xdcff:
            out.append(f"\\x{o - 0xdc00:02x}")     # G2: a byte that is not UTF-8 (surrogateescape)
        elif o > 0x9f and unicodedata.category(c) == "Cf":
            out.append(f"\\u{o:04x}" if o <= 0xffff else f"\\U{o:08x}")
        else:
            out.append(c)
    return "".join(out)


def fold(name):
    """v2.19.2: a name as macOS's default filesystem (APFS, case- and normalization-insensitive) compares it: Unicode
    canonical caseless matching, NFD(casefold(NFD(name))). There .Envrc, FLAKE.NIX, flaKe.nix (a Kelvin sign) and
    ﬂake.nix (a ligature) are the files host direnv and nix open as .envrc and flake.nix, so a protect pattern's name
    and a .git compare folded, on every system: on a case-sensitive one, that only protects more."""
    import unicodedata
    return unicodedata.normalize("NFD", unicodedata.normalize("NFD", name).casefold())


GIT = fold(".git")


def walk_named(top, names):
    """The .git entries under TOP, and the entries a **/NAME pattern names (NAMES), each compared folded; never below
    a .git, never through a link. Sorted."""
    want = {fold(n) for n in names}
    gits, named = [], []
    for d, ds, fs in os.walk(top):
        for e in ds + fs:
            k = fold(e)
            if k == GIT:
                gits.append(os.path.join(d, e))
            elif k in want:
                named.append(os.path.join(d, e))
        ds[:] = [e for e in ds if fold(e) != GIT]
    return sorted(gits), sorted(named)


class Pins:
    """A workspace's pins: DIRS, folders bound writable onto themselves (each git directory, and every folder
    between a pin and its root, so none is renamed or removed), and RO, paths bound read-only onto themselves.
    compute() resolves and places them (the placeholders created); compute(place=False) only resolves them, so
    PLANNED holds the placeholders a backend creates first: what resolve() returns as data."""

    def __init__(self, top, common, protect, cfg):
        self.top, self.common, self.protect, self.cfg = top, common, protect, cfg
        self.dirs, self.ro, self.seen = [], [], set()
        self.placeholders = []
        self.planned = {}     # path -> (kind, text): the placeholders, in order ("folder" or "file")

    @classmethod
    def from_data(cls, d, protect):
        """The pins resolve() returned (launch.json's pins entry), for the before look and the after event."""
        p = cls(d["top"], d["common"], protect, {})
        p.dirs, p.ro = list(d["dirs"]), [r["path"] for r in d["ro"]]
        p.dotgits, p.named = list(d["dotgits"]), list(d["named"])
        return p

    def exists(self, p):
        return os.path.lexists(p) or p in self.planned

    def is_folder(self, p):
        return (os.path.isdir(p) and not os.path.islink(p)) or self.planned.get(p, ("",))[0] == "folder"

    def plan(self, p, kind, text=None):
        self.planned[p] = (kind, text)

    def rel(self, p):
        return p[len(self.top) + 1:] if p.startswith(self.top + "/") else p

    def nolink(self, p, root):
        d = p
        while len(d) > len(root):
            if os.path.islink(d):
                what = "make it read-only" if d == p else f"pin {self.rel(p)}, below it"
                raise Refused(f"{self.rel(d)} is a symbolic link, so the sandbox can't {what} (a run must not write "
                              "what decides the sandbox or what the host runs): replace the link with what it points "
                              "to, or remove it, then run again", "link", d)
            d = os.path.dirname(d)

    def pin(self, how, p):
        root = self.top if p.startswith(self.top + "/") else self.common
        self.nolink(p, root)
        up, d = [], p
        while True:
            d = os.path.dirname(d)
            if len(d) <= len(root):
                break
            up.insert(0, d)
        if how == "rw":
            up.append(p)
        for d in up:
            if d not in self.seen:
                self.seen.add(d)
                self.dirs.append(d)
        if how == "ro" and ("ro:" + p) not in self.seen:
            self.seen.add("ro:" + p)
            self.ro.append(p)

    def gitdirs(self, g):
        out = [g]
        wt = os.path.join(g, "worktrees")
        if os.path.isdir(wt):
            for d in sorted(os.listdir(wt)):
                if os.path.isfile(os.path.join(wt, d, "HEAD")):
                    out += self.gitdirs(os.path.join(wt, d))
        mods = os.path.join(g, "modules")
        if os.path.isdir(mods):
            out += self.below(mods)
        return out

    def below(self, d):
        out = []
        for e in sorted(os.listdir(d)):
            p = os.path.join(d, e)
            if os.path.isdir(p) and not os.path.islink(p):
                out += self.gitdirs(p) if os.path.isfile(os.path.join(p, "HEAD")) else self.below(p)
        return out

    def scan(self):
        self.dotgits, self.named = walk_named(self.top, [p[3:] for p in self.protect if p.startswith("**/")])

    def allgitdirs(self):
        out = []
        for e in self.dotgits:
            if os.path.isdir(e) and not os.path.islink(e):
                out += self.gitdirs(e)
        if self.common != os.path.join(self.top, ".git"):
            out += self.gitdirs(self.common)
        return out

    def compute(self, hooks, place=True):
        """Resolve the pins; with PLACE (bubblewrap's clone mode, Seatbelt) create the placeholders too. 0.3.0: every
        check comes before any placeholder, so a run that is refused leaves none."""
        self.scan()
        for g in self.allgitdirs():
            self.pin("rw", g)
            if os.path.isfile(os.path.join(g, "config")) and not os.path.isdir(os.path.join(g, "hooks")):
                self.plan(os.path.join(g, "hooks"), "folder")
            for name, body in PLACEHOLDERS.items():
                f = os.path.join(g, name)
                if not os.path.lexists(f) and os.path.isfile(os.path.join(g, "HEAD")):
                    self.plan(f, "file", body)
            for f in GITFILES:
                if self.exists(os.path.join(g, f)):
                    self.pin("ro", os.path.join(g, f))
        for e in self.dotgits:
            if not (os.path.isdir(e) and not os.path.islink(e)):
                self.pin("ro", e)
        for e in self.named:
            self.pin("ro", e)
        for p in self.protect:
            if p.startswith("**/"):
                continue
            ap = os.path.join(self.top, p.rstrip("/"))
            self.nolink(ap, self.top)
            if os.path.lexists(ap):
                self.pin("ro", ap)
        if hooks and (hooks.startswith(self.top + "/") or hooks.startswith(self.common + "/")):
            missing, d = [], hooks
            while not self.exists(d):
                missing.insert(0, d)
                d = os.path.dirname(d)
            for d in missing:
                self.plan(d, "folder")
            self.pin("ro", hooks)
        # a protected folder is read-only throughout: a mount point pinned below it (a scion's folder,
        # the parent of its .git file) is bound read-only too, or its deeper bind would make it writable
        rod = [p for p in self.ro if self.is_folder(p)]
        for d in [d for d in self.dirs if any(d.startswith(r + "/") for r in rod)]:
            self.dirs.remove(d)
            if ("ro:" + d) not in self.seen:
                self.seen.add("ro:" + d)
                self.ro.append(d)
        for f in self.ro:
            if f in self.planned:
                continue
            st = os.lstat(f)
            if stat.S_ISREG(st.st_mode) and st.st_nlink > 1:
                raise Refused(f"{self.rel(f)} has another hard link, so a run could write it through that link (a run "
                              "must not write what decides the sandbox or what the host runs): replace it with a copy "
                              "of its own (cp -p F F.copy && mv F.copy F), then run again", "hard-link", f)
        if place:
            self.place()

    def place(self):
        """Create the planned placeholders, never through what appeared at their paths since (a link is never
        followed): a folder (a git directory's hooks), or a file with its text, recorded in PLACED for cleanup."""
        for p, (kind, text) in self.planned.items():
            try:
                if kind == "folder":
                    os.mkdir(p)
                else:
                    with open(p, "x") as fh:
                        fh.write(text)
            except FileExistsError:
                raise Stop(f"{self.rel(p)} appeared after pot resolved the workspace's pins: run again") from None
            if kind == "file":
                self.placeholders.append([p, text])
                PLACED.append([p, text])
        self.planned = {}

    def data(self):
        """The pins as launch.json carries them (contracts/launch-1.md, pins)."""
        return {"top": self.top, "common": self.common, "dirs": list(self.dirs),
                "ro": [{"path": p, "kind": "folder" if self.is_folder(p) else "file"} for p in self.ro],
                "placeholders": [dict({"path": p, "kind": k}, **({"text": t} if k == "file" else {}))
                                 for p, (k, t) in self.planned.items()],
                "dotgits": list(self.dotgits), "named": list(self.named)}


def resolve(top, common, protect, hooks):
    """0.3.0: a workspace's pins as data, resolved without writing anything: what pot's core passes to the backend.
    A link on a protected path, or a protected file with a second hard link, is `refused` (the run stops), with
    bwrap's message; the lists are then empty."""
    p = Pins(top, common, protect, {})
    try:
        p.compute(hooks, place=False)
    except Refused as e:
        return {"top": top, "common": common, "dirs": [], "ro": [], "placeholders": [], "dotgits": [], "named": [],
                "refused": {"why": e.why, "path": e.path, "message": str(e)}}
    return p.data()


def resolve_workspace(gitbin, w):
    """resolve() for a policy.json workspace entry in place, with its sealed folders (sealed_dirs): the hooks path is
    host git's, as bwrap asked it (core.hooksPath set to /dev/null, so in practice /dev/null)."""
    top, common = w["path"], w["common"]
    hooks = git({"git": gitbin}, top, "rev-parse", "--path-format=absolute", "--git-path", "hooks").stdout.strip()
    d = resolve(top, common, w["protected"], hooks)
    d["sealed"] = [] if d.get("refused") else sealed_dirs({}, top, w.get("sealed") or [])
    return d


def place(d):
    """Create a resolved workspace's placeholders (resolve()'s data), as a backend's prepare does first; Stop when one
    appeared since. Returns the Pins, for bind lists and the before look."""
    p = Pins(d["top"], d["common"], [], {})
    for ph in d["placeholders"]:
        p.plan(ph["path"], ph["kind"], ph.get("text"))
    p.place()
    return p


def git_lines(s):
    """K1: git's output as lines, split at newlines only (str.splitlines also splits at U+0085, U+2028 and the
    other line breaks a run-chosen name may hold)."""
    return [l for l in s.split("\n") if l]


def git_run(argv, env=None, timeout=None):
    """K1: every host git call that reads run-chosen names (refs, paths, subjects) reads bytes, decoded with
    surrogateescape (a byte that is not UTF-8 survives; visible() shows it as \\xNN; a CR stays one). K3: pathspecs
    are literal (GIT_LITERAL_PATHSPECS), so a run-chosen path is never pathspec magic."""
    env = {**(os.environ if env is None else env), "GIT_LITERAL_PATHSPECS": "1"}
    r = subprocess.run(argv, capture_output=True, env=env, timeout=timeout)
    r.stdout = r.stdout.decode("utf-8", "surrogateescape")
    r.stderr = r.stderr.decode("utf-8", "surrogateescape")
    return r


def git(cfg, top, *args):
    """Host git reading a run-written repository: no hooks, no fsmonitor, no config it could run."""
    return git_run([cfg["git"], "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null",
                    "-c", "safe.directory=*", "-C", top] + list(args))


def fingerprint(cfg, pins):
    """What host git, a shell prompt, an editor or direnv would read and could run code from."""
    lines = set(pins.dotgits) | set(pins.named)
    for g in pins.allgitdirs():
        lines.add(g)
        for f in GITFILES:
            if os.path.lexists(os.path.join(g, f)):
                lines.add(os.path.join(g, f))
    for l in git_lines(git(cfg, pins.top, "ls-files", "-s").stdout):
        if l.startswith("160000 "):
            lines.add("gitlink " + l.split("\t", 1)[1])
    return sorted(lines)


def hostrun_hashes(top, protect):
    """Contents of host-run files (the protect list, plus §9's extras): path -> hash."""
    import hashlib
    out = {}
    pats = [p for p in protect if not p.startswith("**/")] + HOST_RUN_EXTRA
    names = {fold(p[3:]) for p in protect if p.startswith("**/")}

    def h(p):
        if os.path.isdir(p) and not os.path.islink(p):
            for d, ds, fs in os.walk(p):
                ds[:] = [x for x in ds if fold(x) != GIT]
                for f in fs:
                    h(os.path.join(d, f))
        elif os.path.lexists(p):
            try:
                with open(p, "rb") as f:
                    out[p[len(top) + 1:]] = hashlib.sha256(f.read()).hexdigest()
            except OSError:
                out[p[len(top) + 1:]] = "unreadable"
    for p in pats:
        h(os.path.join(top, p))
    for d, ds, fs in os.walk(top):
        ds[:] = [x for x in ds if fold(x) != GIT and x != "node_modules"]
        for f in fs + ds:
            if fold(f) in names:
                h(os.path.join(d, f))
    return out


def refs(cfg, top):
    r = git(cfg, top, "for-each-ref", "--format=%(objectname) %(refname)")
    head = git(cfg, top, "rev-parse", "HEAD").stdout.strip()
    return {"refs": git_lines(r.stdout), "head": head}



def sealed_dirs(cfg, top, patterns):
    """v2.19.1: the real folders under TOP that a sealedProtect pattern names: **/NAME anywhere (a .git pruned, as
    Pins.scan does), or a path from the top. The outermost only: a sealed folder's entries are read-only throughout."""
    out = []
    names = [p[3:] for p in patterns if p.startswith("**/")]
    if names and os.path.isdir(top):
        out += walk_named(top, names)[1]          # folded, as Pins.scan; links and files are left out below
    for p in patterns:
        if not p.startswith("**/"):
            out.append(os.path.join(top, p.rstrip("/")))
    out = sorted({d for d in out if os.path.isdir(d) and not os.path.islink(d)})
    return [d for d in out if not any(d.startswith(o + "/") for o in out)]


def sealed_left(base, entries, rel):
    """v2.19.1: what a run left in a seal's BASE, beyond the entries' mount points: (path shown, type, size), never
    following a link; pi's lock folders (proper-lockfile's NAME.lock) left out."""
    out = []
    for e in sorted(os.listdir(base)):
        if e in entries:
            continue
        p = os.path.join(base, e)
        if e.endswith(".lock") and os.path.isdir(p) and not os.path.islink(p):
            continue
        todo = [(p, f"{rel}/{e}")]
        while todo:
            q, shown = todo.pop(0)
            st = os.lstat(q)
            if stat.S_ISLNK(st.st_mode):
                out.append((shown, "link", None))
            elif stat.S_ISDIR(st.st_mode):
                kids = sorted(os.listdir(q))
                if not kids:
                    out.append((shown + "/", "folder", None))
                todo = [(os.path.join(q, k), f"{shown}/{k}") for k in kids] + todo
            elif stat.S_ISREG(st.st_mode):
                out.append((shown, "file", st.st_size))
            else:
                out.append((shown, "other", None))
    return out


def tilde(p, home):
    return "~" + p[len(home):] if p == home or p.startswith(home + "/") else p


def workspace_hidden(cfg, policy, hidden, home):
    """v2.19.1: the HIDDEN paths under an in-place workspace's top that exist (each masked by the backend), the start
    notes for those git tracks (the run sees them empty), and per top the tracked ones' relative paths (the after
    event's hidden_blanked)."""
    ws_hidden, notes, tracked_by = [], [], {}
    for w in policy["workspace"]:
        if w.get("clone"):
            continue
        top = w["path"]
        for h in hidden:
            if (h.startswith(top + "/")) and os.path.lexists(h) and h not in ws_hidden:
                ws_hidden.append(h)
                rel = os.path.relpath(h, top)
                tracked = git(cfg, top, "ls-files", "-z", "--", rel).stdout
                if tracked:
                    notes.append(f"pot: {tilde(h, home)} is hidden here, but git tracks it: the run sees it empty, and "
                                 "git's history still holds it; don't commit it from this run")
                    tracked_by.setdefault(top, []).append(rel)
    return ws_hidden, notes, tracked_by


def before_look(cfg, pinsets):
    """Before an in-place run: what the after event compares (before.json): each workspace's fingerprint of what
    host git, a prompt, an editor or direnv would read, its host-run files' hashes, and its refs."""
    before = {"pins": [], "hostrun": {}, "refs": {}}
    for pins in pinsets:
        before["pins"].append({"top": pins.top, "fingerprint": fingerprint(cfg, pins)})
        before["hostrun"][pins.top] = hostrun_hashes(pins.top, pins.protect)
        before["refs"][pins.top] = refs(cfg, pins.top)
    return before


def look_after(cfg, policy, before, ev, hidden_tracked, repo_kit_paths):
    """After an in-place run: into the after event EV, against BEFORE (before_look's), the new git metadata, the
    changed host-run files, the new commits (and the unsigned ones), the hidden paths committed empty and the repo
    kit's changed files."""
    for b in before["pins"]:
        top = b["top"]
        w = next(w for w in policy["workspace"] if w["path"] == top)
        pins = Pins(top, w["common"], w["protected"], cfg)
        pins.scan()
        new = sorted(set(fingerprint(cfg, pins)) - set(b["fingerprint"]))
        ev["new_git_metadata"] += [visible(n.replace(top + "/", "")) for n in new]
        now = hostrun_hashes(top, w["protected"])
        was = before["hostrun"].get(top, {})
        ev["touches_host_run"] += sorted(visible(p) for p in set(now) | set(was) if now.get(p) != was.get(p))
        old = set(before["refs"].get(top, {}).get("refs", []))
        cur = refs(cfg, top)
        # commits new on local branches and HEAD only: fetched ones (refs/remotes) are not the run's
        local = [l for l in cur["refs"] if l.split(" ", 1)[1].startswith("refs/heads/")]
        newrefs = [l.split(" ", 1)[0] for l in local if l not in old]
        if cur["head"] and cur["head"] != before["refs"].get(top, {}).get("head"):
            newrefs.append(cur["head"])
        if newrefs:
            olds = [l.split(" ", 1)[0] for l in old] + ([before["refs"][top]["head"]] if before["refs"].get(top, {}).get("head") else [])
            r = git(cfg, top, "rev-list", *newrefs, "--not", *olds) if olds else git(cfg, top, "rev-list", *newrefs)
            commits = [c for c in r.stdout.split() if c]
            ev["commits"] += [c[:12] for c in commits]
            if commits:
                sig = git(cfg, top, "log", "--no-walk", "--format=%G?", *commits).stdout.split()
                ev["unsigned"] += sum(1 for s in sig if s == "N")
                ev["base"] = before["refs"].get(top, {}).get("head", "")[:12]
                # v2.19.1: a hidden path git tracks, which the run saw empty, committed empty at a new tip
                for rel in hidden_tracked.get(top, []):
                    for tip in newrefs:
                        t = git(cfg, top, "ls-tree", "-z", tip, "--", rel).stdout.split("\0")[0]
                        if t and t.split()[2:3] == [EMPTY_BLOB] and visible(rel) not in ev.setdefault("hidden_blanked", []):
                            ev["hidden_blanked"].append(visible(rel))
        # the repo kit's own files only: its flake directory, or the top's flake.nix and flake.lock
        flake = repo_kit_paths.get(top, [])
        ev["repo_kit_change"] += [p for p in ev["touches_host_run"] if any(p == f or p.startswith(f + "/") for f in flake)]


def read_own(rundir, name):
    """A file of the run's, never through a link (O_NOFOLLOW), as JSON; None when it is not there."""
    try:
        fd = os.open(os.path.join(rundir, name), os.O_RDONLY | os.O_NOFOLLOW)
    except FileNotFoundError:
        return None
    with os.fdopen(fd) as f:
        if not stat.S_ISREG(os.fstat(f.fileno()).st_mode):
            raise OSError(f"{name}: not a regular file")
        return json.load(f)


def main(argv):
    """`pins.py before|after GIT RUNDIR`: the container backend's look, bwrap's code. before writes before.json and
    prints {"notes": [...]}; after prints the in-place part of the after event, with sealed_left from sealed.json (the
    backend's seals: dest, base, entries, rel)."""
    if len(argv) != 3 or argv[0] not in ("before", "after"):
        print("usage: pins.py before|after GIT RUNDIR", file=sys.stderr)
        return 2
    verb, cfg, rundir = argv[0], {"git": argv[1]}, argv[2]
    try:
        policy, launch = read_own(rundir, "policy.json"), read_own(rundir, "launch.json")
        if policy is None or launch is None:
            raise OSError("policy.json or launch.json is missing")
        protect = {w["path"]: w["protected"] for w in policy["workspace"]}
        if verb == "before":
            before = before_look(cfg, [Pins.from_data(d, protect[d["top"]]) for d in launch.get("pins") or []])
            hidden = [p for i in policy["items"] if i["kind"] == "hidden" and i["effective"] for p in i.get("paths", [])]
            _, notes, before["hiddenTracked"] = workspace_hidden(cfg, policy, hidden, launch["home"])
            path = os.path.join(rundir, "before.json")
            if os.path.lexists(path):
                os.unlink(path)
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "w") as f:
                json.dump(before, f)
            print(json.dumps({"notes": notes}))
            return 0
        before = read_own(rundir, "before.json")
        if before is None:
            raise OSError("before.json is missing (run prepare first)")
        ev = {"commits": [], "unsigned": 0, "touches_host_run": [], "repo_kit_change": [], "new_git_metadata": [],
              "sealed_left": []}
        for sd in read_own(rundir, "sealed.json") or []:
            try:
                ev["sealed_left"] += [{"path": visible(p), "type": t, "size": z} for p, t, z in
                                      sealed_left(sd["base"], sd["entries"], sd["rel"])]
            except OSError:
                pass
        look_after(cfg, policy, before, ev, before.get("hiddenTracked") or {}, launch.get("repoKitPaths") or {})
        print(json.dumps(ev))
        return 0
    except (OSError, ValueError, KeyError) as e:
        print(visible(f"pins.py {verb}: {e}"), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
