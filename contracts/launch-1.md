---
status: draft
---
# launch.json/1: the launcher's facts, for a run's backend

The host facts a backend needs beside the policy: the command, the caller's identity, the closure, the host's tools,
and the sibling and clone facts. pot writes it as `<run dir>/launch.json` before it calls the backend's `prepare`,
next to [policy.json/1](policy-1.md). The policy says what the run may reach; this file says who runs what, where.
It never enters the run: no backend binds it there.

| | |
|---|---|
| Version | 1: the file name's contract; the document has no `version` field (below) |
| Producer | pot (`core/pot.py`: `launch_facts`, then `run` adds the run's facts, the pins among them from `core/pins.py`); the backend's `prepare` adds its own keys |
| Reader | a backend: bubblewrap (`core/bwrap.py`) and Seatbelt (`core/seatbelt.py`), built with pot, and a backend built apart, such as a container backend; pot itself (the start notes; a sibling's host entry reads its caller's file) |
| Owner | pot |
| Schema | [launch-1.schema.json](launch-1.schema.json) (JSON Schema 2020-12) |
| Fixtures | [fixtures/launch-1/](fixtures/launch-1/) |

**Why it has a version now.** It was internal while every backend was built with the pot that wrote it. A backend
built as its own binary reads it across the same seam as policy.json, so it is written down the same way.

## Shape

```json
{
  "argv": [ "bash" ], "cwd": "/home/u/src/r", "home": "/home/u", "user": "u", "uid": 1000, "gid": 100, "group": "users",
  "closure": [ "/nix/store/…-bash-5.3", "…" ], "git": { "name": "test", "email": "test@example.com" },
  "xdgRuntime": "/run/user/1000", "potBin": "/nix/store/…-pot/bin", "resolve": { }, "pane": "%177",
  "hostTools": { "nix": "/nix/store/…/bin/nix", "ssh": "/run/current-system/sw/bin/ssh", "workmux": "…", "tmux": "…" },
  "netFix": "…", "pkgFix": "…", "toolsFix": "…", "root": { "contexts": { "work": { "include": [ "stand" ] } } },
  "hostSigns": false, "hostPot": [ "/nix/store/…/bin/python3", "-I", "-c", "…", "/nix/store/…-pot-root.json" ],
  "wsFacts": [ { "top": "/home/u/src/r", "id": "repo:…", "…": "…" } ], "tree": "/home/u/.local/state/pot/runs/7f3a",
  "depth": 0, "repoKitPaths": { "/home/u/src/r": [ "flake.nix", "flake.lock" ] },
  "pins": [ { "top": "/home/u/src/r", "common": "/home/u/src/r/.git", "dirs": [ "/home/u/src/r/.git" ],
              "ro": [ { "path": "/home/u/src/r/.git/config", "kind": "file" }, { "path": "/home/u/src/r/flake.nix", "kind": "file" } ],
              "placeholders": [ { "path": "/home/u/src/r/.git/commondir", "kind": "file", "text": ".\n" } ],
              "dotgits": [ "/home/u/src/r/.git" ], "named": [ "/home/u/src/r/flake.nix" ], "sealed": [ ] } ],
  "policyDigest": "sha256-…",
  "closureCount": 158, "sealed": [ ], "notes": [ ]
}
```

**Two forms.** A run's file has every field down to `repoKitPaths`, and `policyDigest`. `pot explain
--backend-command` writes only the first fourteen (`argv` to `hostTools`), and `pins` with write access, before it
calls `prepare --print` ([explain.json](fixtures/launch-1/explain.json)). The schema ties them: with `policyDigest`,
the run's fields are required.

**When it is read.** pot writes the file twice before `prepare` (the second time with `hookEnv` and `policyDigest`);
a backend reads the second. bubblewrap's and Seatbelt's `prepare` read it, add their keys and write it back; `run`,
`after` and `cleanup` read that.

**Marks.** **MUST**: a backend reads it to run the command correctly, or to write a correct `after` event; getting
it wrong runs the wrong command, as the wrong person, or reports the wrong result. **Info**: text for people (the
denials, the not-found message); a backend may show it, never acts on it. **pot**: read back by pot itself; a
backend leaves it as written and need not read it. **Backend**: added by a backend's `prepare` (below).

### Always present

| Field | Type | | Meaning |
|---|---|---|---|
| `argv` | list of strings, one or more | MUST | The command. An argument `/run/pot/…` is the run folder as the run sees it (Seatbelt rewrites it to `<run dir>/…`). |
| `cwd` | absolute path | MUST | The working directory in the run. A sibling's is its caller's, as the caller saw it. |
| `home` | absolute path | MUST | The run's `HOME`; `~/…` in the policy (state mounts, `env.set`, files) is under it. |
| `user`, `group` | string | MUST | The caller's names: `USER`, `LOGNAME`, and the run's `passwd` and `group` entries. |
| `uid`, `gid` | integer | MUST | The caller's ids: the run's user is the caller, never root. |
| `closure` | list of store paths | MUST | The store paths the run sees when the backend proves `closure-store`: the root's, the bundles', `--package`'s and the kits' files' closures. A backend adds its own runtime's closure. With the `nix` binding, or inspect's `view`, the whole store is visible instead. |
| `git` | `{ name?, email? }` | MUST | The host's `user.name` and `user.email`, each when set: the run's generated `~/.gitconfig` ([pot reference](../docs/reference/pot.md)), which signs nothing. |
| `xdgRuntime` | `/run/user/<uid>` | Info | The caller's runtime folder. bubblewrap's inspect view keeps it empty; a backend whose view hides `/run` need not read it. |
| `potBin` | absolute path, or `null` | MUST | This pot's `bin/`: bound read-only at `mounts.launcher` (policy.json/1), on the run's `PATH`, and the `pot trailer` hook in `~/.gitconfig` (when it is in the store). `null`: no launcher in the run. |
| `resolve` | `{}` | Info | Always empty, never read. |
| `pane` | `%<n>`, or `null` | MUST | The caller's tmux pane, when it runs in tmux: the `status.relay` broker's target, and where the proxy shows a denial. |
| `hostTools` | `{ nix?, ssh?, workmux?, tmux? }` | MUST | The host programs the bindings' helpers run, on the host, never in the run: `ssh` for `git.fetch`, `nix` for the `nix` relay (and its `bin/` on the run's `PATH`), `workmux` for `status.relay`, `tmux` for the proxy's notices. Each when it is on the caller's `PATH`. A binding whose program is missing stops the run before the command starts. |

### A run's

| Field | Type | | Meaning |
|---|---|---|---|
| `netFix`, `pkgFix`, `toolsFix` | string | Info | The permanent fix a denial names (`--net`), a missing command (`--package`), and the root's tools. |
| `root` | `{ contexts }` | pot | The root's contexts as the root wrote them; pot names them when a command is not found. |
| `hostSigns` | boolean | MUST | Whether the host signs commits: the `after` event's `signs`. |
| `hostPot` | argv, or `null` | MUST | The host's pot as a sibling's host entry runs it: the siblings' broker's `--entry`, when a `children` row has `how: "sibling"`. `null` (no root manifest): no siblings' broker. |
| `wsFacts` | list | pot | Each repository as pot found it at the start (before grants narrowed it), from which a sibling is composed without evaluating again. A repo kit's and a scion's `manifest` and the root's `repoKits` entries are copied as written. |
| `tree` | absolute path | MUST | The top run's folder: the siblings' slots are under pot's state, named after it. |
| `depth` | integer | pot | Sibling starts above this run (0 for a top run); pot caps it. |
| `repoKitPaths` | `{ top: [ relative path ] }` | MUST | Each repository's repo-kit files (its flake directory, or `flake.nix` and `flake.lock`): the `after` event's `repo_kit_change`. |
| `policyDigest` | `sha256-<hex>` | pot | The digest of `policy.json`'s bytes: a sibling's host entry checks its caller's policy against it. A backend never rewrites `policy.json`. |

### A run's, when it applies

| Field | When | | Meaning |
|---|---|---|---|
| `siblingOf` | a sibling | MUST | Its caller's run id. A sibling leaves `SIGINT` to its command, its proxy shows no one-run fix, and its `cleanup` leaves the tree's slots to the top run. |
| `inClone` | a sibling | MUST | The sibling runs in its caller's clone: its caller stages the clone, so its own `after` does not. |
| `hookEnv` | a sibling, or a pre-commit hook | MUST | `GIT_INDEX_FILE` (only when it names a workspace repository's index) and `GIT_PREFIX`, added to the run's environment. No other variable. |
| `clone` | clone mode | MUST | `base` (the commit the clone started from), `objects` (the source's object folder) and each of `scions` (`name`, `path`, `commit`, `objects`): what `after` stages the clone's commits from. `branch` and `path` are Info. |
| `rootRepo` | clone mode, of the root's repository | MUST | `true`: `after` flags changes to the root's files (`root_attention`). |
| `grantsRel` | with `rootRepo`, when the grants file is inside it | MUST | The grants file's path in the repository, flagged too. |
| `pins` | write access, a workspace in place (0.3.0) | MUST | Each in-place workspace's pins, which pot's core resolved at the run's start ([below](#pins)). |

### pins

With `access: write`, pot's core resolves each in-place workspace's pins once, as the run starts, and every backend
carries out the same list (decided 2026-11-21: one implementation, `core/pins.py`, no copy in a backend). It describes
this host at this moment, so it is here and not in policy.json. One entry per workspace entry in place (none for a
clone), in policy.json's order:

| Key | Type | Meaning |
|---|---|---|
| `top`, `common` | absolute path | The workspace's `path` and `common`, as policy.json has them |
| `dirs` | list of absolute paths | Folders bound writable onto themselves: each git directory (the top's `.git`, a worktree's common directory and its `worktrees/NAME`, a submodule's under `modules/`), and every folder between a pin and the top or the common directory. A mount point can't be renamed or removed, so no pin can be moved away with its folder |
| `ro` | list of `{ path, kind }` | Bound read-only onto themselves, over the writable workspace: each git directory's `config`, `config.worktree`, `hooks`, `info`, `commondir` and `gitdir`, a `.git` file, what a `**/NAME` pattern names, and each protected path that exists. `kind` is `file` or `folder` (a placeholder's is the kind it will have) |
| `placeholders` | list of `{ path, kind, text? }` | What the backend creates before it binds, each listed in `ro` too: an absent `commondir` (text `.\n`), `config.worktree` or `gitdir` (empty) in a git directory, so a run can't create one that host git would read; and a git directory's `hooks` folder (`kind: folder`, no text). A file is created exclusively, never through a link; one that appeared since stops the run. The backend removes a file placeholder after the run while it still holds its text, and leaves a folder |
| `dotgits`, `named` | lists of absolute paths | The `.git` entries under the top, and what a `**/NAME` pattern names (names compared folded, as macOS compares them), as found: what the before look's fingerprint starts from |
| `sealed` | list of absolute paths | The outermost real folders a `sealed` pattern names: each sealed as bubblewrap seals one (its entries read-only, new entries landing in the run folder) |
| `refused` | `{ why, path, message }`, optional | A pin the sandbox can't make: `why` is `link` (a link on a protected path, or above one) or `hard-link` (a protected file with a second hard link). The backend's `prepare` stops the run with `message`, bubblewrap's; the lists are then empty |

A backend checks each path again before it binds it (a link there, or a file with a second hard link, stops the run:
something changed since pot resolved it). bubblewrap and the container backend bind these lists; Seatbelt resolves
its own with the same code. A launch.json without `pins` for a write run (one a test wrote) makes bubblewrap resolve
them itself, with the same code, and the container backend refuse write access.

### Added by a backend

A backend's `prepare` may add these, and pot's tests validate the file after `prepare`, so they are part of the
schema.

| Field | By | | Meaning |
|---|---|---|---|
| `notes` | any backend | pot | Start notes: each a warning pot prints on the host's terminal after `prepare` (a sealed folder over the cap, a hidden path git tracks). |
| `closureCount` | bubblewrap, Seatbelt | Backend | How many store paths the run sees, or `the whole store (nix)` / `the whole store`: for `--backend-command`. |
| `walk` | bubblewrap, Seatbelt | Backend | Inspect's view: the secrets and sockets the host walk masked, and its seconds. |
| `sealed` | bubblewrap | Backend | Each sealed folder (`dest`, its `base` in the run folder, the `entries` it had, `rel`): `after`'s `sealed_left`. |
| `hiddenTracked` | bubblewrap | Backend | Per repository, the hidden paths git tracks: `after`'s `hidden_blanked`. |

`closureCount`, `walk`, `sealed` and `hiddenTracked` are the built-in backends' own state, kept here because they are
built with pot. A backend built apart adds only `notes`; it keeps its own state in its own file in the run folder.

## Versioning

- **No version field.** pot writes the file and the backend reads it in the same run, and the policy beside it
  carries the version (`policy.json`'s `version: 1`). A pot that writes launch.json/1 writes policy.json/1; a later
  pair names both.
- **Fields, not values, need a version.** A new field, or a changed meaning, is launch.json/2. A new host tool in
  `hostTools` is a new field. One exception, in 0.3.0 (a contract change below 1.0, so a `0.(y+1)` release): `pins`,
  added within version 1. It is optional, and no reader runs wider for not knowing it: bubblewrap and Seatbelt are
  built with the pot that writes it, and a container backend before 0.3.0 refuses `access: write`, the only runs
  that carry it (it reads launch.json leniently, so the field is ignored, never refused).
- **Strict schema.** Every object pot builds is closed. Objects copied as written stay open: `root.contexts`' values
  and, in `wsFacts`, a `manifest` and the `rootRepoKits` entries.
- **No secret values.** Nothing in the file is a secret: credentials are read on the host at run time from the
  policy's `bindings` (references, never values), `hookEnv` holds two git variables and no other, and `git` holds a
  name and an address. A backend never writes a secret value here.

## For backend authors

Read [policy.json/1's](policy-1.md#for-backend-authors) first: the verbs and the rules. With launch.json:

1. Run `argv` in `cwd` as `user` (`uid`, `gid`, `group`), with `HOME` `home`, and the environment policy.json/1 and
   [run-env/1](run-env-1.md) describe, plus `hookEnv`.
2. Make visible the store paths in `closure` (plus the backend's own runtime) when it proves `closure-store`.
3. Start each binding's helper with the program in `hostTools`; stop before the command starts when one is missing.
4. Bind `potBin`'s `pot` at `mounts.launcher` when `potBin` is set; start the siblings' broker with `hostPot` and
   `tree` when a row is a sibling and `hostPot` is set.
5. With write access in place, create `pins`' placeholders, bind its `dirs` writable and its `ro` read-only over the
   workspace (each onto itself, deeper paths over shallower ones), seal its `sealed` folders, and stop on a `refused`
   entry; check each path again before the bind.
6. Write the `after` event from `hostSigns`, `repoKitPaths`, `clone`, `rootRepo`, `grantsRel` and `inClone`.
7. Never make launch.json visible in the run, never rewrite `policy.json`, and never change pot's fields; add only
   `notes`.

## Fixtures

[fixtures/launch-1/](fixtures/launch-1/) holds files that pot and bubblewrap wrote in pot's own tests
(`tests/run.sh`), after `prepare`, with the host's facts made synthetic: the home as `/home/u`, the user as `u`, the
host's name as `test-host` and the tests' work folder as `/tmp/pot-test`. Each of policy.json/1's fixtures from a run
has its launch.json under the same name, from the same run; Seatbelt's was composed without a run, so it has none.

| Fixture | What it shows |
|---|---|
| `work-bindings.json`, `git-fetch.json`, `pi-agent.json`, `work-siblings.json` | a top run in place: `hostTools`, `hostPot`, `pane`, a `closureCount` |
| `dispatch-clone.json` | `clone`, with a scion |
| `root-repo.json` | a clone of the root's repository: `rootRepo` |
| `sibling.json` | a sibling: `siblingOf`, `depth` 1, empty `hookEnv` |
| `hook.json` | a pre-commit hook's sibling: `hookEnv` with `GIT_INDEX_FILE` and `GIT_PREFIX` |
| `sibling-in-clone.json` | a sibling in its caller's clone: `inClone` |
| `nix.json` | `closureCount` `the whole store (nix)` |
| `inspect.json` | inspect: `walk`, no `closureCount` |
| `hidden-tracked.json` | `hiddenTracked`, two `sealed` folders and a `note`; `pins` with two sealed folders |
| `explain.json` | what `pot explain --backend-command` writes: the first fourteen fields, and `pins` (from `work-bindings.json`'s run) |
| `work-bindings.json`, `git-fetch.json`, `nix.json`, `pi-agent.json`, `work-siblings.json` | `pins` (0.3.0): placeholders, a submodule's git directory, a protected folder (`scions`), a sealed folder (`pi-agent.json`) |

`invalid/` holds documents a reader must reject: each breaks one rule above.

The `pins` (0.3.0) were added to the fixtures by hand: what `core/pins.py`'s `resolve_workspace` returns for
workspaces made like the tests' (`sbc`, `piws`, `nc`), with the paths made the fixtures'.

pot's tests hold pot to the schema: `tests/run.sh contracts` validates the fixtures and refuses the invalid ones;
the `pot`, `dispatch`, `sibling` and `pi` sections validate every `launch.json` their runs wrote.

## Sources

- [core/pins.py](../core/pins.py): `Pins`, `resolve`, `resolve_workspace`, `place` (the pins); `before_look`,
  `look_after`, `workspace_hidden` (the host's look)
- [core/pot.py](../core/pot.py): `launch_facts`, `host_tools`, `hook_env`, `host_pot`, `workspace_pins`, `run` (writing launch.json),
  `backend_notes`, `sibling_entry`, `explain`
- [core/bwrap.py](../core/bwrap.py): `prepare`, `run_env`, `gitconfig_text`, `start_bindings`, `run`, `after`,
  `clone_after`, `cleanup`
- [core/seatbelt.py](../core/seatbelt.py): `prepare`, `run_argv`, `start_proxy`, `run`, `leftovers`
- [tests/run.sh](../tests/run.sh): the `contracts` section, and `launches_valid`
