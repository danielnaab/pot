---
status: draft
---
# pot-container probe

`pot-container [--config CONFIG] probe RUNDIR` prints the guarantees this host's engine proves, as pot's backends
do ([policy.json/1, For backend authors](../../../contracts/policy-1.md#for-backend-authors)): `{ "guarantees":
[ … ] }`, or none beside an `error`. It always exits 0; pot refuses the run when the list is empty. The engine is the
config's `engine`, else podman when it is on `PATH`, else docker. See [the container backend](../container.md#probe)
for the checks.

These transcripts run in `probe.in/`, where `fake-docker` prints what docker 27.5.1 says of a host with the memory
and pids controllers and its default seccomp profile:

```console
$ pot-container --config docker.json probe .
{"guarantees":["closure-store","own-netns","own-pidns","no-tiocsti","local-listen","in-place","root-files"]}

```

docker never lists `resource-limits`: a docker container is not in pot's scope, so `prepare` refuses it there. A
daemon whose seccomp profile is `unconfined` proves nothing:

```console
$ pot-container --config unconfined.json probe .
{"error":"docker has no seccomp filter here","guarantees":[]}

```

An engine that is not there, or one this backend does not know:

```console
$ pot-container --config missing.json probe .
{"error":"podman: can't run ./no-such-podman: No such file or directory (os error 2)","guarantees":[]}

$ pot-container --config lxc.json probe .
{"error":"the backend's --config names engine [..]

```

The rules for rootless podman (never root, the caller's NoNewPrivs, its seccomp filter, its version) are unit
tests (`probe::tests`), since their answers depend on the host that runs the tests.

`[..]` in an expected line matches the rest of it (the transcripts' runner reads a backslash as a path separator,
so a line with one is cut there).

## Sources

- [backends/container/src/probe.rs](../../../backends/container/src/probe.rs): `assess`, `facts`
- The phase 2 spike (flexion-assistant session record `2026-11-21-pot-phase-2/spike.md`, executed on platform-vm,
  2026-11-21): NoNewPrivileges breaks rootless podman's `newuidmap`; the controllers resource limits need
