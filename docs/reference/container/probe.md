---
status: draft
---
# pot-container probe

`pot-container [--config CONFIG] probe RUNDIR` prints the guarantees this host's engine proves, as pot's backends
do ([policy.json/1, For backend authors](../../../contracts/policy-1.md#for-backend-authors)): `{ "guarantees":
[ … ] }`, or none beside an `error`. It always exits 0; pot refuses the run when the list is empty. The engine is the
config's `engine` (else the one whose program it names), run by the program the config names, never one on `PATH`. See [the container backend](../container.md#probe)
for the checks.

These transcripts run in `probe.in/`, where `fake-docker` prints what docker 27.5.1 says of a host with the memory
and pids controllers and its default seccomp profile:

```console
$ pot-container --config docker.json probe .
{"guarantees":["closure-store","own-netns","own-pidns","no-tiocsti","local-listen","in-place","root-files"]}

```

docker never lists `resource-limits`: a docker container is not in pot's scope, so pot never claims it for a run
there (and `prepare` refuses a policy that does). A
daemon whose seccomp profile is `unconfined` proves nothing:

```console
$ pot-container --config unconfined.json probe .
{"error":"docker has no seccomp filter here","guarantees":[]}

```

An engine that is not there, or one this backend does not know:

```console
$ pot-container --config missing.json probe .
{"error":"podman: can't run [CWD]/no-such-podman: not found","guarantees":[]}

$ pot-container --config lxc.json probe .
{"error":"the backend's --config names engine [..]

```

The engine's program is always the config's, an absolute path: pot's root names it (its `pkgs.podman`, or the
docker program it was given). The backend never runs an engine it finds on `PATH`, where a repository's `bin`, or a
run, could put a program of its own (QA B2):

```console
$ pot-container --config no-program.json probe .
{"error":"the backend's --config names no podman program: give its absolute path (pot-container never runs an engine it finds on PATH)","guarantees":[]}

$ pot-container --config relative.json probe .
{"error":"the backend's --config names docker [..]

$ pot-container --config empty.json probe .
{"error":"the backend's --config names no engine: podman or docker, with its program's absolute path (pot-container never runs an engine it finds on PATH)","guarantees":[]}

```

The rules for rootless podman (never root, the caller's NoNewPrivs, its seccomp filter, its version) are unit
tests (`probe::tests`), since their answers depend on the host that runs the tests.

`[..]` in an expected line matches the rest of it (the transcripts' runner reads a backslash as a path separator,
so a line with one is cut there).

## Sources

- [backends/container/src/probe.rs](../../../backends/container/src/probe.rs): `assess`, `facts`
- The phase 2 spike (potting-shed session record `2026-11-21-pot-phase-2/spike.md`, executed on platform-vm,
  2026-11-21): NoNewPrivileges breaks rootless podman's `newuidmap`; the controllers resource limits need
