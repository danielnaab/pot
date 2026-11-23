# Changelog

pot's releases, from 0.1.0. Below 1.0, each `0.y` series is its own major: a `0.(y+1)` may change a contract,
and a patch never does. Until 0.1.0 pot shipped inside graft; its history up to graft v2.21.x is in
[graft's CHANGELOG](https://github.com/danielnaab/graft/blob/v2/CHANGELOG.md) (the entries that name pot, from
v2.14.0's `sandbox` on).

## Unreleased

### Added

- **The container backend starts siblings** ([container.md](docs/reference/container.md#siblings)): with a sibling row
  and `hostPot`, `run` starts pot's children broker (`siblings.py`) on the host, bound at `/run/pot/children.sock` and
  checked by the entry; the broker stops each sibling at the run's end (12 s), and `cleanup` stops a broker left and
  removes the top run's tree of slots. launch.json's `hookEnv` is set last, as bwrap's. A sibling runs under its
  caller's backend.
- **The container backend, a first slice** (draft, [docs/reference/container.md](docs/reference/container.md)):
  `pot-container`, pot's first Rust code (`packages.pot-container`). It reads policy.json/1 strictly, probes podman
  or docker, and renders a run as one `podman run` / `docker run`, refusing what it does not carry out yet; no
  root offers it. policy.json/1's schema gains the `backend.name` value `container`.
- **The container backend runs a command** (in place, read access or none): `run` executes the prepared command
  (`--tty` when stdin is a terminal, the engine's init as PID 1), exits with its status, and returns only once the
  container is removed and the engine's processes for the run have ended (no `--rm`; audit N7). SIGTERM stops the
  container. podman's store, runroot and temporary files are the run folder's. docker runs from an empty imported
  image, `localhost/pot-empty:1`. `after` prints bwrap's empty event. `cleanup` removes a killed run's container and
  store through the engine (`podman unshare` for subuid-owned files), never follows a link in the run folder, and
  names every removal it could not make instead of ignoring it (audit N6).
- **The container backend's run slice, after its conformance audit (R1-R6):** a secret variable never reaches
  the engine. Under podman it goes over a pipe to fd 3 (`--preserve-fd`), read by the run's bash before the
  command, and is in no file. docker refuses a secret variable. docker's container has the run's name, so an early
  SIGTERM leaves none, and a stop is retried until the container exists. `run` refuses engine paths that are not as
  `prepare` left them (links). `cleanup` never hands the engine a linked path and keeps the store when the container
  could not be removed. docker's image must have no configuration (`--entrypoint ""`, `rm --volumes`). The wait is
  for this run's podman and conmon processes only. `podman unshare` removes whatever the user can't.
- **The container backend's re-audit (S1-S7):**
  - The secrets' pipe is read at its own fd, whatever fds the caller holds, and is closed before the command.
  - The stream ends with a sentinel, and an incomplete one runs nothing.
  - With a secret, the command starts from `env -i`, with exactly pot's planned environment.
  - Without one, `env -u HOSTNAME`.
  - `run` checks every file it binds, and an unexpected `status`, before the engine starts.
  - `cleanup` keeps the engine's state while a process of the run lives or the store is not a real folder.
  - `prepare` refuses, with a secret, a shell the run can't see.
  - A SIGTERM before the engine starts records status 143.
  - The engine processes are matched by the configured program's names.
- **The container backend's third audit (T1, T2):** no secret's value is ever in a process's arguments. The run's
  bash, in an environment `env -i` cleared, declares each record, then execs `env -u _ -u SHLVL -u PWD -u OLDPWD -C
  CWD -- POT'S VALUES… ARGV`. `prepare` refuses an `env` item bash can't hand over faithfully (`UID`, `SHELLOPTS`,
  `SHLVL`, `PWD`, `_`, `PS1`, `BASH_*` and the like). A test that moves the run's store puts it back and cleans up,
  even when it fails.
- **The container backend's final audit (U1, U2):** `PIPESTATUS`, which bash resets after every command, is refused
  as a run's variable. An env item named like one of pot's values (`TZ` from env.set, `LANG`) takes the host's
  value when the host has it set and pot's value when not, as under bubblewrap.
- **The container backend: secrets over a named pipe, and the network.** The run's variables (a secret's among them)
  go over a named pipe in the run folder (`secrets.fifo`, mode 0600, bound read-only at `/run/pot/secrets.fifo`)
  under podman and docker alike, so docker now carries a secret variable; `--preserve-fd` is gone, and no fd of the
  caller's reaches the container. `run` writes the pipe without blocking and never hangs on a container that did not
  read it; the pipe is removed when the command ends and at `cleanup`. An effective network reach item is carried
  out instead of refused: `run` starts pot's `proxy.py` on the host on the run folder's `proxy.sock` (bound at
  `/run/pot/proxy.sock`), the container keeps `--network none`, socat in the run serves it as 127.0.0.1:3128, the
  proxy's variables are bwrap's, `after` reports `denials` and `loopback`, and `cleanup` stops a proxy still running.
  `probe` proves `private-proxy` when the backend's config names `python`, `proxy` and `socat` (new config keys,
  with `netClosure`; no contract change). The dev shell gains socat, curl and strace for the backend's tests. After
  its QA (V1-V3): the entry holds the pipe's stream to the planned names, given as its arguments (one record per name,
  in order, then the sentinel), so records a same-user process wrote first never widen the run; probe and prepare
  share one test of the network's programs (socat a real file under `netClosure`); the pipe is unlinked from the run
  folder once the records are delivered.
- **The container backend carries out the `credential` and `mcp` bindings**, both `via` modes, under podman and
  docker, as bubblewrap does. `run` reads each value on the host when it starts, from the binding's `from` (a key
  file, a host variable, or the host's variable of the same name), and hands it over the named pipe, which every run
  with a binding now uses; a binding's variable beats every other value of the same name, as under bubblewrap.
  `via: file` writes the value to the run folder's `secrets/NAME` (0400, the caller's, created exclusively without
  following a link), bound read-only at `/run/pot/secrets`, and removes it when the command ends; `prepare` and
  `cleanup` remove it too (after a `run` killed with SIGKILL). `/run/pot/mcp.json` is bubblewrap's, bound read-only
  for every run, and holds no value; `run` refuses a via-file mcp key with a quote, a backslash or a control
  character. The other bindings (`status.relay`, `git.fetch`, `nix`) are still refused.
- **The container backend carries out write access in place**, under podman and docker, from launch.json's `pins`
  (pot's core resolves them; no port of bwrap's `Pins`). The workspace (and a worktree's common directory) is bound
  writable, each git directory and each folder above a pin writable onto itself, and each pin read-only onto itself
  over it, so a pin can't be written, renamed, removed or replaced, nor moved with its folder, and `git config`
  fails while a commit works. `prepare` creates the placeholders (`cleanup` removes them), seals the sealed folders
  as bwrap does, stops on a link or hard-link refusal with bwrap's message, and records each pin's inode; `run`
  refuses a pin that changed since, before the engine starts. `after` reports bwrap's in-place write facts (commits,
  host-run changes, new git metadata, hidden paths committed empty, sealed leftovers) by running core's `pins.py`.
  New config keys `pins` and `git` (with `python`); `probe` proves `protected-as-mounts` when they are named. Clone
  mode, `git.fetch`, `nix`, `status.relay` and siblings are still refused.
- **The container backend's write access, after its audit (X1-X6):** the run's entry checks, inside the container and
  before the command, that every pin is a mount point of the inode `prepare` recorded, so a pin's folder swapped
  for a link while the engine starts is never used (exit 125, naming the pin; the engines can't bind from an open
  descriptor). `run`'s check also compares a file's size and status-change time. bubblewrap's `prepare` checks each
  resolved pin again (a link, a hard link or another kind since pot resolved it stops the run, as before 0.3.0).
  `pins.py` runs in an environment of its own (no caller variable but its list, no `GIT_*`). A path with a control
  character, or a sealed entry whose name is not UTF-8, is refused at `prepare`. `after` takes a missing
  `before.json` as an empty look, as bwrap does.
- **The container backend's check covers every bind under a bound folder (its re-audit, Y1-Y6):** with read access as
  with write access, the entry checks each mask (a hidden path's, a socket's), socket, `read` or `write` item, state
  folder and seal under a bound host folder, as it checks the pins (`/run/pot/binds.txt`), so a mask swapped away
  while the engine starts never shows the hidden path. The 125 message names the mount found there (its source and
  root). A path ending with a space is checked as it is. bubblewrap's re-check names a link above a pin as core does.
- **The container backend carries out the `status.relay`, `git.fetch` and `nix` bindings**, under podman and docker,
  as bubblewrap does. `run` starts pot's broker (status.relay, git.fetch) and the nix relay (socat running `nix daemon
  --stdio --force-untrusted`) on the host in the run folder before the engine; each socket (`relay.sock`, `git.sock`,
  `nix.sock`) is bound read-only at the binding's `/run/pot/` path, and the container keeps `--network none`. The run
  gets bwrap's shims (`workmux`, `pot-ssh`) in `/run/pot/bin`, first on `PATH`, bwrap's `GIT_SSH_COMMAND`,
  `GIT_SSH_VARIANT`, `NIX_REMOTE` and `NIX_CONFIG`, nix's `bin/` on `PATH`, and with nix the whole store read-only.
  The helpers get environments of their own (the broker also `SSH_AUTH_SOCK`), never the run's or the engine's; links
  planted at their sockets, script or log are removed, never followed. When the command ends `run` clears the
  caller's workmux pane and stops the helpers; `cleanup` stops what a `run` killed with SIGKILL left (the relay and
  its daemons never stop by themselves) and removes the sockets, the script and `bin/`. New config keys `broker`,
  `shim` and `shimClosure` (bwrap's names, with `python`; nix needs `socat`); a missing `hostTools.ssh` or `.nix`
  stops `prepare` with bwrap's message; `closure-store` with nix is refused. Siblings and clone mode are still
  refused.
- **The container backend's relays, after their conformance review (A1-A5):** the run-folder files the relays bind
  are checked as the pins are: `prepare` records `bin/` and each shim (one link), `run` refuses one changed since
  and records each socket it checks, and the entry's in-container check stops the command (125) when the engine bound
  another file there (a socket swapped for a link while the engine starts, which the review won 11 of 15 times under
  podman and 10 of 10 under docker, never reaches the run now). git.fetch's key file's `~` is the caller's `HOME`, as
  under bubblewrap. `cleanup` stops a helper only when its program is the config's python or socat or `hostTools.nix`,
  so a decoy with the same words is left alone. The broker keeps `KRB5CCNAME`, the nix relay `NIX_DAEMON_SOCKET_PATH`,
  `NIX_STATE_DIR` and `NIX_STORE_DIR`. `prepare --print` stops as `prepare` does when a binding's host program is
  missing.

### Changed

- **`~/.config/containers` is in pot's default hidden list**, next to `~/.docker/config.json`. podman reads
  `containers.conf`, `mounts.conf` and `storage.conf` there. A run that could read or change them could widen a
  later container run. policy.json/1's fixtures hold the new item.
- **pot's core resolves the workspace's pins, once, and passes them to the backend** (a contract change: 0.3.0).
  For a run with write access in place, `core/pins.py` (bwrap.py's `Pins`, moved) lists what to bind writable and
  read-only onto itself, the placeholders, the sealed folders, and a link or hard link that stops the run, and pot
  writes it as launch.json/1's new optional `pins` (prose, schema and fixtures). bubblewrap binds that list instead
  of resolving its own, with the same result; Seatbelt resolves its own with the same code. Every check now comes
  before any placeholder, so a refused run leaves none. `pins` is the one field added within launch.json/1: no reader
  runs wider for not knowing it (bubblewrap and Seatbelt are built with pot; an older container backend refuses
  write access and ignores the field). `pins.py before|after` is the host's look (bwrap's before.json and the
  in-place part of its after event) for a backend built apart.

### Fixed

- **Two runs starting at once in worktrees of one repository no longer collide** on its git directory's S1
  placeholders (`commondir`, `config.worktree`, `gitdir`). One stopped with "… appeared after pot resolved the
  workspace's pins: run again", or the first to end removed a placeholder the other still ran with. pot now places and
  removes them under the git directory's lock (in pot's state); a run takes on a placeholder another placed, and the
  last run out removes it (a start that found one the last run out removed meanwhile places it again). Each run lists
  what it relies on in `pins-used.json`, held while its pot lives, so a run killed outright counts for nothing: the
  next start takes its placeholders on, or removes them. A placeholder a run pot v0.2.0 started placed (listed in its
  `placeholders.json` alone) is taken on too, so its cleanup leaves it while a run relies on it. On every backend
  (Seatbelt untested here).
- **`pot --backend NAME`'s cleanup is that backend's** (and a context's `backend`), not the root's default.

### Upgrading

1. **End every run pot v0.2.0 started before switching** (`pot runs` lists them; let each end, or stop it). A v0.2.0
   pot that ends after the switch removes its S1 placeholders as v0.2.0 did, without asking whether a newer run in a
   worktree of the same repository took one on. That run has it bound read-only. Removed on the host, its bind is
   gone too (Linux detaches a mount whose file is removed in another namespace), so the run could create
   `config.worktree` or `commondir` there and point host git at a config it wrote. A v0.2.0 run that is lost (its pot
   killed) is safe: the newer pot's cleanup of it leaves what a live run relies on.

## v0.2.0 - 2026-11-21

### Added

- **policy.json/1, a contract** ([contracts/policy-1.md](contracts/policy-1.md)): a run's `policy.json` as prose, a
  JSON Schema (2020-12) and fixtures, for a backend built apart from pot. It says which fields a backend must
  enforce and which are for explanations, the backend's verbs, and what a backend must refuse rather than run
  weaker. pot writes exactly what 0.1.0 wrote: `tests/run.sh` validates every `policy.json` its runs write, and
  `pot explain --json`.
- **launch.json/1, a contract** ([contracts/launch-1.md](contracts/launch-1.md)): the launcher's facts beside
  policy.json, as prose, a JSON Schema (2020-12) and fixtures: which fields a backend must use, which are pot's own,
  and the keys a backend's `prepare` adds. pot writes exactly what 0.1.0 wrote: `tests/run.sh` validates every
  `launch.json` its runs write.

### Changed

- **run-env/1 no longer lets a release add a key to a policy item** within version 1: a new field, an item's key
  included, is a new version (policy.json/1). pot 0.1.0 added none.

### Fixed

- **`access = "none"` is no workspace at all**, in both backends: neither the repository nor its git directory is
  in the run, even under a folder a read request binds (masked, or denied under Seatbelt), and the command starts
  in the run's home. 0.1.0 treated `none` as `read`, so a context with `access = "none"` saw its workspace
  read-only. A clone context with `access = "none"` is refused, and `pot explain` and `pot why` say there is no
  workspace. Siblings: a run with `none` covers no read inside the workspace.
- **A `credential` or `mcp` binding with `via = "file"` is honoured.** The value goes to `/run/pot/secrets/NAME`,
  a file of the run's own (mode 400, read-only to the run, removed when it ends and at cleanup), and the variable
  holds that path; an `mcp` server's header comes from a `headersHelper` that reads the file. 0.1.0 passed the
  value as the variable whatever `via` said. **Breaking for a binding that says `via = "file"` for a program that
  reads the key from its variable** (Claude's `CLAUDE_CODE_OAUTH_TOKEN`, pi's keys, a gateway key, as earlier
  examples did): set `via = "env"` (or drop `via`), keeping `from.file`. The examples now do.

## v0.1.0 - 2026-11-21

pot in its own repository, extracted from graft v2.22.0 with its history. It reads and writes exactly what graft
v2.21.0's pot does: the same kit/1, pot-lib/1, pot-kits/1, grants/1 (the same canonical items and digests, so every
grant still applies), run-env/1 and runs/1, and the same state folder. A root can move to it and back
([Release checks](docs/development.md#release-checks)).

### Upgrading from graft's pot

In a root (`~/.workstation`, or a flake built with `graft.lib.mkRoot`):

1. Take pot as an input in place of graft:
   `inputs.pot.url = "git+https://github.com/danielnaab/pot?ref=refs/tags/v0.1.0&exportIgnore=1";` (or
   `github:danielnaab/pot/v0.1.0`), and `nixpkgs.follows = "pot/nixpkgs"` where you followed `graft/nixpkgs`.
2. Rename what you take from it: `graft.lib.mkRoot` → `pot.lib.mkRoot`, `graft.kits.claude-code` →
   `pot.kits.claude-code`, `graft.kits.pi` → `pot.kits.pi`, `graft.apis.*` → `pot.apis.*`, and
   `inputs.graft.homeManagerModules.pot` → `inputs.pot.homeManagerModules.pot` (or `.default`). The options under
   `programs.pot` are unchanged.
3. Build and switch (`home-manager switch`). `pot doctor` prints `pot 0.1.0`. No grant is asked again.

To go back, revert the input and switch again: graft v2.21.0's pot reads everything 0.1.0 wrote.

A repository's own `pot-kits` keeps working as it is: a bundle that graft's mkFlake built is read the same way. A
repository moves to tuteur on its own schedule, and then declares `inputs.pot` for its bundle
([pot-lib/1](contracts/pot-lib-1.md)).

### Changed

- **pot's own flake.** `lib.mkRoot`, `lib.pot.kitsFor` (the path pot-lib/1 promised), `kits.claude-code`,
  `kits.pi`, `apis`, `homeManagerModules.pot` and `homeManagerModules.default`. `packages.default` and
  `apps.default` are pot with its default root (`nix run github:danielnaab/pot -- CMD`), also named `pot`.
- **Messages name pot.** `pot.lib.mkRoot: …`, `pot.kits.claude-code: …`, `pot.kits.pi: …`, `pot.apis.gateway: …`;
  the in-run README says "pot"; a darwin root built before Seatbelt's return is rebuilt "with this pot"; a wrapper
  under a root too old for wrapped commands says `take pot 0.1.0 or later in your root (home-manager switch)`.
  The refusal of a `graft-kits`-only flake still names graft, which wrote those bundles.
- **"submodule kit"** in messages and prose for what graft called a scion kit: `pot explain`'s list is
  `submodules:`, `pot doctor` says `no submodule kits`, and `pot review --fetch` names "that submodule's refs".
  Field names stay in the v1 contracts (`scions` in pot-kits/1's `kit.json`, runs/1's `after.scions` and
  `clone.scionCommits`).
- **`pot init`** writes a root on `inputs.pot` (`pot.lib.mkRoot`, `pot.kits.claude-code`,
  `nixpkgs.follows = "pot/nixpkgs"`), pinned to this release (`github:danielnaab/pot/v0.1.0`, from pot's version,
  so it can't fall behind as v2.21.0's `graft/v2.20.0` had). `POT_INIT_URL` replaces `POT_INIT_GRAFT`.
- **pot's version is its own**, set once in `core/pot.py` (`VERSION`); `pot doctor` prints it.
- **`kits.just`'s header** (pot-kits/1) names no producer:
  `# Generated from the kits' recipes (pot-kits/1) by the bundle's producer, which writes and compares it:`. A
  producer that compares the file's bytes (tuteur's check) writes it again with its sync. The wrappers' first
  comment says `pot (pot-kits/1)`.

### Added

- **A repository with submodules and a flake but no `pot-kits`** gets one line at start, on stderr, whatever
  `POT_QUIET` says: `pot (CONTEXT): DIR's flake has no pot-kits; its own kits and protections are not applied`.
- **[trailer/1](contracts/trailer-1.md)**, the `Pot-Run:` trailer that `pot trailer` has written since graft
  v2.21.0, as a contract: prose, a schema and fixtures, which tuteur's `ci signers` reads.
- **[pot-lib/1](contracts/pot-lib-1.md) is open to any producer**, with a producer of your own as its example.
- **Release checks**: the golden grant test against graft v2.21.0's `golden.json`, byte for byte, and
  `tests/run.sh rollback` ([Development](docs/development.md#release-checks)).

### Kept for 0.1.x

- `GRAFT_SANDBOX*` stays refused in kits' `env.set` and `--env` (graft v2.14.0's names); 0.2.0 drops the check.
