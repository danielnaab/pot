---
status: draft
---
# The container backend (pot-container)

pot's third backend, beside bubblewrap and Seatbelt: it carries out a run's [policy.json/1](../../contracts/policy-1.md)
as one `podman run` or `docker run`, a fresh container per run, on any host or CI that has either. It is pot's
first Rust code, a binary of its own behind the policy-1 contract (`backends/container/`, the flake's
`packages.pot-container`). This page is the first slice (pot phase 2, plan step 3): it probes and renders the
command; `run` and `after` are not written yet, and no root offers the backend. The transcripts that test it are
the pages under [container/](container/probe.md).

```
pot-container [--config CONFIG] probe|prepare|run|after|cleanup RUNDIR [--print]
```

| Verb | This slice |
|---|---|
| `probe` | Prints `{ "guarantees": [ … ] }`, or none beside an `error` ([transcript](container/probe.md)) |
| `prepare` | Checks the policy and refuses what it can't carry out; places the run's files and `container.argv` (the command, NUL-separated). `--print` prints the command and writes nothing ([transcript](container/prepare.md)) |
| `run`, `after` | Refuse: not in this slice |
| `cleanup` | Removes what `prepare` placed; never fails |

A refusal is one line on stderr, `pot (<context>): …`, exit 1.

## Inputs

**`--config`** (JSON; unknown keys refused): `engine` (`podman` or `docker`; by default podman when it is on `PATH`,
else docker), `podman` and `docker` (the programs), `image` (docker's empty image, default `localhost/pot-empty:1`),
and the store paths bound into every run, named as bwrap's config names them: `sh`, `shell`, `env`, `cacert`,
`terminfo`, `tzdir` and `baseClosure` (a file of store paths). `prepare` needs all of the store paths.

**`launch.json`** ([launch.json/1](../../contracts/launch-1.md)) is read leniently, and only these fields:

| Field | Used for |
|---|---|
| `argv` | The command |
| `cwd` | Where it starts (`env -C`); under `access: none`, the home instead, always |
| `home` | The run's `HOME`: an empty tmpfs, owned by the run's user |
| `user`, `group` | The run's `/etc/passwd` and `/etc/group` |
| `uid`, `gid` | `--user`; with podman, the id `--userns=keep-id` keeps. `prepare` refuses a `uid` that is not the caller's |
| `closure` | The store paths bound read-only under `closure-store` |
| `potBin` | `/run/pot/launcher` and `~/.config/pot/launcher`, and on `PATH` |

## Probe

The engine must run as the caller and filter system calls; the checks are the phase 2 spike's:

- **podman:** rootless (`podman info`: never root); the caller's `NoNewPrivs` is 0 (rootless podman's setuid
  `newuidmap` fails under it, and systemd's `DynamicUser` sets it); its seccomp filter is on; version 4.4 or later.
- **docker:** its seccomp profile is on; version 20.10 or later. docker-group membership is root-equivalent: use
  docker only where the caller already is (a CI VM).
- `resource-limits` when the engine reports the memory and pids controllers (podman's delegated controllers;
  docker's `MemoryLimit` and `PidsLimit`). pot proves it per run (a run in its user systemd scope), so it ignores
  the probe's.

Guarantees, in bubblewrap's vocabulary (no new names):

| Guarantee | How |
|---|---|
| `closure-store` | Each store path of the closure bound read-only; never the whole store |
| `own-netns` | `--network none`: only `lo` |
| `own-pidns` | The engine's own PID namespace |
| `no-tiocsti` | The container never holds the host's terminal: the engine relays its streams through pipes (or a pty of its own with `-t`) |
| `local-listen` | Its own loopback |
| `in-place` | The workspace at its own path |
| `root-files` | A `file` item at `/…`, bound into the container's own root |
| `resource-limits` | podman only: `--cgroups=split` keeps the container in the cgroup of the pot that starts it, so the scope's `MemoryMax` and `TasksMax` apply (seen in a smoke run: the container's cgroup is a child of its caller's) |

Not proved yet: `protected-as-mounts`, `same-path-clone`, `private-proxy` (see [What's next](#whats-next)); never:
`nest`, `separate-kernel`.

**pot must not cache this probe.** pot caches a backend's probe per build; this one's answer depends on the caller
(its `NoNewPrivs`, the engine's state and configuration), so pot must probe it for every run.

## The engine's configuration

podman and docker read configuration that changes every container: `CONTAINERS_CONF_OVERRIDE` and the user's
`~/.config/containers/containers.conf` can add a volume, a variable or `seccomp_profile = "unconfined"`;
`~/.config/containers/mounts.conf` adds mounts; `CONTAINER_HOST` and `DOCKER_HOST` (or a docker context) send the
command to another engine. The conformance audit (2026-11-21) saw each widen a run while the probe claimed every
guarantee. So the engine starts with:

- **an environment of pot's**: the caller's cleared, but for `PATH HOME USER LOGNAME XDG_RUNTIME_DIR
  DBUS_SESSION_BUS_ADDRESS TMPDIR LANG TERM COLORTERM` (`engine::KEEP`) and the run's variables passed by name;
- **podman:** `CONTAINERS_CONF` the run's `containers.conf` (it replaces every other containers.conf; no default
  mounts file, volumes, variables or devices; the default seccomp profile); `CONTAINERS_STORAGE_CONF` the run's
  `storage.conf` (native overlay, no `mount_program`: a user's storage.conf named a program the engine ran at the
  run's start); and `HOME` the run's `enginehome`, emptied at each `prepare`, since podman reads
  `~/.config/containers/mounts.conf` from `$HOME` whatever containers.conf says;
- **docker:** `DOCKER_CONFIG` an empty folder of the run's (probe: one that does not exist).

**The store is the run's own** (`enginestore`, removed by `cleanup`): with `--rootfs` there are no image layers to
keep, and a fresh store cost about 10 ms a start against a persistent one (rootless podman 5.4.1 on platform-vm,
2026-11-21: 105 ms against 96 ms, mean of five `true` runs); native rootless overlay works on kernel 6.6, so no
`mount_program` is needed. **probe** runs the engine in a temporary folder of its own (`$TMPDIR/pot-container-probe-…`,
made 0700, holding podman's home, storage.conf and store) and removes it: it leaves nothing in the folder pot gives
it, which is often pot's state.

**A run's variables reach the container through the engine's environment** (`--env NAME`: the engine reads the
value, so no value is in an argument or a file; policy-1 rule 6 rules out an env file). So a run may not be granted
a variable the engine reads as configuration: `prepare` refuses an effective `env` item named `CONTAINERS_*`,
`_CONTAINERS_*`, `CONTAINER_*`, `DOCKER_*`, `PODMAN_*`, `BUILDAH_*`, `STORAGE_*`, `XDG_*`, `DBUS_*`, `LD_*`, `HOME`,
`PATH`, `TMPDIR`, `NOTIFY_SOCKET`, `REGISTRY_AUTH_FILE`, `SSH_AUTH_SOCK`, or one of Go's runtime variables (`GODEBUG`,
`GOGC`, `GOMAXPROCS`, `GOMEMLIMIT`, `GOTRACEBACK`) (`engine::configures_engine`). A later `run` could hand the
values over a pipe instead (`--env-file /dev/fd/N`), which would lift this.

`prepare` writes these as `container.env` (the names kept and the values set; no secret's value) for `run`, and
`--print` shows them. A root that offers this backend should also hide `~/.config/containers` (and
`~/.docker/config.json`, which pot hides already) from its runs; adding `~/.config/containers` to pot's default
hidden list is a change to pot's core, listed in [What's next](#whats-next).

## Policy to flags

| Policy | podman | docker |
|---|---|---|
| (always) | `run --rm --interactive --log-driver none --cidfile RUNDIR/container.id --hostname pot --ipc private --cgroupns private` | the same |
| (always) | `--cap-drop all --security-opt no-new-privileges --read-only`, the engine's default seccomp profile | the same |
| the run's user (`launch.json` `uid`, `gid`) | `--userns keep-id --user UID:GID --passwd=false` | `--user UID:GID` |
| the root | `--rootfs RUNDIR/rootfs:O` (an empty folder, an overlay so nothing is written to it); options before it | an empty imported image, `--pull never` |
| the working directory | `/usr/bin/env -C CWD -- ARGV` (podman takes no `-w` with `--rootfs`) | the same |
| `/tmp`, `/run`, `HOME` | `--tmpfs …,notmpcopyup` (podman's default copies the folder below into the tmpfs); the home `mode=0700,U` | `--tmpfs`; the home `mode=0700,uid=UID,gid=GID` |
| no effective network reach item | `--network none` | the same |
| a bind | `--volume SRC:DEST:ro` (or `:rw`) | `--mount type=bind,source=SRC,target=DEST[,readonly]`: docker's `--volume` creates a missing source, as root |
| `closure-store` proved | a read-only bind for each path of `launch.json` `closure` and the config's base closure | the same |
| `closure-store` not proved | `/nix/store` bound read-only | the same |
| `/bin/sh`, `/usr/bin/env`, the CA bundle, `/etc/passwd`, `/etc/group`, `/etc/hosts` | read-only binds | the same |
| `access: read` (in place) | `PATH` bound read-only, and `common` when it is outside (a worktree's); in each, sockets and hidden paths masked | the same |
| `access: none` | no workspace at all (pot 0.2.0): `PATH` and `common` masked where a bound folder would show them (bwrap's `unseen`); the command starts in the home | the same |
| `read`, `write` items | bound read-only or writable, where the path exists and is not hidden | the same |
| `socket` items | bound; every other socket under a bound folder or the workspace is masked | the same |
| `hidden` items | under a bound folder or the workspace: a read-only bind of an empty folder (or file) over it; nothing is bound inside a hidden folder | the same |
| `mounts.state` | `DIR` bound writable at `HOME/AT` (`prepare` creates `DIR`) | the same |
| `file`, mode `fixed` | `SOURCE` bound read-only at `PATH` | the same |
| `env.set` | `--env NAME=VALUE` (`~` as the home) | the same |
| `env` items, `TERM`, `COLORTERM` | `--env NAME`: the engine reads the value from its own environment, so no value is in an argument | the same |
| pot's variables | `HOME USER LOGNAME SHELL PATH LANG TERMINFO_DIRS SSL_CERT_FILE NIX_SSL_CERT_FILE TZDIR POT POT_POLICY POT_RUN PS1`, `POT_TOOLS`, `POT_CLONE` (bwrap's `run_env`) | the same |
| `mounts.run` | `/run/pot/policy.json` and `/run/pot/denials`, read-only | the same |
| `mounts.launcher` | `POTBIN/pot` at `/run/pot/launcher`; `~/.config/pot/launcher` links to it | the same |
| `resource-limits` proved | `--cgroups split` | refused |

A mask is a read-only bind of an empty file or folder of the run's, never a `--tmpfs`: the first smoke run's tmpfs
mask showed the hidden folder's contents (podman's `tmpcopyup` fills a tmpfs with what is below it; every podman
tmpfs now says `notmpcopyup`). The engines mount by destination depth.

**Paths are compared as strings**, so `prepare` refuses a bound path, a workspace top or common, or a socket that is
not its own real path (a link, or `..`), naming it. A hidden path that is a link (pot lists a hidden link and its
real path; Home Manager's files are links) is not refused: the link is masked where a bound folder shows it, and so
is its real path, by the kind of what it points to. And it refuses a bind whose source does not exist (the run folder's own files and the state
directories aside), since docker would create it.

## Refusals

`prepare` refuses, naming every gap in one line, never running without one (policy-1 rules 1-5):

**What the contract says a backend built apart must refuse** (the policy is read strictly: an unknown field is
refused, since a field this backend does not know could narrow the run):

1. a `version` other than 1; a `backend.name` other than `container`;
2. a guarantee in `backend.proved` or `requires` this contract does not know, or a `requires` the probe did not
   prove; a proved guarantee this slice does not uphold;
3. an effective item of an unknown kind; a malformed item (the schema's conditionals and patterns);
4. a binding of a reserved or unknown capability; an effective `unsandboxed` item.

**What this slice does not carry out yet** (each is refused, never dropped):

- write access in place: the workspace's pins (git directories, `config`, hooks and the protected paths as read-only
  mounts, links and hard links refused, the placeholders), as bwrap's `Pins`;
- clone mode: the clone at its original path, its pins, proposal links, and the `after` staging fetch;
- the network: any effective network reach item (pot's proxy over a socket in `/run/pot`, with its 108-byte
  `sun_path` limit);
- every binding: `status.relay`, `git.fetch`, `nix`, `credential`, `mcp` (with `via: file` as a 0400 file under
  `/run/pot`, decided 2026-11-21);
- a children table with a `sibling` row (the run's children broker);
- inspect's `view`;
- `dir` items (folders of links) and `file` items in mode `seed` or `reassert`;
- `resource-limits` under docker;
- a path a bind can't name (`:`, `,` or `"`), and a command whose name holds `=` (`env` would read it as a variable);
- a run's variable that configures the container engine (above);
- a path that is not its real path, and a bind source that does not exist (above).

A decoding error names where it is and what was expected there (`items[2].effective: expected a boolean`), never the
value found: a secret misplaced into a field is never printed. `prepare` can run again: it replaces its files, never
writing through a link planted in the run folder, and names the file it could not write.

Not placed yet, which leaves the run narrower than bubblewrap's, never wider: `/run/pot/README.md`, `explain.txt`,
`why.json`, `bin/` (the shims), `notfound/` and `bash_env` (the not-found advice), `mcp.json`, `~/.gitconfig`, and
`/run/current-system`. Sealed folders are read-only (only read access is carried out, so every workspace folder
is).

## What's next

1. `run`: the docker image (`docker import` of an empty tar), `-t` when stdin is a terminal, the exit status; then
   the proxy over a bound socket, and the bindings.
2. Write access: pot's core resolves the workspace's protected paths and pins and passes them to the backend
   (decided 2026-11-21: one implementation, no port of bwrap's `Pins`); the backend binds them read-only.
3. `after` and `cleanup` for clones; the conformance review with executed evidence per guarantee (plan step 4).
4. Closure delivery as an image built from a root's closure (plan step 5), for hosts without `/nix/store`.
5. pot choosing the backend from a root, probing it for every run; `~/.config/containers` in pot's default hidden
   list (a core change).

## Sources

- [backends/container/src/](../../backends/container/src/): `policy.rs` (reading policy.json/1), `probe.rs`,
  `render.rs` (the flags), `main.rs` (the verbs)
- [core/bwrap.py](../../core/bwrap.py): `prepare`, `run_env`, `masked`, `Pins`, `cleanup` (the reference backend)
- [core/pot.py](../../core/pot.py): `launch_facts` (launch.json), `scope_for` (resource-limits)
- The phase 2 spike, executed on platform-vm on 2026-11-21 (flexion-assistant session record
  `2026-11-21-pot-phase-2/spike.md`): rootless podman and docker flags, `--rootfs DIR:O`, no `-w` with `--rootfs`,
  options before `--rootfs`, keep-id against `--user`, docker's `noexec` tmpfs, NoNewPrivileges and `newuidmap`
- A smoke run of this slice's command under rootless podman 5.4.1 on platform-vm (2026-11-21): the run's uid and
  gid kept, the workspace read-only (EROFS), the home and state writable, `CapEff` 0, `NoNewPrivs` 1, seccomp
  filter on, 17 store paths visible, a secret variable passed by name, the container's cgroup under its caller's
  with `--cgroups split`; a `--tmpfs` mask under the workspace bind did not hide the folder, a bind did
- The conformance audit (2026-11-21, real containers) and the smoke runs after its fixes: with `access: none` and
  the workspace's parent bound, the workspace was empty and the command started in the home; with a hostile
  `~/.config/containers` (a volume, a variable, `seccomp_profile = "unconfined"`, a mounts.conf) and
  `CONTAINERS_CONF_OVERRIDE`, the run had `Seccomp: 2`, no variable and neither mount (before: `Seccomp: 0`, both);
  a home tmpfs under a bound folder was empty (before, with podman's default: a copy of the host folder); a `..`
  path and a linked workspace were refused
- The re-audit (2026-11-21) and its smoke run: a granted `CONTAINERS_CONF_OVERRIDE` variable is refused; a caller
  storage.conf with a `mount_program` no longer runs it (before: it ran at the run's start); a mounts.conf left in
  the engine's home is gone after `prepare`; probe leaves its folder empty; a hidden link under a bound folder and
  its real path are both empty in the run
