---
status: draft
---
# The container backend (pot-container)

pot's third backend, beside bubblewrap and Seatbelt: it carries out a run's [policy.json/1](../../contracts/policy-1.md)
as one `podman run` or `docker run`, a fresh container per run, on any host or CI that has either. It is pot's
first Rust code, a binary of its own behind the policy-1 contract (`backends/container/`, the flake's
`packages.pot-container`). This page is its second slice (pot phase 2, plan step 3): it probes, renders the
command and runs it, for runs in place with read access or none; no root offers the backend yet. The transcripts
that test it are the pages under [container/](container/probe.md).

```
pot-container [--config CONFIG] probe|prepare|run|after|cleanup RUNDIR [--print]
```

| Verb | This slice |
|---|---|
| `probe` | Prints `{ "guarantees": [ … ] }`, or none beside an `error` ([transcript](container/probe.md)) |
| `prepare` | Checks the policy and refuses what it can't carry out; places the run's files and `container.argv` (the command, NUL-separated). `--print` prints the command and writes nothing ([transcript](container/prepare.md)) |
| `run` | Runs `container.argv` with the engine environment `prepare` recorded; exits with the command's status once the container is gone ([Run and cleanup](#run-and-cleanup)) |
| `after` | Prints the run's `after` event: bwrap's, empty (nothing a read-only run can leave) |
| `cleanup` | Removes the run's container and store through the engine, then what `prepare` placed; names every removal it could not make (exit 1) |

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
`secret: true`) never reaches the engine. With one, every variable passed by name (the secrets, `TERM`, `COLORTERM`
and the plain items) goes over a pipe instead. `run` writes `NAME=value` records, each ended by a NUL, then the
sentinel record `.`. The engine passes the pipe's read end on with `--preserve-fd FD`, which keeps its number, and
`run` gives the script that number. No other fd of the caller's reaches the container. The entry starts with
`/usr/bin/env -i --`, which clears what the engine set. Then the run's bash (the config's `shell`) collects the
records up to the sentinel and closes the fd. If the stream is not whole (no pipe, a short read, a record that is not
`NAME=value`), it exits 125 and nothing runs. Otherwise it declares each record in bash (`unset -v NAME`, which drops
a special meaning such as `RANDOM`'s, then `declare -x`). A record is never an argument: an earlier version passed
the records to `env -i`, and the audit read a secret in that process's `/proc/PID/cmdline` (T1). Bash then execs the
last hop, `/usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C CWD -- POT'S VALUES… ARGV`, whose arguments hold only names
and pot's values, which are not secret. The command's environment is then exactly the planned one: pot's values,
`env.set`, the items and the secrets. Nothing of bash's (`SHLVL`, `PWD`, `_`) or the engine's (`HOSTNAME`) is
added, and a secret named like a bash variable (`RANDOM`, `SECONDS`, `IFS`) keeps its value (audit S1, S3, T1). For
this to hold, `prepare` refuses, for every `env` item and under either engine, a name bash can't hand over: read-only
in bash (`UID`, `EUID`, `PPID`, `BASHOPTS`, `SHELLOPTS`), set by bash itself (`SHLVL`, `PWD`, `OLDPWD`, `_`), unset
by it (`PS1`, `PS2`), `BASH_*`, or the script's own `__pot_*` (`engine::bash_sets`). Without a secret, `env -C CWD -u HOSTNAME --` removes the one variable podman adds, unless the plan sets it. The
value is in no file, no argument and not in the engine's environment (checked in a real run: a marker value was in
no file of the run folder or of `$XDG_RUNTIME_DIR/crun`, and in no engine process's arguments or environment, while
the command printed it; and with every process on the host watched while a 100 000-byte variable slowed the
script's reads, no process's arguments ever held a record's value). With a secret, `prepare` refuses a shell that is not a real file under a path the run binds
(S5). docker passes no file descriptor, so `prepare` refuses a secret variable under docker. The run's bash is used
rather than this binary, which would need its own closure (glibc, libgcc) bound, a closure the config does not name. A plain variable may not be one the engine reads as configuration: `prepare` refuses an effective `env` item named `CONTAINERS_*`,
`_CONTAINERS_*`, `CONTAINER_*`, `DOCKER_*`, `PODMAN_*`, `BUILDAH_*`, `STORAGE_*`, `XDG_*`, `DBUS_*`, `LD_*`, `LISTEN_*`, `HOME`,
`PATH`, `TMPDIR`, `NOTIFY_SOCKET`, `REGISTRY_AUTH_FILE`, `SSH_AUTH_SOCK`, or one of Go's runtime variables (`GODEBUG`,
`GOGC`, `GOMAXPROCS`, `GOMEMLIMIT`, `GOTRACEBACK`) (`engine::configures_engine`). For a secret variable under podman
this list is no longer needed, since the engine never sees it: only the names pot sets itself (`HOME`, `PATH` and the
rest of pot's variables) would stay refused. It still applies to secret variables for now; lifting it is a later
change. Plain variables already take the same pipe when a secret is there; with no secret they do not.

`prepare` writes these as `container.env` for `run`: the names kept, the values set, the names that go over the
pipe, and the kind (file or folder) of every file of the run's that the plan binds. It holds no secret's value.
`prepare --print` shows them, and
A root that offers this backend should also hide `~/.config/containers` (and
`~/.docker/config.json`, which pot hides already) from its runs; adding `~/.config/containers` to pot's default
hidden list is a change to pot's core, listed in [What's next](#whats-next).

## Policy to flags

| Policy | podman | docker |
|---|---|---|
| (always) | `--root RUNDIR/enginestore --runroot RUNDIR/engineroot --tmpdir RUNDIR/enginetmp`, before `run` | none |
| (always) | `run --interactive --log-driver none --cidfile RUNDIR/container.id --hostname pot --ipc private --cgroupns private`; no `--rm` ([Run and cleanup](#run-and-cleanup)) | the same, and `--name pot-HASH` (16 hex digits of the run folder's path, FNV-1a) |
| (always) | `--init`: the engine's init is PID 1 (catatonit, or docker's `docker-init`), a read-only bind of the engine's own file; the command is never PID 1, which ignores a signal it has no handler for | the same |
| stdin a terminal | `--tty`, added by `run` | the same |
| a secret `env` item | `--preserve-fd FD`, added by `run`; no `--env NAME`; the entry is `/usr/bin/env -i -- SHELL --noprofile --norc -c SCRIPT pot-secrets FD /usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C CWD -- POT'S VALUES… ARGV` | refused |
| (always) | `--cap-drop all --security-opt no-new-privileges --read-only`, the engine's default seccomp profile | the same |
| the run's user (`launch.json` `uid`, `gid`) | `--userns keep-id --user UID:GID --passwd=false` | `--user UID:GID` |
| the root | `--rootfs RUNDIR/rootfs:O` (an empty folder, an overlay so nothing is written to it); options before it | an empty imported image, `--pull never --entrypoint ""` |
| the working directory | `/usr/bin/env -C CWD -u HOSTNAME -- ARGV` (podman takes no `-w` with `--rootfs`); with a secret, as above | the same |
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
| `env` items (not secret), `TERM`, `COLORTERM` | `--env NAME`: the engine reads the value from its own environment, so no value is in an argument | the same |
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

## Run and cleanup

**`run`** reads `container.argv` and `container.env` (regular files only, never a link). Just before it starts the
engine, it checks that the run folder's engine paths are as `prepare` left them (audit R2), since the engine writes
through a link and binds the run's files. `enginestore`, `enginehome` and `rootfs` (docker: `dockercfg`) must be real
folders. `engineroot`, `enginetmp`, `emptydir` and `potcfg` must be real folders or absent. The run's files must be
regular files or absent. Every file and folder of the run's that the plan binds (`policy.json`, `denials`,
`passwd` and the rest, S4) must still be the kind `prepare` recorded. `container.id` must be absent, and `status` a
file or nothing (an empty folder there is removed). Otherwise `run` refuses, naming each path, and the engine
never starts. It starts the engine with the caller's environment cleared but for the names `container.env`
keeps, then the values it sets. stdin, stdout and stderr are the engine's. It adds `--tty` when stdin is a terminal,
and the secrets' pipe ([above](#the-engines-configuration)). docker's root is the config's image, by default
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

- The container, through the engine (podman's when the run's store is not empty; docker's by the run's name),
  waiting for the engine's processes as `run` does. The engine is never handed a linked path (R2). When
  `engineroot`, `enginetmp` or `enginehome` is not a real folder, or the removal or the wait fails, the engine's
  state is kept whole and named, never deleted under a container that may still run. So it is when `enginestore`
  is not a real folder, and whenever a process of this run's engine is alive, whatever the store is (S2: deleting the
  runroot under a live container left its mounts in podman's namespace, which no later cleanup could remove).
- The engine's state (`enginestore`, `engineroot`, `enginetmp`). What the user can't remove (files of another id of
  the user namespace, an overlay's mode-000 `work/work`) is removed with `podman unshare rm -rf`, run by a podman
  whose own store is a private temporary folder, never the run's.
- The engine's home and what `prepare` placed. It never follows a link planted in the run folder (a
link is removed, not followed) and never empties a folder that should be empty (`rootfs`, `emptydir`). Every
removal it could not make is named in one refusal (exit 1); what it removed stays removed, so it can run again.

Outside the run folder, as with any rootless podman: the OCI runtime's state for a running container
(`$XDG_RUNTIME_DIR/crun/ID`, gone once the container is removed) and the user's podman pause process, which keeps
the user namespace for later podman commands and is shared with them.

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
- a run's variable that configures the container engine, or that bash can't hand over (above), and a secret
  variable under docker;
- a path that is not its real path, and a bind source that does not exist (above).

A decoding error names where it is and what was expected there (`items[2].effective: expected a boolean`), never the
value found: a secret misplaced into a field is never printed. `prepare` can run again: it replaces its files, never
writing through a link planted in the run folder, and names the file it could not write.

Not placed yet, which leaves the run narrower than bubblewrap's, never wider: `/run/pot/README.md`, `explain.txt`,
`why.json`, `bin/` (the shims), `notfound/` and `bash_env` (the not-found advice), `mcp.json`, `~/.gitconfig`, and
`/run/current-system`. Sealed folders are read-only (only read access is carried out, so every workspace folder
is).

## What's next

1. The network (pot's proxy over a socket bound into `/run/pot`, with its 108-byte `sun_path` limit), then the
   bindings.
2. Write access: pot's core resolves the workspace's protected paths and pins and passes them to the backend
   (decided 2026-11-21: one implementation, no port of bwrap's `Pins`); the backend binds them read-only.
3. `after` and `cleanup` for clones; the conformance review with executed evidence per guarantee (plan step 4).
4. Closure delivery as an image built from a root's closure (plan step 5), for hosts without `/nix/store`.
5. pot choosing the backend from a root, probing it for every run; `~/.config/containers` in pot's default hidden
   list (a core change).

## Sources

- [backends/container/src/](../../backends/container/src/): `policy.rs` (reading policy.json/1), `probe.rs`,
  `render.rs` (the flags), `lifecycle.rs` (run's and cleanup's commands, the after event), `main.rs` (the verbs)
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
