---
status: draft
---
# The container backend (pot-container)

pot's third backend, beside bubblewrap and Seatbelt: it carries out a run's [policy.json/1](../../contracts/policy-1.md)
as one `podman run` or `docker run`, a fresh container per run, on any host or CI that has either. It is pot's
first Rust code, a binary of its own behind the policy-1 contract (`backends/container/`, the flake's
`packages.pot-container`). This page is its fifth slice (pot phase 2, plan step 3): it probes, renders the
command and runs it, for runs in place with read access, write access or none, with the network through pot's proxy
and the `credential` and `mcp` bindings; no root offers the backend yet. The transcripts
that test it are the pages under [container/](container/probe.md).

```
pot-container [--config CONFIG] probe|prepare|run|after|cleanup RUNDIR [--print]
```

| Verb | This slice |
|---|---|
| `probe` | Prints `{ "guarantees": [ … ] }`, or none beside an `error` ([transcript](container/probe.md)) |
| `prepare` | Checks the policy and refuses what it can't carry out; places the run's files and `container.argv` (the command, NUL-separated). `--print` prints the command and writes nothing ([transcript](container/prepare.md)) |
| `run` | Reads the bindings' values on the host, then runs `container.argv` with the engine environment `prepare` recorded; exits with the command's status once the container is gone ([Run and cleanup](#run-and-cleanup), [Bindings](#bindings)) |
| `after` | Prints the run's `after` event: bwrap's, with the proxy's `denials` and `loopback`; with write access, the host's look after the run (commits, changed host-run files, new git metadata, sealed leftovers: [Write access](#write-access)) |
| `cleanup` | Stops the run's proxy if it still runs, removes the run's container and store through the engine, then what `prepare` and `run` placed (a via-file binding's values, the placeholders, the sealed folders' bases among them); names every removal it could not make (exit 1) |

A refusal is one line on stderr, `pot (<context>): …`, exit 1.

## Inputs

**`--config`** (JSON; unknown keys refused): `engine` (`podman` or `docker`; by default podman when it is on `PATH`,
else docker), `podman` and `docker` (the programs), `image` (docker's empty image, default `localhost/pot-empty:1`),
and the store paths bound into every run, named as bwrap's config names them: `sh`, `shell`, `env`, `cacert`,
`terminfo`, `tzdir` and `baseClosure` (a file of store paths). `prepare` needs all of the store paths. The network's
programs, also named as bwrap's config names them: `python` and `proxy` (pot's `proxy.py`), which run on the host,
`socat`, which runs in the run, and `netClosure` (a file of store paths: socat's closure, bound into a run with the
network under `closure-store`). Unless `python` and `proxy` are files and `socat` a real file under a path of
`netClosure` (one test, which `probe` and `prepare` share), `probe` does not prove `private-proxy` and `prepare`
refuses the network and `private-proxy`. No contract field is added: these are the backend's own config, and the proxy's other
inputs are launch.json/1's existing fields (below). Write access (0.3.0) needs `pins` (pot's `core/pins.py`) and
`git` (host git) beside `python`, each a file: the host's look before and after the run. Without them `probe` does
not prove `protected-as-mounts` and `prepare` refuses write access.

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
| `pane`, `hostTools.tmux` | The proxy's notices of a denial on the caller's tmux pane (`--tmux`, `--pane`), as bwrap's |
| `netFix` | The permanent fix a denial names (the proxy's `--fix`) |
| `siblingOf` | A sibling's proxy shows no one-run fix (`--sibling`) |
| `pins` | Write access: each workspace's pins, as pot's core resolved them ([Write access](#write-access)); read strictly (an unknown key is refused) |
| `repoKitPaths` | Write access: the look's `repo_kit_change` (read by `pins.py`) |

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
| `protected-as-mounts` | When the config names the look's programs (`python`, `pins`, `git`, each a file): write access binds each pin pot's core resolved onto itself, read-only over the writable workspace, a mount point the run can't write, rename, remove or replace ([Write access](#write-access)) |
| `private-proxy` | When the config's network programs pass the one test `prepare` applies too (`render::net_gaps`, V2): `python` and `proxy` files, `socat` a real file (not a link) under a path of `netClosure`, which is required. The proxy listens on a socket in the run folder, which only the caller can reach, and the run serves it on 127.0.0.1:3128 in its own network namespace ([The network](#the-network)) |

Not proved yet: `same-path-clone` (see [What's next](#whats-next)); never: `nest`,
`separate-kernel`.

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
`mount_program` is needed. **So is its runtime state**: podman's global options name the store (`--root`), the
runroot (`--runroot RUNDIR/engineroot`: the mount table, the container's runtime files) and the temporary files
(`--tmpdir RUNDIR/enginetmp`: exit codes). A runroot in the run's storage.conf is not enough: podman ignores it for
a rootless user when `CONTAINERS_STORAGE_CONF` names the file (podman 5.4.1, 2026-11-21). The sockets are no limit
on the run folder's path: conmon's attach socket is in the store (218 bytes in a smoke run), bound and reached
through `/proc/self/fd`, and with `--network none` the runroot has none. **probe** runs the engine in a temporary folder of its own (`$TMPDIR/pot-container-probe-…`,
made 0700, holding podman's home, storage.conf and store) and removes it: it leaves nothing in the folder pot gives
it, which is often pot's state.

**A run's plain variables reach the container through the engine's environment** (`--env NAME`: the engine reads
the value, so no value is in an argument). The engine writes that value into the container's configuration
(`enginestore/overlay-containers/ID/userdata/config.json`, `$XDG_RUNTIME_DIR/crun/ID/config.json`, docker's
container config), which is acceptable for a plain variable and not for a secret (policy-1 rule 6: never in the
run's files; the conformance audit of 2026-11-21 found a secret there, R3). **A secret variable** (an `env` item with
`secret: true`) never reaches the engine, and neither does a `credential` or `mcp` binding's ([Bindings](#bindings)).
With either, every variable passed by name (the secrets, the bindings', `TERM`, `COLORTERM` and the plain items) goes
over a pipe instead. `run` writes `NAME=value` records, each ended by a NUL, then the
sentinel record `.`. The pipe is a named pipe, `RUNDIR/secrets.fifo` (decided 2026-11-22, for both engines: docker
passes no file descriptor, and podman's `--preserve-fd` passed one of the caller's): `run` makes it just before the
engine starts, whatever is at its path removed first (a link is removed, never followed), mode 0600, and checks that
it is a pipe of the caller's with that mode. The plan binds it read-only at `/run/pot/secrets.fifo`; the container's
user is the caller (podman's `keep-id`, docker's `--user` with no user namespace), so it can open it under both
engines. `run` opens it without following a link and without blocking (retried until the container has it open), and
writes in a thread without blocking, until the engine exits, so `run` never waits on a container that ended before it
read. Once it has written the sentinel it unlinks the pipe from the run folder (V3): the container's bind holds the
pipe's own inode and the entry has it open, so the run is unaffected, and no host process can open it any more. It
removes the pipe when the command ends if it is still there, and `cleanup` does too. No fd of the caller's reaches the
container. The read-only bind does not stop a process in the container from opening the pipe for writing (a mount's
read-only flag does not apply to a pipe's I/O); that is harmless: the command starts only after the entry has read
every record and closed the pipe, so nothing in the run reads what is written there afterwards, and `run` has
finished with it (a blocking open waits for a reader that never comes; a non-blocking one fails).
The entry starts with `/usr/bin/env -i --`, which clears what the engine set. Then the run's bash (the config's `shell`) opens the pipe read-write
(so its open never waits for `run`), collects the records up to the sentinel, each within 30 s, and closes it. It holds
the stream to the planned names (V1), which it is given as arguments (their count, then each: names are not secret):
exactly one record per planned name, in the planned order, `NAME=value`, or the bare `NAME` for a variable the host has
unset (it stays unset in the run), then the sentinel. `run` writes every planned name so. Any other stream exits 125
and nothing runs: no pipe, a short read, a timeout, a name not planned (`LD_PRELOAD` written by a process of the same
user that opened the pipe first), a name twice, a name missing, or a sentinel before the last planned name. Otherwise it declares each record in bash (`unset -v NAME`, which drops
a special meaning such as `RANDOM`'s, then `declare -x`). A record is never an argument: an earlier version passed
the records to `env -i`, and the audit read a secret in that process's `/proc/PID/cmdline` (T1). Bash then execs the
last hop, `/usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C CWD -- POT'S VALUES… ARGV`, whose arguments hold only names
and pot's values, which are not secret. The command's environment is then exactly the planned one: pot's values,
`env.set`, the items and the secrets. Nothing of bash's (`SHLVL`, `PWD`, `_`) or the engine's (`HOSTNAME`) is
added, and a secret named like a bash variable (`RANDOM`, `SECONDS`, `IFS`) keeps its value (audit S1, S3, T1). For
this to hold, `prepare` refuses, for every `env` item and under either engine, a name bash can't hand over: read-only
in bash (`UID`, `EUID`, `PPID`, `BASHOPTS`, `SHELLOPTS`), set by bash itself at its start, its exec or after every
command (`SHLVL`, `PWD`, `OLDPWD`, `_`, `PIPESTATUS`), unset by it (`PS1`, `PS2`), `BASH_*`, or the script's own
`__pot_*` (`engine::bash_sets`). Without a secret, `env -C CWD -u HOSTNAME --` removes the one variable podman adds, unless the plan sets it. The
value is in no file, no argument and not in the engine's environment (checked in a real run: a marker value was in
no file of the run folder or of `$XDG_RUNTIME_DIR/crun`, and in no engine process's arguments or environment, while
the command printed it; and with every process on the host watched while a 100 000-byte variable slowed the
script's reads, no process's arguments ever held a record's value). With a secret, a binding or the network, `prepare` refuses a shell that is not a real file under a path the run binds
(S5), and with the network a `socat` that is not one. The run's bash is used
rather than this binary, which would need its own closure (glibc, libgcc) bound, a closure the config does not name. A plain variable may not be one the engine reads as configuration: `prepare` refuses an effective `env` item named `CONTAINERS_*`,
`_CONTAINERS_*`, `CONTAINER_*`, `DOCKER_*`, `PODMAN_*`, `BUILDAH_*`, `STORAGE_*`, `XDG_*`, `DBUS_*`, `LD_*`, `LISTEN_*`, `HOME`,
`PATH`, `TMPDIR`, `NOTIFY_SOCKET`, `REGISTRY_AUTH_FILE`, `SSH_AUTH_SOCK`, or one of Go's runtime variables (`GODEBUG`,
`GOGC`, `GOMAXPROCS`, `GOMEMLIMIT`, `GOTRACEBACK`) (`engine::configures_engine`). For a secret variable
this list is no longer needed, since the engine never sees it: only the names pot sets itself (`HOME`, `PATH` and the
rest of pot's variables) would stay refused. It still applies to secret variables for now; lifting it is a later
change. Plain variables already take the same pipe when a secret, a binding or the network is there; otherwise they do
not.

`prepare` writes these as `container.env` for `run`: the names kept, the values set, the names that go over the
pipe (and whether the pipe is used), the proxy's command, the `credential` and `mcp` bindings (their `capability`,
`selector`, `env`, `from` and `via`: references, never a value), and the kind (file or folder) of every file of the
run's that the plan binds, but for the pipe and the proxy's socket, which `run` makes. It holds no secret's value.
`prepare --print` shows them, and the proxy's command. pot's default hidden list holds `~/.config/containers` and
`~/.docker/config.json` (unreleased): no run can read or change the configuration of a later container run.

## Policy to flags

| Policy | podman | docker |
|---|---|---|
| (always) | `--root RUNDIR/enginestore --runroot RUNDIR/engineroot --tmpdir RUNDIR/enginetmp`, before `run` | none |
| (always) | `run --interactive --log-driver none --cidfile RUNDIR/container.id --hostname pot --ipc private --cgroupns private`; no `--rm` ([Run and cleanup](#run-and-cleanup)) | the same, and `--name pot-HASH` (16 hex digits of the run folder's path, FNV-1a) |
| (always) | `--init`: the engine's init is PID 1 (catatonit, or docker's `docker-init`), a read-only bind of the engine's own file; the command is never PID 1, which ignores a signal it has no handler for | the same |
| stdin a terminal | `--tty`, added by `run` | the same |
| a secret `env` item, a `credential` or `mcp` binding, or the network | `--volume RUNDIR/secrets.fifo:/run/pot/secrets.fifo:ro`; no `--env NAME`; the entry is `/usr/bin/env -i -- SHELL --noprofile --norc -c SCRIPT pot-entry /run/pot/secrets.fifo SOCAT N NAMES… /usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C CWD -- POT'S VALUES… ARGV` (`SOCAT` empty without the network) | the same, with `--mount` |
| (always) | `--cap-drop all --security-opt no-new-privileges --read-only`, the engine's default seccomp profile | the same |
| the run's user (`launch.json` `uid`, `gid`) | `--userns keep-id --user UID:GID --passwd=false` | `--user UID:GID` |
| the root | `--rootfs RUNDIR/rootfs:O` (an empty folder, an overlay so nothing is written to it); options before it | an empty imported image, `--pull never --entrypoint ""` |
| the working directory | `/usr/bin/env -C CWD -u HOSTNAME -- ARGV` (podman takes no `-w` with `--rootfs`); with a secret, a binding or the network, as above | the same |
| `/tmp`, `/run`, `HOME` | `--tmpfs …,notmpcopyup` (podman's default copies the folder below into the tmpfs); the home `mode=0700,U` | `--tmpfs`; the home `mode=0700,uid=UID,gid=GID` |
| (always, with the network or without) | `--network none`: only `lo` | the same |
| an effective network reach item | `--volume RUNDIR/proxy.sock:/run/pot/proxy.sock:ro`; socat's closure (`netClosure`) bound read-only with the closure; the entry starts socat on 127.0.0.1:3128 ([The network](#the-network)) | the same, with `--mount` |
| the same | `--env` `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy` `=http://127.0.0.1:3128`, `NO_PROXY`, `no_proxy` `=localhost,127.0.0.1,::1` (bwrap's `run_env`, set last: they beat `env.set` and an item of the same name) | the same |
| a bind | `--volume SRC:DEST:ro` (or `:rw`) | `--mount type=bind,source=SRC,target=DEST[,readonly]`: docker's `--volume` creates a missing source, as root |
| `closure-store` proved | a read-only bind for each path of `launch.json` `closure` and the config's base closure | the same |
| `closure-store` not proved | `/nix/store` bound read-only | the same |
| `/bin/sh`, `/usr/bin/env`, the CA bundle, `/etc/passwd`, `/etc/group`, `/etc/hosts` | read-only binds | the same |
| `access: read` (in place) | `PATH` bound read-only, and `common` when it is outside (a worktree's); in each, sockets and hidden paths masked | the same |
| `access: write` (in place) | `PATH` bound writable, and `common` when it is outside (a worktree's); then launch.json's `pins`: each of `dirs` bound writable onto itself and each of `ro` read-only onto itself; sockets and hidden paths masked as with read access; each sealed folder a folder of the run's (`RUNDIR/sealed/N`) bound writable at its path with its entries bound read-only into it ([Write access](#write-access)) | the same, with `--mount` (`readonly` for `ro`) |
| a bind under a bound host folder, or a pin (read or write access) | the entry checks it first: `SHELL --noprofile --norc -c PIN_CHECK pot-pins STAT /run/pot/binds.txt`, then the rest of the entry; without the pipe the last hop is `/usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C CWD …` ([Write access](#write-access)) | the same |
| `access: none` | no workspace at all (pot 0.2.0): `PATH` and `common` masked where a bound folder would show them (bwrap's `unseen`); the command starts in the home | the same |
| `read`, `write` items | bound read-only or writable, where the path exists and is not hidden | the same |
| `socket` items | bound; every other socket under a bound folder or the workspace is masked | the same |
| `hidden` items | under a bound folder or the workspace: a read-only bind of an empty folder (or file) over it; nothing is bound inside a hidden folder | the same |
| `mounts.state` | `DIR` bound writable at `HOME/AT` (`prepare` creates `DIR`) | the same |
| `file`, mode `fixed` | `SOURCE` bound read-only at `PATH` | the same |
| `env.set` | `--env NAME=VALUE` (`~` as the home) | the same |
| `env` items (not secret), `TERM`, `COLORTERM` | `--env NAME`: the engine reads the value from its own environment, so no value is in an argument | the same |
| an `env` item named like one of pot's values | passed by name, with no `--env NAME=VALUE`; pot's value is its default, which `run` uses when the host has it unset (bwrap's order: pot's values, the host's `TERM` and `COLORTERM`, `env.set`, then the items the host has set) | the same |
| pot's variables | `HOME USER LOGNAME SHELL PATH LANG TERMINFO_DIRS SSL_CERT_FILE NIX_SSL_CERT_FILE TZDIR POT POT_POLICY POT_RUN PS1`, `POT_TOOLS`, `POT_CLONE` (bwrap's `run_env`) | the same |
| `mounts.run` | `/run/pot/policy.json`, `/run/pot/denials` and `/run/pot/mcp.json` (bwrap's, for every run), read-only | the same |
| a `credential` or `mcp` binding | its variables over the named pipe, last (they beat pot's values, `env.set`, the items and the proxy's variables), never `--env` ([Bindings](#bindings)) | the same |
| a binding with `via: file` | `--volume RUNDIR/secrets:/run/pot/secrets:ro` (a folder of the run's, 0700) | the same, with `--mount` |
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

## Write access

`access: write` in place (0.3.0) carries out what bubblewrap's `Pins` did, from the list pot's core resolved once as
the run started (launch.json/1's [`pins`](../../contracts/launch-1.md#pins), `core/pins.py`; decided 2026-11-21: one
implementation, no port of `Pins` here):

- **Binds.** The workspace's top (and a worktree's common directory, outside it) writable at its own path; each of
  `dirs` (every git directory, and every folder between a pin and its root) writable onto itself; each of `ro` (a
  git directory's `config`, `config.worktree`, `hooks`, `info`, `commondir` and `gitdir`, a `.git` file, what a
  `**/NAME` pattern names, each protected path that exists) read-only onto itself. A pin is a mount point: the run
  can't write it (EROFS), or rename, remove or replace it, or rename a file over it (EBUSY); a folder above a pin is
  a mount point too, so a pin can't be moved away with its folder. So `git config` fails (its rename over
  `.git/config`), while a commit works (objects, refs and the index are writable). podman and docker mount by
  destination depth, so each pin lands over the writable workspace (tested under both: `tests/run.rs`,
  `write_access_in_place_holds_every_pin`). Sockets and hidden paths in the workspace are masked as with read
  access, and nothing is bound inside a hidden folder, a pin neither (bwrap's K4). A protected path that does not
  exist at the start is not pinned, as under bubblewrap: the run can create it, and `after` reports it.
- **Placeholders.** `prepare` creates each of `pins`' placeholders before the engine binds them (a file exclusively,
  never through a link; one there already stops the run), records the files in `placeholders.json` (bwrap's), and
  binds them read-only: a run can't create `commondir`, `config.worktree` or `gitdir` in a git directory, or a hooks
  folder where git had none. `cleanup` removes each file that still holds what `prepare` wrote, and leaves a folder,
  as bwrap does.
- **Refusals.** A `refused` entry (a link on or above a protected path, a protected file with a second hard link)
  stops `prepare` with its message, bwrap's, before anything is placed. `prepare` then checks each pin again (no
  link, its kind, a protected file with one link, no placeholder there yet): the host may have changed it since pot
  resolved it.
- **Checked again before the engine.** `prepare` records each workspace path the plan binds (the top, each pin, each
  sealed entry) with its device and inode, and a file's size and status-change time (X3: a file replaced by one
  that reuses its inode, or rewritten in place, is not taken for it), in `container.env`; `run`, just before it
  starts the engine, refuses one that is now another file or changed, a link, of another kind, or (a protected
  file) has a second hard link: `the workspace changed since prepare: PATH (…); run prepare again`, and the engine
  never starts.
- **Checked inside the container, before the command** (X1, Y1: QA 2026-11-22). The engines resolve a bind's source
  and destination by path when they mount it, so a process that can rename in a bound folder (a process of the
  caller's, or a concurrent run, sandboxed or not, whose own pins do not cover that folder) can swap a folder for a
  link in the moment between `run`'s check and the engine's mount: the engine then binds another folder at a pin
  (under podman the QA's swap won 6 of 22 tries, and the run read and wrote the outside folder), or lands a mask or a
  pin elsewhere (a hidden folder's mask landed on a decoy and the command read the hidden secret: podman 3 of 30 with
  write access and 11 of 30 with read access, docker 2 of 30). Binding from a descriptor `run` holds open is not
  possible: rootless podman can't open another process's `/proc/PID/fd/N` (`statfs …: permission denied`), and runc
  refuses to bind a mount from another mount namespace (`mount src=/proc/PID/fd/3 …: invalid argument`), both tried
  on platform-vm. So the entry checks first, from inside, under read and write access alike: `prepare` writes
  `binds.txt`, bound read-only at `/run/pot/binds.txt`, a line `DEV:INO PATH` for every bind whose destination is
  under a bound host folder (a pin, a hidden path's or a socket's mask, a socket, a `read` or `write` item, a state
  folder or a seal there; a destination in the container's own tmpfs or root can't be reached from outside) and for
  every pin, with its source's device and inode (a mask's source is the run folder's `empty` or `emptydir`). The run's
  bash, before anything else, checks that each is a mount point (`/proc/self/mountinfo`) whose file has that device and
  inode (coreutils' `stat`, beside the config's `env`). Otherwise nothing runs: exit 125, naming each with the mount
  found there (Y2: its source and root, mountinfo's fields 10 and 4): `pot: the container is not the binds prepare
  recorded: PATH (another file: mounted from /dev/sda, root /…/victim) …; the engine may have bound another folder
  there and made mount points in it; nothing ran`. Tested under both engines with the QA's swaps
  (`a_pin_swapped_for_a_link_while_run_starts_is_never_used`, `a_mask_swapped_away_while_run_starts_never_shows_the_hidden_path`,
  read and write): no try read or wrote the outside folder or read the hidden path; most were stopped by this check.
- **What the check leaves** (Y2). It detects, and does not prevent, the engine making a mount point (an empty file or
  folder named like the pin below it) in a folder it was handed by the swap: podman left some in 9 of the QA's 220
  tries. The run never writes there, and `cleanup` does not remove them, since they are in a folder that is not the
  run's: the 125 message names the root, the host path the engine bound. Under rootless podman they belong to one of
  the caller's subordinate ids; remove them with `podman unshare rm -- ROOT/NAME` (docker's are root's).
- **During the run** (Y6). A mount inside the container follows its folder when it is renamed on the host, but a file
  renamed over a pin from outside (a host process, under bubblewrap too) replaces the host's entry: the pin inside
  is detached from the workspace's view, and `after` reports the change to a protected path. The checks above hold
  for what the command starts with, not against the host changing the workspace while it runs.
- **A changed pin** (Y5). A file pin whose status-change time changed between `prepare` and `run`, even by `touch`
  or `chmod` with the same contents, is refused as `another file, or changed`: run `prepare` again.
- **Sealed folders**, as bwrap's `seal`: each of `pins`' `sealed` folders (not hidden, at most 256 entries) is the run
  folder's `sealed/N`, bound writable at its path, with each entry of the host's bound read-only into it (`prepare`
  makes its mount point in `sealed/N` first, so no engine creates one as root) and a link made there as the same
  link. No entry can be written, replaced, renamed or removed, and a new entry (pi's `settings.json.lock`) lands in
  `sealed/N`, never on the host. A folder with more entries stays as it is, with bwrap's start note. `cleanup`
  removes `sealed/`, making a folder the run left unwritable writable first.
- **The host's look.** `prepare` runs pot's own code, `PYTHON -I -S PINS before GIT RUNDIR` (the config's programs, in
  pot's environment, as bwrap's `prepare` runs), which writes `before.json` (bwrap's: the git metadata fingerprint,
  the host-run files' hashes, the refs) and prints the start notes for hidden paths git tracks; `prepare` adds those
  and a sealed folder's note to launch.json's `notes`, the one key a backend built apart adds. It runs in an
  environment of its own (X4): `PATH`, `HOME`, `USER`, `LOGNAME`, the locale, `TZ`, `TMPDIR`, `XDG_CONFIG_HOME` and `GNUPGHOME` (git's configuration and its
  signature check), and no other variable of the caller's, a `GIT_*` one neither. `after` runs `PINS
  after`, which reports what bwrap's `after` does for an in-place write run: `commits`, `unsigned`, `base`,
  `touches_host_run`, `repo_kit_change`, `new_git_metadata`, `hidden_blanked`, and `sealed_left` (from the seals
  `prepare` recorded in `sealed.json`); a missing `before.json` is an empty look, as bwrap's (X6). It reads the run
  folder's files never through a link, and its arguments hold paths only. `agent_config` stays empty, as it is for
  bwrap's in-place runs. launch.json's keys come back in serde_json's order when `prepare` adds notes (pot reads them
  by name; keeping the order would need serde_json's `preserve_order` and a new crate, so it is left: X6).

## Run and cleanup

**`run`** reads `container.argv` and `container.env` (regular files only, never a link). Just before it starts the
engine, it checks that the run folder's engine paths are as `prepare` left them (audit R2), since the engine writes
through a link and binds the run's files. `enginestore`, `enginehome` and `rootfs` (docker: `dockercfg`) must be real
folders. `engineroot`, `enginetmp`, `emptydir` and `potcfg` must be real folders or absent. The run's files must be
regular files or absent. Every file and folder of the run's that the plan binds (`policy.json`, `denials`,
`passwd` and the rest, S4) must still be the kind `prepare` recorded. `container.id` must be absent, and `status` a
file or nothing (an empty folder there is removed). Otherwise `run` refuses, naming each path, and the engine
never starts. It starts the engine with the caller's environment cleared but for the names `container.env`
keeps, then the values it sets. stdin, stdout and stderr are the engine's. It adds `--tty` when stdin is a terminal.
Before the engine it makes the named pipe ([above](#the-engines-configuration)) and starts the proxy
([The network](#the-network)), then checks the run folder again, the pipe and the proxy's socket included (each must
be what `run` just made: a pipe, a socket). Whatever it started is stopped and removed when the command ends, and on
every early return. docker's root is the config's image, by default
`localhost/pot-empty:1`. `run` imports it from an empty tar when it is missing (`docker import`) and refuses an image
that is not the one empty layer with no configuration (R4): `Entrypoint`, `Cmd`, `Env`, `Volumes`, `User`,
`WorkingDir`, `OnBuild`, `Labels`, `ExposedPorts`, `Healthcheck`, `StopSignal`, `Shell` and `ArgsEscaped` must be
empty, null or unset. Its name is content-stable, so runs share it and `cleanup` leaves it.

**`run` returns only once the container is gone** (audit N7). With `--rm`, conmon's cleanup (its exit command, a
`podman container cleanup`) outlived `podman run` and recreated `enginestore` after `cleanup`. So there is no
`--rm`: once the engine exits, `run` removes the container (podman: `rm --all --force --time 0` in the run's own
store, so a container whose id never reached the cidfile goes too; docker: `rm --force --volumes` with the run's name,
R1, R4). It then waits until none of the engine's processes for this run is left, up to 30 s. Those are a podman
named as the configured program is (its name, its real file's, or nix's `.NAME-wrapped`, S7) whose arguments hold
the exact pair `--root RUNDIR/enginestore` (its `run`, `rm`, `stop` and conmon's exit command),
and a conmon whose bundle (`-b`) is in the run's store, whose `--exit-dir` is the run's, or whose pidfile is in the
run's runroot. A process that only names a path there is not one of them (R5). It writes the status to `RUNDIR/status`, as bwrap's `run` does, and exits with it: the command's
exit code, or 128 and the signal's number when a signal ended the engine. A removal that failed is one line on
stderr; the status is still the command's.

**Signals.** SIGINT is the command's: `run` keeps running, and the terminal sends it to the engine too, which passes
it on (bwrap's `run` ignores it the same way). SIGTERM or SIGHUP stops the container through the engine (`stop
--time 10`: SIGTERM, then SIGKILL after 10 s), then `run` removes it as above. A stop before the container exists is
retried each second until it does, so the engine's create is never cut off half done (R1: an early SIGTERM had left
a docker container behind). podman with still no container after 10 s is killed; its store is the run's alone. The
handlers are in place before `run` checks anything: a SIGTERM before the engine starts ends the run with 143, its
status recorded, and no engine started (S6). `status` exists once `run` has finished, as pot's `helpers_alive`
reads it.

**`cleanup`** (audit N6) removes, in order:

- The run's proxy, if it still runs (a `run` killed with SIGKILL leaves it for up to a second): a process whose
  arguments hold `proxy.sock` and the run's own `--denials RUNDIR/denials` and whose working folder is the run folder,
  stopped with SIGTERM, then SIGKILL after 2 s; one that would not stop is named.
- The container, through the engine (podman's when the run's store is not empty; docker's by the run's name),
  waiting for the engine's processes as `run` does. The engine is never handed a linked path (R2). When
  `engineroot`, `enginetmp` or `enginehome` is not a real folder, or the removal or the wait fails, the engine's
  state is kept whole and named, never deleted under a container that may still run. So it is when `enginestore`
  is not a real folder, and whenever a process of this run's engine is alive, whatever the store is (S2: deleting the
  runroot under a live container left its mounts in podman's namespace, which no later cleanup could remove).
- The engine's state (`enginestore`, `engineroot`, `enginetmp`). What the user can't remove (files of another id of
  the user namespace, an overlay's mode-000 `work/work`) is removed with `podman unshare rm -rf`, run by a podman
  whose own store is a private temporary folder, never the run's.
- The engine's home, what `prepare` placed, and the named pipe and the proxy's socket. `proxy.log` stays, as
  bwrap's does (the run's record, which `after` reads). It never follows a link planted in the run folder (a
link is removed, not followed) and never empties a folder that should be empty (`rootfs`, `emptydir`). Every
removal it could not make is named in one refusal (exit 1); what it removed stays removed, so it can run again.

Outside the run folder, as with any rootless podman: the OCI runtime's state for a running container
(`$XDG_RUNTIME_DIR/crun/ID`, gone once the container is removed) and the user's podman pause process, which keeps
the user namespace for later podman commands and is shared with them.

## The network

An effective `network` item of class `reach` is carried out with pot's own proxy, as bubblewrap does: the run reaches
exactly the allowlist, through the proxy, and nothing else (policy-1 rule 5). The container keeps `--network none`,
so it has only `lo`: a connection that bypasses the proxy has no route, and 127.0.0.1 is the run's own loopback,
never the host's.

- **The proxy, on the host.** `run` starts `PYTHON -I -S PROXY proxy.sock --denials RUNDIR/denials --context CTX
  [--tmux TMUX --pane PANE] --fix FIX [--sibling 1] HOST:PORT…` (bwrap's command; `prepare` writes it in
  `container.env`), in the run folder, before the engine. Its socket is `proxy.sock`, relative to its working folder:
  a run folder's path may be longer than `sun_path`'s 108 bytes. Its log is `proxy.log`, made afresh, never through a
  link. Its environment is the caller's `PATH HOME USER LOGNAME LANG TMUX TMUX_TMPDIR XDG_RUNTIME_DIR` and nothing
  else (bwrap passes the whole environment; the caller's secrets never reach this proxy). `run` waits up to 5 s for
  the socket, else refuses with the log's last line. The proxy refuses a host not allowed (403), records it once in
  `denials` (the run's `/run/pot/denials`), and never reaches the host's loopback: a name that resolves to it is
  refused (`LOOPBACK`).
- **In the run.** The run folder's `proxy.sock` is bound read-only at `/run/pot/proxy.sock`, a short path whatever
  the run folder's. The entry's bash (always used with the network) starts the config's `socat` on 127.0.0.1:3128,
  forwarding to that socket, with its streams `/dev/null`, its arguments pot's and its environment bash's own (no
  variable of the run's is declared yet), and waits until it listens (`/proc/net/tcp`, as bwrap's `netwrap`), then
  declares the variables and execs the command. socat is the command's child, as under bubblewrap; when the command
  ends, the engine's init (PID 1) exits and the kernel ends every other process of the container, socat included, so
  it never keeps the container alive. socat and its closure (`netClosure`) are bound read-only.
- **The variables**, bwrap's `run_env`'s, set last: `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`,
  `ALL_PROXY` and `all_proxy` are `http://127.0.0.1:3128`, and `NO_PROXY` and `no_proxy` `localhost,127.0.0.1,::1`.
  With the network every variable passed by name goes over the named pipe, as with a secret, so the command's
  environment is exactly the plan's.
- **The end.** When the command ends, `run` stops the proxy (SIGTERM, then SIGKILL after 2 s, bwrap's
  `stop_helpers`) and removes its socket. The proxy is `run`'s child and stops itself once its parent is gone
  (`proxy.py`'s `orphaned`, checked each second), so a `run` killed with SIGKILL leaves no proxy behind (tested: gone
  within a second or two); `cleanup` stops one still there.
- **`after`** reads `proxy.log` (a regular file only) as bwrap's does: each `DENY host:port` counted in `denials`,
  each `LOOPBACK host:port address` in `loopback`.

## Bindings

A `credential` or `mcp` binding (policy-1 `bindings[]`) is carried out as bwrap.py carries it out (`credentials`,
`deliver`, `mcp_servers`; `bindings.rs`). A binding holds references only, and so do `prepare`'s files.

- **The values, at run time.** Before it starts anything, `run` reads each binding's variables on the host, from its
  `from`: `file` or `envFile`, a key file of `NAME=VALUE` lines (`export NAME=VALUE`, `#` comments, one pair of quotes
  stripped; a leading `~` or `~/` is the caller's `HOME`; `~user/` is not expanded, so such a path is refused as missing, where
  bwrap's `expanduser` would read it), or for a `credential` of one variable with no such line in `file`, the whole
  file, stripped (a token file); `env`, a host variable of another name (a binding of one variable); nothing, the
  host's variables of the same names. A variable the host has unset is left out. A key file that can't be read, has
  no `NAME=VALUE` line where one is needed, or lacks a variable (but `AWS_CREDENTIAL_EXPIRATION`) stops the run before
  the engine starts, with bwrap's message, which names the binding and the file and never the value: `the credential
  binding SELECTOR reads FILE: No such file or directory`. So does a via-file mcp key with a quote, a backslash or a
  control character (below), and a NUL in a value that goes as a variable (bwrap would pass it truncated).
- **`via: env`.** The value goes over the named pipe as the variable's record, as a secret's does: never in a file, an
  argument or the engine's environment. Every run with a binding uses the pipe, under both engines.
- **`via: file`.** `prepare` makes the run folder's `secrets/` (mode 0700, emptied) and binds it read-only at
  `/run/pot/secrets`, as bwrap binds its own. `run` opens it without following a link (it must be the caller's, 0700)
  and writes each value to `NAME` there, created exclusively (`openat` with `O_EXCL|O_NOFOLLOW`, whatever was at the
  name removed first), mode 0400, owned by the caller, which is the run's user under both engines (podman's
  `keep-id`, docker's `--user` with no user namespace). Just before the engine starts it checks the folder again: the
  caller's, 0700, holding exactly those files, each a regular file of the caller's, 0400. The variable's record is the
  file's path in the run, `/run/pot/secrets/NAME`. The run reads the file, and can't write, replace, `chmod` or remove
  it, or add one beside it (a read-only bind; every process of the run is its user, with no capability). `run`
  removes `secrets/` when the command ends and on every early return after it writes the files (a run that stops
  reading a value leaves the empty folder `prepare` made, holding nothing, for `cleanup`); after a `run` killed with SIGKILL, `cleanup`
  removes it, and so does the next `prepare`, before anything else, even when it then stops. This is the one file
  policy-1 rule 6 allows.
- **Precedence**, as bwrap's `run` (`env.update(deliver(credentials()))` after `run_env`): a binding's variable beats
  pot's values, `env.set`, the items and the proxy's variables. So a binding's variable is never set by the entry's
  last hop: it goes over the pipe, and when the binding has no value at run time, its record is what the plan would
  have given it (the host's value when it is also an env item, `TERM` or `COLORTERM`, else pot's value, else unset).
- **`mcp.json`** is bwrap's, byte for byte (`json.dump(…, indent=2)`, ASCII only): `{"mcpServers": {SERVER: {"type":
  "http", "url": URL, …}}}`, written by `prepare` for every run (`{"mcpServers": {}}` without an mcp binding) and bound
  read-only at `/run/pot/mcp.json`. With `via: env`, `headers` holds `HEADER: "SCHEME ${NAME}"`: the client expands the
  variable, and no value is in the file. With `via: file`, a `headersHelper`, `read -r v < "$NAME"; printf '%s%s%s'
  '{"HEADER": "SCHEME ' "$v" '"}'`, reads the key with shell builtins and prints the header; `run` refuses a key with a
  quote, a backslash or a control character, which that JSON could not carry (bwrap's message, `… use via = "env"`).
- **Shown**, as bwrap's `print_command`: `prepare --print` names each binding's variable, a via-file one with its
  file's path, another as `(the run holds it)`.
- **Names.** `prepare` refuses a binding's variable bash can't hand over (`engine::bash_sets`, as for an env item) or
  that is not a `NAME`. The engine list (`engine::configures_engine`) does not apply: no binding's variable reaches
  the engine.

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

- write access when launch.json has no `pins` for a workspace (a pot before 0.3.0), or the config names no `python`,
  `pins` and `git`;
- clone mode: the clone at its original path, its pins, proposal links, and the `after` staging fetch;
- the network, and `private-proxy`, when the config's `python`, `proxy`, `socat` and `netClosure` do not pass
  `probe`'s test (above);
- the `status.relay`, `git.fetch` and `nix` bindings (the broker, the nix relay);
- a children table with a `sibling` row (the run's children broker);
- inspect's `view`;
- `dir` items (folders of links) and `file` items in mode `seed` or `reassert`;
- `resource-limits` under docker;
- a path a bind can't name (`:`, `,`, `"` or a control character, such as a newline: X5), a sealed entry whose name is not
  UTF-8 (named), and a command whose name holds `=` (`env` would read it as a variable);
- a run's variable that configures the container engine, or that bash can't hand over (above); a binding's variable
  that bash can't hand over, or that is not a `NAME`;
- a path that is not its real path, and a bind source that does not exist (above).

It stops the run, as bwrap does and not as a gap, on a pin pot's core could not make (launch.json's `refused`: a
link, a hard link), with its message, and on a pin that changed since ([Write access](#write-access)).

A decoding error names where it is and what was expected there (`items[2].effective: expected a boolean`), never the
value found: a secret misplaced into a field is never printed. `prepare` can run again: it replaces its files, never
writing through a link planted in the run folder, and names the file it could not write.

Not placed yet, which leaves the run narrower than bubblewrap's, never wider: `/run/pot/README.md`, `explain.txt`,
`why.json`, `bin/` (the shims), `notfound/` and `bash_env` (the not-found advice), `~/.gitconfig`, and
`/run/current-system`. So the command's environment is bwrap's but for `BASH_ENV`, the exported not-found handler, and
`/run/pot/bin` and `/run/pot/notfound` on `PATH` (tested against bwrap.py's own `run_env` and `deliver`). With read
access a sealed folder stays plainly read-only, as the whole workspace is (bwrap seals it there too); with write
access it is sealed as bwrap seals it ([Write access](#write-access)). The hidden-path start notes are given for
write runs only.

## What's next

1. The other bindings: `status.relay`, `git.fetch`, `nix` (the broker and the nix relay); then siblings.
2. Clone mode: the clone at its original path, its pins (the same `core/pins.py`), `after` and `cleanup` for clones;
   the conformance review with executed evidence per guarantee, write access included (plan step 4).
3. Closure delivery as an image built from a root's closure (plan step 5), for hosts without `/nix/store`.
4. pot choosing the backend from a root, probing it for every run.

## Sources

- [backends/container/src/](../../backends/container/src/): `policy.rs` (reading policy.json/1), `probe.rs`,
  `render.rs` (the flags), `bindings.rs` (the credential and mcp bindings), `lifecycle.rs` (run's and cleanup's
  commands, the after event), `main.rs` (the verbs)
- [backends/container/tests/run.rs](../../backends/container/tests/run.rs): real containers, when `POT_TEST_PODMAN`
  or `POT_TEST_DOCKER` names the engine (rootless podman 5.4.1 and docker 27.5.1 on platform-vm, 2026-11-21): `true`
  and `exit 3` with no engine process or container left at return, SIGTERM stopping the run (143), a killed run's
  container, store and subuid-owned files removed by `cleanup`, and `cleanup` run twice
- The run slice's conformance audit (2026-11-21, flexion-assistant session record
  `2026-11-21-pot-phase-2/container-qa-2.md`) and the fixes for its findings R1-R6, each with a test in `run.rs`: an
  early SIGTERM leaves no container (podman and docker); a link planted after `prepare` is refused by `run`; `cleanup`
  keeps the store when it could not remove the container; a secret's value in no file or engine process; a docker
  image with a configuration refused; a bystander process naming the store not waited for; a mode-000 folder of
  another id removed through `podman unshare`, or named
- This slice's tests in `run.rs` (rootless podman 5.4.1 and docker 27.5.1 on platform-vm, 2026-11-22), under both
  engines: a secret arrives exactly over the named pipe and its marker is in no file of the run folder or
  `$XDG_RUNTIME_DIR/crun`, not in the engine's `container inspect`, and in no exec's arguments (`strace -f -e
  execve` of `run`, which under podman follows into the container's bash and its last hop); a link planted at the
  pipe's path is removed, never written through; a container that never opens the pipe does not hang `run`; with the
  network, `https://example.com` (allowed) answers 200, `https://example.org` is refused (403) and is in `denials` and
  the after event, a direct connection fails, `localhost:PORT` through the proxy is refused as the host's loopback
  (`loopback` in the after event) and the host's listener on 127.0.0.1 is never reached, `/proc/net/dev` shows only
  `lo`; the command's environment with the network is exactly the plan's; the proxy is gone within seconds of a
  SIGKILL of `run`; `cleanup` stops a live proxy and removes its socket
- The bindings slice's tests (rootless podman 5.4.1 and docker 27.5.1 on platform-vm, 2026-11-22), under both
  engines, with fake key files: a marker from each `from` kind (key file, `envFile`, a host variable, the same-named
  one) arrives in each `via` mode; no marker is in a file of the run folder (but its via-file file) or of
  `$XDG_RUNTIME_DIR/crun`, in `container inspect`, in `container.argv`, or in any exec's arguments (`strace -f -e
  execve`); the via-file file is 0400 and the caller's, inside and out, and the run can't write, replace, `chmod` or
  remove it, or create one beside it; `secrets/` is gone when the command ends, after a SIGKILL of `run` then
  `cleanup`, and after a `prepare` that stops (a link planted there is removed, not followed); `mcp.json` has bwrap's
  shape with no value; a via-file mcp key with a quote, and a missing key file, are refused before the engine starts,
  naming the binding and file; the command's environment equals what bwrap.py's `run_env` plus `deliver` computes for
  the same policy (but for what is not placed yet, above)
- The write slice's tests in `run.rs` (rootless podman 5.4.1 and docker on platform-vm, 2026-11-22), under both
  engines, with the pins resolved by `core/pins.py` as pot does: the workspace is writable and a commit works; a
  write, `rm`, `mv` and a rename over each of `flake.nix`, `sub/flake.nix`, `AGENTS.md`, `.git/config`, `.git/hooks`,
  `.git`, the placeholders and a sealed entry fail, so do `mv sub` (a pin's folder), `git config` and a write or
  `mkdir` outside the workspace; a sealed folder's new entry stays in the run folder and is in `sealed_left`; a
  created `missing.md` (protected, absent at the start) is in `touches_host_run`, as under bwrap; in a worktree, the
  common `config`, `hooks`, the worktree's git directory and its `.git` file hold and a commit works; a link on a
  pin and a hard-linked protected file stop `prepare` with bwrap's message and place nothing; a pin replaced, rewritten
  in place, linked or hard-linked after `prepare` stops `run` before the engine starts; after the QA audit
  (2026-11-22, X1-X6, Y1-Y6): a pin swapped for a link while `run` starts is never used (12 to 50 tries under each
  engine), nor is a mask swapped away, with read or write access (15 tries each), a pin path ending with a space is
  checked as it is,
  a pin the container does not see as recorded stops the entry (125) before the command, the command's environment
  gains nothing from the check, and the look's environment holds no caller variable but its own list
- [core/pins.py](../../core/pins.py): `Pins`, `resolve_workspace` (the pins), `before_look`, `look_after` (the look)
- [core/bwrap.py](../../core/bwrap.py): `prepare`, `run_env`, `credentials`, `deliver`, `mcp_servers`, `masked`, `Pins`, `cleanup`, `run` (the proxy),
  `after` (denials and loopback) (the reference backend); [core/proxy.py](../../core/proxy.py)
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
