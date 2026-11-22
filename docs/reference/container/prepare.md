---
status: draft
---
# pot-container prepare

`pot-container --config CONFIG prepare RUNDIR` reads the run's `policy.json` ([policy.json/1](../../../contracts/policy-1.md))
and `launch.json`, refuses what this backend does not carry out, and places the run's files; `--print` prints the
engine's command instead and writes nothing (`pot explain --backend-command`). The flags are explained in
[the container backend](../container.md#policy-to-flags).

These transcripts run in `prepare.in/`. `read-only/` is a review context: read access to `~/src/notes`, a read
path, a hidden folder, a state mount, a fixed file, a secret variable (handed over the run folder's named pipe), and
`resource-limits` proved (the run is in pot's systemd scope). `network/` is `access-none/` with the network: one host
through pot's proxy, and `private-proxy` proved. `config-podman.json` and `config-docker.json` name the store paths
this backend binds into every run, and the network's programs (`python`, `proxy`, `socat`, `netClosure`). None of
the host paths exist here, so no read path, mask or socket is bound.

## podman

```console
$ pot-container --config config-podman.json prepare --print read-only
# the podman command for this run, an option a line (pot explain --backend-command)
# the engine runs with only these of the caller's variables: PATH HOME USER LOGNAME XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS TMPDIR LANG TERM COLORTERM; and CONTAINERS_CONF=[CWD]/read-only/containers.conf CONTAINERS_STORAGE_CONF=[CWD]/read-only/storage.conf HOME=[CWD]/read-only/enginehome
podman
  --root [CWD]/read-only/enginestore --runroot [CWD]/read-only/engineroot --tmpdir [CWD]/read-only/enginetmp
  run --interactive
  --init
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
  --tmpfs /run:rw,nosuid,nodev,mode=0755,notmpcopyup
  --tmpfs /tmp:rw,exec,nosuid,nodev,mode=1777,notmpcopyup
  --volume /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/sh:/bin/sh:ro
  --volume [CWD]/read-only/group:/etc/group:ro
  --volume [CWD]/read-only/hosts:/etc/hosts:ro
  --volume [CWD]/read-only/passwd:/etc/passwd:ro
  --tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,U,notmpcopyup
  --volume [CWD]/read-only/hidden.txt:/home/u/.pot-hidden:ro
  --volume [CWD]/read-only/denials:/run/pot/denials:ro
  --volume /nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin/pot:/run/pot/launcher:ro
  --volume [CWD]/read-only/mcp.json:/run/pot/mcp.json:ro
  --volume [CWD]/read-only/policy.json:/run/pot/policy.json:ro
  --volume [CWD]/read-only/secrets.fifo:/run/pot/secrets.fifo:ro
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
  --rootfs [CWD]/read-only/rootfs:O
  /usr/bin/env -i -- /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/bash --noprofile --norc -c [..] pot-entry /run/pot/secrets.fifo '' 3 TERM COLORTERM NOTES_TOKEN /usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C /home/u/src/notes -- [..] notes check --all
# and before the root: --volume P:P:ro for each of 7 store paths (the closure)
# and writes the variables passed by name to the run folder's named pipe, secrets.fifo: TERM COLORTERM NOTES_TOKEN

```

## docker

docker can't keep a container in pot's scope, so it refuses a run that has `resource-limits`:

```console
$ pot-container --config config-docker.json prepare --print read-only
? 1
pot (review): the container backend does not carry out yet: resource-limits under docker (a container is not in pot's scope)

```

Without it (`access-none/`: no workspace at all, so the command starts in the home; `NOTES_TOKEN` a plain variable):

```console
$ pot-container --config config-docker.json prepare --print access-none
# the docker command for this run, an option a line (pot explain --backend-command)
# the engine runs with only these of the caller's variables: PATH HOME USER LOGNAME XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS TMPDIR LANG TERM COLORTERM NOTES_TOKEN; and DOCKER_CONFIG=[CWD]/access-none/dockercfg
docker
  run --interactive
  --init
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
  --name pot-[..]
  --entrypoint ''
  --tmpfs /run:rw,nosuid,nodev,mode=0755
  --tmpfs /tmp:rw,exec,nosuid,nodev,mode=1777
  --mount type=bind,source=/nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/sh,target=/bin/sh,readonly
  --mount type=bind,source=[CWD]/access-none/group,target=/etc/group,readonly
  --mount type=bind,source=[CWD]/access-none/hosts,target=/etc/hosts,readonly
  --mount type=bind,source=[CWD]/access-none/passwd,target=/etc/passwd,readonly
  --tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,uid=1000,gid=100
  --mount type=bind,source=[CWD]/access-none/hidden.txt,target=/home/u/.pot-hidden,readonly
  --mount type=bind,source=[CWD]/access-none/denials,target=/run/pot/denials,readonly
  --mount type=bind,source=/nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin/pot,target=/run/pot/launcher,readonly
  --mount type=bind,source=[CWD]/access-none/mcp.json,target=/run/pot/mcp.json,readonly
  --mount type=bind,source=[CWD]/access-none/policy.json,target=/run/pot/policy.json,readonly
  --mount type=bind,source=/nix/store/zwplch93wbpj82bp7riwaixmdx2xi0ah-coreutils-9.7/bin/env,target=/usr/bin/env,readonly
  --mount type=bind,source=/nix/store/7f6kqbjr4l3qkl1m2s0a9hb3s5z7r0bq-nss-cacert-3.111/etc/ssl/certs/ca-bundle.crt,target=/etc/ssl/certs/ca-certificates.crt,readonly
  --mount type=bind,source=/home/u/.local/state/pot/state/9f2e/notes,target=/home/u/.cache/notes
  --mount type=bind,source=[CWD]/access-none/potcfg,target=/home/u/.config/pot,readonly
  --mount type=bind,source=/nix/store/0aqd4v7xhp2c9z2lfy5h4v0rdf3q4k8n-config.toml,target=/home/u/.config/notes/config.toml,readonly
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
  /usr/bin/env -C /home/u -u HOSTNAME -- notes check --all
# and before the root: --volume P:P:ro for each of 7 store paths (the closure)

```

## The network

The container keeps `--network none`. `run` starts pot's proxy on the host, listening on the run folder's
`proxy.sock`, bound at `/run/pot/proxy.sock`; the entry starts socat (its closure bound) on 127.0.0.1:3128 before the
command; the proxy's variables are bwrap's. The variables passed by name go over the named pipe:

```console
$ pot-container --config config-podman.json prepare --print network
# the podman command for this run, an option a line (pot explain --backend-command)
# the engine runs with only these of the caller's variables: PATH HOME USER LOGNAME XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS TMPDIR LANG TERM COLORTERM; and CONTAINERS_CONF=[CWD]/network/containers.conf CONTAINERS_STORAGE_CONF=[CWD]/network/storage.conf HOME=[CWD]/network/enginehome
podman
  --root [CWD]/network/enginestore --runroot [CWD]/network/engineroot --tmpdir [CWD]/network/enginetmp
  run --interactive
  --init
  --log-driver none
  --cidfile [CWD]/network/container.id
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
  --tmpfs /run:rw,nosuid,nodev,mode=0755,notmpcopyup
  --tmpfs /tmp:rw,exec,nosuid,nodev,mode=1777,notmpcopyup
  --volume /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/sh:/bin/sh:ro
  --volume [CWD]/network/group:/etc/group:ro
  --volume [CWD]/network/hosts:/etc/hosts:ro
  --volume [CWD]/network/passwd:/etc/passwd:ro
  --tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,U,notmpcopyup
  --volume [CWD]/network/hidden.txt:/home/u/.pot-hidden:ro
  --volume [CWD]/network/denials:/run/pot/denials:ro
  --volume /nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin/pot:/run/pot/launcher:ro
  --volume [CWD]/network/mcp.json:/run/pot/mcp.json:ro
  --volume [CWD]/network/policy.json:/run/pot/policy.json:ro
  --volume [CWD]/network/proxy.sock:/run/pot/proxy.sock:ro
  --volume [CWD]/network/secrets.fifo:/run/pot/secrets.fifo:ro
  --volume /nix/store/zwplch93wbpj82bp7riwaixmdx2xi0ah-coreutils-9.7/bin/env:/usr/bin/env:ro
  --volume /nix/store/7f6kqbjr4l3qkl1m2s0a9hb3s5z7r0bq-nss-cacert-3.111/etc/ssl/certs/ca-bundle.crt:/etc/ssl/certs/ca-certificates.crt:ro
  --volume /home/u/.local/state/pot/state/9f2e/notes:/home/u/.cache/notes:rw
  --volume [CWD]/network/potcfg:/home/u/.config/pot:ro
  --volume /nix/store/0aqd4v7xhp2c9z2lfy5h4v0rdf3q4k8n-config.toml:/home/u/.config/notes/config.toml:ro
  --env ALL_PROXY=http://127.0.0.1:3128
  --env HOME=/home/u
  --env HTTPS_PROXY=http://127.0.0.1:3128
  --env HTTP_PROXY=http://127.0.0.1:3128
  --env LANG=C.UTF-8
  --env LOGNAME=u
  --env NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
  --env NOTES_CACHE=/home/u/.cache/notes
  --env NO_PROXY=localhost,127.0.0.1,::1
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
  --env all_proxy=http://127.0.0.1:3128
  --env http_proxy=http://127.0.0.1:3128
  --env https_proxy=http://127.0.0.1:3128
  --env no_proxy=localhost,127.0.0.1,::1
  --rootfs [CWD]/network/rootfs:O
  /usr/bin/env -i -- /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/bash --noprofile --norc -c [..] pot-entry /run/pot/secrets.fifo /nix/store/q17xc8gl00wiq6gdblhn5gzln98zibm4-socat-1.8.0.3/bin/socat 3 TERM COLORTERM NOTES_TOKEN /usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C /home/u -- [..] notes check --all
# and before the root: --volume P:P:ro for each of 9 store paths (the closure)
# before the engine, run starts pot's proxy in the run folder: /nix/store/jd20rkmqmkfkcvk2wl2lmzz7acq4svlr-python3-3.12.12/bin/python3 -I -S /nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/proxy.py proxy.sock --denials [CWD]/network/denials --context review --fix 'the repo kit'/''s requests.network' api.example.com:443
# and writes the variables passed by name to the run folder's named pipe, secrets.fifo: TERM COLORTERM NOTES_TOKEN

```

## Bindings

`bindings/` is `access-none/` with three bindings: a `credential` with `via: file` (its key file), a `credential` with
`via: env` (a host variable of another name) and an `mcp` with `via: env`. Their variables go over the named pipe;
`run` reads the values on the host when it starts. `secrets/` is bound for the via-file one, and `mcp.json` (bound for
every run, as bubblewrap's) names the variable, never its value:

```console
$ pot-container --config config-podman.json prepare --print bindings
# the podman command for this run, an option a line (pot explain --backend-command)
# the engine runs with only these of the caller's variables: PATH HOME USER LOGNAME XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS TMPDIR LANG TERM COLORTERM; and CONTAINERS_CONF=[CWD]/bindings/containers.conf CONTAINERS_STORAGE_CONF=[CWD]/bindings/storage.conf HOME=[CWD]/bindings/enginehome
podman
  --root [CWD]/bindings/enginestore --runroot [CWD]/bindings/engineroot --tmpdir [CWD]/bindings/enginetmp
  run --interactive
  --init
  --log-driver none
  --cidfile [CWD]/bindings/container.id
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
  --tmpfs /run:rw,nosuid,nodev,mode=0755,notmpcopyup
  --tmpfs /tmp:rw,exec,nosuid,nodev,mode=1777,notmpcopyup
  --volume /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/sh:/bin/sh:ro
  --volume [CWD]/bindings/group:/etc/group:ro
  --volume [CWD]/bindings/hosts:/etc/hosts:ro
  --volume [CWD]/bindings/passwd:/etc/passwd:ro
  --tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,U,notmpcopyup
  --volume [CWD]/bindings/hidden.txt:/home/u/.pot-hidden:ro
  --volume [CWD]/bindings/denials:/run/pot/denials:ro
  --volume /nix/store/7v2kq1n2h0b8yq1d4l3s1w8d0m2c9x3a-pot/bin/pot:/run/pot/launcher:ro
  --volume [CWD]/bindings/mcp.json:/run/pot/mcp.json:ro
  --volume [CWD]/bindings/policy.json:/run/pot/policy.json:ro
  --volume [CWD]/bindings/secrets:/run/pot/secrets:ro
  --volume [CWD]/bindings/secrets.fifo:/run/pot/secrets.fifo:ro
  --volume /nix/store/zwplch93wbpj82bp7riwaixmdx2xi0ah-coreutils-9.7/bin/env:/usr/bin/env:ro
  --volume /nix/store/7f6kqbjr4l3qkl1m2s0a9hb3s5z7r0bq-nss-cacert-3.111/etc/ssl/certs/ca-bundle.crt:/etc/ssl/certs/ca-certificates.crt:ro
  --volume /home/u/.local/state/pot/state/9f2e/notes:/home/u/.cache/notes:rw
  --volume [CWD]/bindings/potcfg:/home/u/.config/pot:ro
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
  --rootfs [CWD]/bindings/rootfs:O
  /usr/bin/env -i -- /nix/store/nn27l879np49xvx6l1a8nqnqlp9apd8n-bash-interactive-5.2p37/bin/bash --noprofile --norc -c [..] pot-entry /run/pot/secrets.fifo '' 6 TERM COLORTERM NOTES_TOKEN EXAMPLE_TOKEN CI_TOKEN DOCS_KEY /usr/bin/env -u _ -u SHLVL -u PWD -u OLDPWD -C /home/u -- [..] notes check --all
# and before the root: --volume P:P:ro for each of 7 store paths (the closure)
# the bindings' values, read on the host when run starts, go over the named pipe: EXAMPLE_TOKEN=/run/pot/secrets/EXAMPLE_TOKEN CI_TOKEN=(the run holds it) DOCS_KEY=(the run holds it)
# a via-file value is written to the run folder's secrets/NAME (0400), removed when the command ends
# and writes the variables passed by name to the run folder's named pipe, secrets.fifo: TERM COLORTERM NOTES_TOKEN EXAMPLE_TOKEN CI_TOKEN DOCS_KEY

```

## What this slice refuses

Write access in place binds the pins pot's core resolved (launch.json's `pins`) and runs pot's `pins.py` for the
host's look before and after the run, so it needs the config's `python`, `pins` and `git`, each a file. This
config names none of them (the run that binds the pins is tested with real engines, `tests/run.rs`):

```console
$ pot-container --config config-podman.json prepare --print write
? 1
pot (review): the container backend does not carry out yet: write access: the backend's --config names no python, pins and git (the host's look before and after the run)

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
