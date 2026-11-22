---
status: draft
---
# pot-container prepare

`pot-container --config CONFIG prepare RUNDIR` reads the run's `policy.json` ([policy.json/1](../../../contracts/policy-1.md))
and `launch.json`, refuses what this backend does not carry out, and places the run's files; `--print` prints the
engine's command instead and writes nothing (`pot explain --backend-command`). The flags are explained in
[the container backend](../container.md#policy-to-flags).

These transcripts run in `prepare.in/`. `read-only/` is a review context: read access to `~/src/notes`, a read
path, a hidden folder, a state mount, a fixed file, a secret variable passed by name, and `resource-limits` proved
(the run is in pot's systemd scope). `config-podman.json` and `config-docker.json` name the store paths this
backend binds into every run. None of the host paths exist here, so no read path, mask or socket is bound.

## podman

```console
$ pot-container --config config-podman.json prepare --print read-only
# the podman command for this run, an option a line (pot explain --backend-command)
podman
  run --rm --interactive
  --log-driver none
  --cidfile [CWD]/read-only/container.id
  --hostname pot
  --network none
  --ipc private
  --cgroupns private
  --cap-drop all
  --security-opt no-new-privileges
  --read-only
  --userns keep-id
  --user 1000:100
  --passwd=false
  --cgroups split
  --tmpfs /run:rw,nosuid,nodev,mode=0755
  --tmpfs /tmp:rw,exec,nosuid,nodev,mode=1777
  --volume /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/sh:/bin/sh:ro
  --volume [CWD]/read-only/group:/etc/group:ro
  --volume [CWD]/read-only/hosts:/etc/hosts:ro
  --volume [CWD]/read-only/passwd:/etc/passwd:ro
  --tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,U
  --volume [CWD]/read-only/hidden.txt:/home/u/.pot-hidden:ro
  --volume [CWD]/read-only/denials:/run/pot/denials:ro
  --volume /nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin/pot:/run/pot/launcher:ro
  --volume [CWD]/read-only/policy.json:/run/pot/policy.json:ro
  --volume /nix/store/zwplch93wbpj82bp7riwaixmdx2xi0ah-coreutils-9.7/bin/env:/usr/bin/env:ro
  --volume /nix/store/7f6kqbjr4l3qkl1m2s0a9hb3s5z7r0bq-nss-cacert-3.111/etc/ssl/certs/ca-bundle.crt:/etc/ssl/certs/ca-certificates.crt:ro
  --volume /home/u/.local/state/pot/state/9f2e/notes:/home/u/.cache/notes:rw
  --volume [CWD]/read-only/potcfg:/home/u/.config/pot:ro
  --volume /home/u/src/notes:/home/u/src/notes:ro
  --volume /nix/store/0aqd4v7xhp2c9z2lfy5h4v0rdf3q4k8n-config.toml:/home/u/.config/notes/config.toml:ro
  --env HOME=/home/u
  --env LANG=C.UTF-8
  --env LOGNAME=u
  --env NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
  --env NOTES_CACHE=/home/u/.cache/notes
  --env PATH=/nix/store/zwplch93wbpj82bp7riwaixmdx2xi0ah-coreutils-9.7/bin:/nix/store/scikhwasnkiqmjsi9iczrrx59bwbn379-pot-kits/bin:/nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin
  --env POT=review
  --env POT_POLICY=/run/pot/policy.json
  --env POT_RUN=3c1a
  --env POT_TOOLS=/nix/store/scikhwasnkiqmjsi9iczrrx59bwbn379-pot-kits/bin
  --env 'PS1=(pot:review) [..]
  --env SHELL=/nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/bash
  --env SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
  --env TERMINFO_DIRS=/nix/store/1ylb9cjvz6p8pjkl3l4bl6wq0b1hd8xm-ncurses-6.5/share/terminfo
  --env TZ=America/Chicago
  --env TZDIR=/nix/store/3kgf5kq7p5xj6q9q0gk8m9z3xy7a0a7r-tzdata-2025b/share/zoneinfo
  --env USER=u
  --env TERM
  --env COLORTERM
  --env NOTES_TOKEN
  --rootfs [CWD]/read-only/rootfs:O
  /usr/bin/env -C /home/u/src/notes -- notes check --all
# and before the root: --volume P:P:ro for each of 7 store paths (the closure)

```

## docker

docker can't keep a container in pot's scope, so it refuses a run that has `resource-limits`:

```console
$ pot-container --config config-docker.json prepare --print read-only
? 1
pot (review): the container backend does not carry out yet: resource-limits under docker (a container is not in pot's scope)

```

Without it (`access-none/`: no workspace at all, so the command starts in the home):

```console
$ pot-container --config config-docker.json prepare --print access-none
# the docker command for this run, an option a line (pot explain --backend-command)
docker
  run --rm --interactive
  --log-driver none
  --cidfile [CWD]/access-none/container.id
  --hostname pot
  --network none
  --ipc private
  --cgroupns private
  --cap-drop all
  --security-opt no-new-privileges
  --read-only
  --user 1000:100
  --pull never
  --tmpfs /run:rw,nosuid,nodev,mode=0755
  --tmpfs /tmp:rw,exec,nosuid,nodev,mode=1777
  --volume /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/sh:/bin/sh:ro
  --volume [CWD]/access-none/group:/etc/group:ro
  --volume [CWD]/access-none/hosts:/etc/hosts:ro
  --volume [CWD]/access-none/passwd:/etc/passwd:ro
  --tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,uid=1000,gid=100
  --volume [CWD]/access-none/hidden.txt:/home/u/.pot-hidden:ro
  --volume [CWD]/access-none/denials:/run/pot/denials:ro
  --volume /nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin/pot:/run/pot/launcher:ro
  --volume [CWD]/access-none/policy.json:/run/pot/policy.json:ro
  --volume /nix/store/zwplch93wbpj82bp7riwaixmdx2xi0ah-coreutils-9.7/bin/env:/usr/bin/env:ro
  --volume /nix/store/7f6kqbjr4l3qkl1m2s0a9hb3s5z7r0bq-nss-cacert-3.111/etc/ssl/certs/ca-bundle.crt:/etc/ssl/certs/ca-certificates.crt:ro
  --volume /home/u/.local/state/pot/state/9f2e/notes:/home/u/.cache/notes:rw
  --volume [CWD]/access-none/potcfg:/home/u/.config/pot:ro
  --volume /nix/store/0aqd4v7xhp2c9z2lfy5h4v0rdf3q4k8n-config.toml:/home/u/.config/notes/config.toml:ro
  --env HOME=/home/u
  --env LANG=C.UTF-8
  --env LOGNAME=u
  --env NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
  --env NOTES_CACHE=/home/u/.cache/notes
  --env PATH=/nix/store/zwplch93wbpj82bp7riwaixmdx2xi0ah-coreutils-9.7/bin:/nix/store/scikhwasnkiqmjsi9iczrrx59bwbn379-pot-kits/bin:/nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin
  --env POT=review
  --env POT_POLICY=/run/pot/policy.json
  --env POT_RUN=3c1a
  --env POT_TOOLS=/nix/store/scikhwasnkiqmjsi9iczrrx59bwbn379-pot-kits/bin
  --env 'PS1=(pot:review) [..]
  --env SHELL=/nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/bash
  --env SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
  --env TERMINFO_DIRS=/nix/store/1ylb9cjvz6p8pjkl3l4bl6wq0b1hd8xm-ncurses-6.5/share/terminfo
  --env TZ=America/Chicago
  --env TZDIR=/nix/store/3kgf5kq7p5xj6q9q0gk8m9z3xy7a0a7r-tzdata-2025b/share/zoneinfo
  --env USER=u
  --env TERM
  --env COLORTERM
  --env NOTES_TOKEN
  localhost/pot-empty:1
  /usr/bin/env -C /home/u -- notes check --all
# and before the root: --volume P:P:ro for each of 7 store paths (the closure)

```

## What this slice refuses

Write access needs the workspace's pins (git directories, hooks, protected paths as read-only mounts), which this
slice does not compute yet:

```console
$ pot-container --config config-podman.json prepare --print write
? 1
pot (review): the container backend does not carry out yet: write access (the workspace's pins: git directories, hooks, protected paths)

```

`prepare` needs the config's store paths:

```console
$ pot-container prepare --print read-only
? 1
pot (review): the backend's --config has no sh

```

`[..]` in an expected line matches the rest of it (the transcripts' runner reads a backslash as a path separator,
so a line with one is cut there).

## Sources

- [backends/container/src/render.rs](../../../backends/container/src/render.rs): `render`, `gaps`, `environment`
- [backends/container/src/main.rs](../../../backends/container/src/main.rs): `prepare`, `print_plan`
