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

**`launch.json`**, until it has a contract of its own (launch.json/1), is read leniently, and only these fields:

| Field | Used for |
|---|---|
| `argv` | The command |
| `cwd` | Where it starts (`env -C`); the home under `access: none` when it is in a workspace |
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

## Policy to flags

| Policy | podman | docker |
|---|---|---|
| (always) | `run --rm --interactive --log-driver none --cidfile RUNDIR/container.id --hostname pot --ipc private --cgroupns private` | the same |
| (always) | `--cap-drop all --security-opt no-new-privileges --read-only`, the engine's default seccomp profile | the same |
| the run's user (`launch.json` `uid`, `gid`) | `--userns keep-id --user UID:GID --passwd=false` | `--user UID:GID` |
| the root | `--rootfs RUNDIR/rootfs:O` (an empty folder, an overlay so nothing is written to it); options before it | an empty imported image, `--pull never` |
| the working directory | `/usr/bin/env -C CWD -- ARGV` (podman takes no `-w` with `--rootfs`) | the same |
| `/tmp`, `/run`, `HOME` | `--tmpfs`; the home `mode=0700,U` | the home `mode=0700,uid=UID,gid=GID` |
| no effective network reach item | `--network none` | the same |
| `closure-store` proved | `--volume P:P:ro` for each path of `launch.json` `closure` and the config's base closure | the same |
| `closure-store` not proved | `--volume /nix/store:/nix/store:ro` | the same |
| `/bin/sh`, `/usr/bin/env`, the CA bundle, `/etc/passwd`, `/etc/group`, `/etc/hosts` | read-only binds | the same |
| `access: read` (in place) | `--volume PATH:PATH:ro`, and `common` when it is outside | the same |
| `access: none` | no workspace at all (decided 2026-11-21) | the same |
| `read`, `write` items | `--volume PATH:PATH:ro` or `:rw`, where the path exists and is not hidden | the same |
| `socket` items | `--volume PATH:PATH:rw`; every other socket under a bound folder or the workspace is masked | the same |
| `hidden` items | under a bound folder or the workspace: a read-only bind of an empty folder (or file) over it; nothing is bound inside a hidden folder | the same |
| `mounts.state` | `--volume DIR:HOME/AT:rw` (`prepare` creates `DIR`) | the same |
| `file`, mode `fixed` | `--volume SOURCE:PATH:ro` | the same |
| `env.set` | `--env NAME=VALUE` (`~` as the home) | the same |
| `env` items, `TERM`, `COLORTERM` | `--env NAME`: the engine reads the value from its own environment, so no value is in an argument | the same |
| pot's variables | `HOME USER LOGNAME SHELL PATH LANG TERMINFO_DIRS SSL_CERT_FILE NIX_SSL_CERT_FILE TZDIR POT POT_POLICY POT_RUN PS1`, `POT_TOOLS`, `POT_CLONE` (bwrap's `run_env`) | the same |
| `mounts.run` | `/run/pot/policy.json` and `/run/pot/denials`, read-only | the same |
| `mounts.launcher` | `POTBIN/pot` at `/run/pot/launcher`; `~/.config/pot/launcher` links to it | the same |
| `resource-limits` proved | `--cgroups split` | refused |

A mask is never a `--tmpfs`: podman mounts every tmpfs before every bind, so a tmpfs below a bound folder is covered
by that folder (a smoke run showed the hidden folder's contents). The engines order binds by destination.

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
- a path a `--volume` can't name (`:` or `,`), and a command whose name holds `=` (`env` would read it as a
  variable).

Not placed yet, which leaves the run narrower than bubblewrap's, never wider: `/run/pot/README.md`, `explain.txt`,
`why.json`, `bin/` (the shims), `notfound/` and `bash_env` (the not-found advice), `mcp.json`, `~/.gitconfig`, and
`/run/current-system`. Sealed folders are read-only (only read access is carried out, so every workspace folder
is).

## What's next

1. `run`: the docker image (`docker import` of an empty tar), `-t` when stdin is a terminal, the exit status; then
   the proxy over a bound socket, and the bindings.
2. The workspace's pins in Rust, or from pot's core as data (an open question: one implementation of the protected
   paths, not two).
3. `after` and `cleanup` for clones; the conformance review with executed evidence per guarantee (plan step 4).
4. Closure delivery as an image built from a root's closure (plan step 5), for hosts without `/nix/store`.
5. `backend.name` `container` in policy-1's schema, and pot choosing the backend from a root.

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
