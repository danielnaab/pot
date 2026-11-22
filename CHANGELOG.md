# Changelog

pot's releases, from 0.1.0. Below 1.0, each `0.y` series is its own major: a `0.(y+1)` may change a contract,
and a patch never does. Until 0.1.0 pot shipped inside graft; its history up to graft v2.21.x is in
[graft's CHANGELOG](https://github.com/danielnaab/graft/blob/v2/CHANGELOG.md) (the entries that name pot, from
v2.14.0's `sandbox` on).

## Unreleased

### Added

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
