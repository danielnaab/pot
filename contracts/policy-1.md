---
status: draft
---
# policy.json/1: a run's policy, for its backend

What pot composes for one run, and what a backend carries out. pot writes it as `<run dir>/policy.json` before it
calls the backend's `prepare`; inside the run it is `/run/pot/policy.json`, read-only, and `POT_POLICY` names it
([run-env/1](run-env-1.md)). `pot explain --json` prints the same document.

| | |
|---|---|
| Version | 1: the `version` field |
| Producer | pot (`core/compose.py` composes it; `core/pot.py` adds the children table and the command's `dir` and mirror `file` items, then writes it) |
| Reader | a backend: bubblewrap (`core/bwrap.py`) and Seatbelt (`core/seatbelt.py`), built with pot, and a backend built apart, such as a container backend; the run's own pot (`pot explain`, `pot why`, `pot runs` inside a run); the host pot when it admits a sibling; [pot-kits/1](pot-kits-1.md)'s wrappers (`mounts.launcher`); programs in a run ([run-env/1](run-env-1.md)) |
| Owner | pot |
| Schema | [policy-1.schema.json](policy-1.schema.json) (JSON Schema 2020-12) |
| Fixtures | [fixtures/policy-1/](fixtures/policy-1/) |

**Why it has a version now.** Until 0.1.0, the backends were built in the same derivation as the pot that wrote the
policy, so the format was internal. A backend built as its own binary, pinned on its own, reads the file across
a seam: what it may rely on, and what it must refuse, has to be written down. The `launch.json` beside it has its
own contract, [launch.json/1](launch-1.md).

## Shape

```json
{
  "version": 1, "run": "7f3a", "context": "work", "mode": "in-place", "access": "write",
  "backend": { "name": "bubblewrap", "requires": [], "proved": [ "closure-store", "own-netns", "…" ] },
  "workspace": [ { "id": "repo:…", "path": "/home/u/src/r", "common": "/home/u/src/r/.git",
                   "protected": [ "**/flake.nix", "…" ], "sealed": [ "**/.pi" ], "proposal": [ ], "clone": null } ],
  "items": [ { "kind": "network", "value": "api.anthropic.com:443", "class": "reach",
               "sources": [ { "kit": "root:claude-code@personal", "file": "…", "as": "request" },
                            { "grant": "grants.json", "digest": "sha256-…" } ], "effective": true,
               "request": "network api.anthropic.com:443", "requestClass": "ordinary" } ],
  "mounts": { "state": [ { "name": "claude", "at": "~/.claude", "key": [ "…" ], "dir": "/…/state/…/claude" } ],
              "run": "/run/pot", "launcher": "/run/pot/launcher" },
  "bindings": [ { "capability": "status.relay", "selector": "*", "kit": "…", "request": "capability status.relay",
                  "socket": "/run/pot/relay.sock", "see": [ "workmux shim" ], "allow": [ "…" ] } ],
  "children": { "git:…/notebook-kit:check-kb": { "slot": "scions.notebook-kit", "command": "check-kb", "how": "sibling",
                                                  "policy": "sha256-…", "argv": [ "…" ], "env": { }, "…": "…" } },
  "grants": [ { "id": "root:claude-code@personal", "digest": "sha256-…", "match": true } ]
}
```

Every field is always present. **MUST** marks what a backend reads and enforces: getting it wrong, or skipping it,
changes what the run can reach. **Info** marks what is there for people and for pot's own explanations (`pot
explain`, `pot why`, the startup line): a backend may show it and must not act on it.

### Top level

| Field | Type | | Meaning |
|---|---|---|---|
| `version` | `1` | MUST | A backend refuses any other value. |
| `run` | string | MUST | The run's id, four hex digits: the backend sets `POT_RUN` to it. `pot explain --json` prints `explain`. |
| `context` | string | MUST | The context's name: `POT`, and the prefix of every denial (`pot (<context>): …`). |
| `mode` | `in-place` \| `clone` | MUST | `clone`: the workspace is a clone (`workspace[].clone`), and the run gets `POT_CLONE=1`. |
| `access` | `write` \| `read` \| `none` | MUST | `write`: the workspace is writable, but for `protected`. `read`: it is read-only. `none` (a context's `access = "none"`): there is no workspace in the run. Nothing binds or allows any `workspace[]` `path` or `common`; where a folder that a read or write item names holds one, or `inspect`'s view would show one, it is masked (bubblewrap) or denied (Seatbelt), as a hidden path is; an item that names the path itself, or a path in it, still binds what it names. The command starts in the run's home, not in `launch.json`'s `cwd`. pot never composes `none` with `mode` `clone` (it refuses the run). Before 0.2.0 both backends treated `none` as `read`. |
| `backend` | object | MUST | `name` (`bubblewrap`, `seatbelt`, `container`: the [container backend](../docs/reference/container.md), built apart), the guarantees the context `requires`, and those the backend `proved` at this run's probe (`resource-limits` only for a run in the resource scope whose backend's probe lists it; `closure-store` dropped for inspect and for a run with the `nix` capability). pot refused the run (125) before writing the file if `requires` is not within `proved`. Since 0.3.0, `resources` (Info, optional): `{ "limits": "MemoryMax=… TasksMax=…" }` for a run that has them, else `{ "limits": null, "why": "…" }` (`at a terminal`, `no user systemd`, `the container backend can't keep its run in pot's scope here`); absent for `pot explain` on a backend that upholds them. A backend never acts on it: `proved` alone decides. |
| `workspace` | list | MUST | Each repository, below. Empty for `pot inspect` outside a repository. |
| `items` | list | MUST | Every see, limits and reach item, below. |
| `mounts` | object | MUST | `state`: each state directory (`name`, `at` in the run's home, its scope `key`, and the host `dir` to bind there); `run`: `/run/pot`, where the run sees its run folder; `launcher`: `/run/pot/launcher`, where the run's own pot is bound read-only (a wrapper tells a v2.18.0 or later run by this key). `key` is Info. |
| `bindings` | list | MUST | Each capability that is in effect and bound, below. |
| `children` | object | MUST (one bit) | The run's children table, keyed `<kit id>:<command>`, below. A backend reads one bit: whether a row has `how: "sibling"`, which means the run gets its siblings' broker. The rows are for the run's pot and the host's. |
| `grants` | list | Info | Each kit of the run: `id`, its request set's `digest` (`null` when it requests nothing) and whether its grant `match`ed. |

### `workspace[]`

| Field | | Meaning |
|---|---|---|
| `id` | Info | A [kit identity](kit-1.md#identity), `repo:…`. |
| `path` | MUST | The original path: where the run sees the repository. |
| `common` | MUST | Its git common directory (in clone mode, the clone's `.git`). |
| `protected` | MUST | Patterns (relative to `path`, `**/` for any depth) whose matches stay read-only and cannot be renamed, removed or replaced. Empty unless `access` is `write`. A link on a protected path, or a protected file with a second hard link, stops the run. |
| `sealed` | MUST | Protected folders whose new entries are the run's alone (under bubblewrap: each existing entry read-only, a new one discarded at the end and listed in the `after` event). A backend that cannot seal keeps them plainly read-only, which is narrower, and says so (`pot explain`: `sealed: none`). Always empty in a clone, and with `access` `none`. |
| `proposal` | MUST | In a clone, the agent-configuration patterns that are writable there and flagged in review: a backend refuses a link on one, and reports changes to them in its `after` event. Always empty in place. |
| `clone` | MUST | `null` in place; in clone mode, the clone's host path, which the run sees at `path` when the backend proves `same-path-clone` (else at its own path). |

### `items[]`

Each item has `kind`, `value`, `class` (`see`, `limits`, `reach`), `sources` and `effective`. **A backend carries
out only items with `effective: true`.** An item not in effect says `why` (`not granted`, `not in this run`,
`no binding`, …): it is there for `pot explain`. See and limits items are always in effect.

| Kind | Class | | What a backend does with an effective one |
|---|---|---|---|
| `package` | see | MUST | `value` is a store path: in the run's closure, and its `bin/` on `PATH`, in order. `tools: true` marks the workspace bundle's: `POT_TOOLS` is its `bin/`. `attr` (a `--package`'s attribute) is Info. |
| `env.set` | see | MUST | `NAME=VALUE`, set in the run; a value `~` or `~/…` is under the run's home. |
| `file` | see | MUST | A file at `value` (`/…`, needing `root-files`, or `~/…`) from the host's `source`. `mode`: `fixed`, bound read-only; `seed`, copied into the state directory that holds it once (each start with `refresh: true`); `reassert`, its `keys` (dotted, `*` for each workspace path) rewritten from `source` at each start. `memory` and `mirror` (a mirror dir's label) are Info. |
| `dir` | see | MUST | A read-only folder at `value` (`~/…`) of `links`: each `[name, target]` a link to `target` (bound read-only where it is outside the store), or `[name, target, "bind"]`, the host entry bound there. `skipped` is Info. |
| `hidden` | limits | MUST | `paths`, the host paths it resolved to, are never visible: masked where they fall under anything bound, and nothing is bound inside a hidden folder. `value` (the pattern) is Info. |
| `protect` | limits | Info | The pattern and its source. A backend reads `workspace[].protected`, which pot computed from these. |
| `access` | limits | Info | The same as the top-level `access`, with its source. |
| `network` | limits | Info | `value` `none`: the context has no network. Every network reach item is then not in effect, so a backend that follows the rule below gives no network. |
| `network` | reach | MUST | `host:port` (`*` and `?` in the host): the run reaches exactly these, through pot's proxy, and nothing else. No effective network reach item: no network at all. `via: "mcp"` (an MCP server's host) is Info. |
| `read` | reach | MUST | `path` (the host path `value` resolved to), bound read-only where it exists. |
| `write` | reach | MUST | `path`, bound writable where it exists. |
| `socket` | reach | MUST | `path`, bound; every other socket under a bound folder is masked. |
| `env` | reach | MUST | The host's variable `value`, passed when it is set. `secret: true`: never shown (`(the run holds it)`). |
| `state` | reach | Info | The request (`at`, `shared`). A backend binds `mounts.state`, which pot computed from these. |
| `capability` | reach | Info | The request (`data`, the `binding` selector). A backend carries out `bindings`, which pot computed from these. |
| `include` | reach | Info | A root kit the context includes; its requests are items of their own. |
| `view` | reach | MUST | `inspect`'s view (value `inspect`): the whole file system read-only, the home empty but for `home` (paths read-only) and, with `homeManager`, home-manager's links; secrets and sockets masked; the workspace read-only, or with `access` `none` masked (the state mounts stay writable). |
| `unsandboxed` | reach | MUST | Never in effect in a policy pot writes: a granted one runs the command on the host instead. A backend refuses a policy that has one in effect. |

`request`, `requestClass` (`ordinary`, `powerful`) and `sources` are Info. Each source is one of `{ kit, file,
as }` (`as`: `see`, `limit`, `request`, `tools`), `{ grant, digest }`, `{ flag }` (`--net`, `--read`, `--write`,
`--env`, `--package`), `{ context }`, `{ binding: "mcp.<selector>" }` or `{ root, why? }` (`root`: `default`,
`bundle`, `standard`, `tools`, `inspect`, `base`, `caBundle`, `hidden`, `pot`, `binding`); run-env/1 says what each
means.

### `bindings[]`

Each has `capability`, `selector` (the root's binding that matched), `kit` and `request` (Info, the last three).
A binding never holds a secret value: `from` and `key` are references to the host's files and variables.

| Capability | | Fields, and what a backend does |
|---|---|---|
| `status.relay` | MUST | `socket` `/run/pot/relay.sock`: the broker's relay, with the `workmux` shim on `PATH`. `see`, `allow` (the relayed verbs) are Info. |
| `git.fetch` | MUST | `socket` `/run/pot/git.sock`: the broker fetches over ssh for the run, only from `remotes` (patterns), with `key` (`{ agent: true }`, or `{ file }` on the host); the run's `GIT_SSH_COMMAND` is the `pot-ssh` shim. |
| `nix` | MUST | `socket` `/run/pot/nix.sock`: an untrusted relay to the host's daemon (`via` `daemon-untrusted`), the whole store read-only, `NIX_REMOTE` set. `notes` are Info. |
| `credential` | MUST | The variables `env`, read on the host at run time from `from` (`file` or `envFile`, a key file; `env`, a host variable; nothing: the host's variables of the same names). `via` says how the run gets each value. `env`: as the variable's value. `file` (since 0.2.0; before it, both backends passed the value as with `env`): written to `secrets/NAME` in the run folder, the run's `/run/pot/secrets/NAME`, mode 0400 and owned by the run's user, readable by that run alone and never writable by it; the variable is that file's path in the run. The backend removes the file when the command ends and at `cleanup` (also when `prepare` stopped). Never written anywhere else: not to policy.json, launch.json, a log or the arguments. `holds` and `refresh` are Info. |
| `mcp` | MUST | As `credential` for its key (`via` included), and the MCP configuration at `config` (`/run/pot/mcp.json`): `server` at `url`. With `via` `env`, `headers` holds `header`: `scheme ${NAME}` (the client expands the variable). With `via` `file`, no `headers`: a `headersHelper`, a shell command the client runs, reads the key from the file the variable names, with shell builtins, and prints `{"header": "scheme KEY"}`; a backend refuses a key with a quote, a backslash or a control character, which that JSON could not carry. |

A backend refuses a binding whose `capability` it does not carry out (`git.push`, `git.sign`, `ops.read` and
`delegate` are reserved; compose never binds them in this release).

### `children`

Each row has `slot` (`root`, `self`, `scions.<name>`), `command`, `kit`, `kitName`, `context`, `mode`, `file` and
`hook` (a pre-commit hook's name, when the command is one). An admitted row adds `how` (`exec`: the same boundary,
run in place; `sibling`: narrower, started by the host beside the run), `policy` (the digest of the row's own
policy, without `run` and `children`), `argv` and `env`. A refused row adds `refused` (why), and maybe `text` (the
message the run's pot prints) and `grant: true` (a grant would admit it). Neither kind has the other's keys.

## Versioning

- **Values, not fields** ([How a contract changes](README.md#how-a-contract-changes)). Within version 1 a release
  may add values: an item kind, a source, a guarantee, a capability, a backend name. **A new field, or a changed
  meaning, is a new version**, policy.json/2. That includes an item's keys: [run-env/1](run-env-1.md)'s older
  promise that a release "may add an optional key to an item" is narrowed here, because a backend that does not
  know a key that limits the run would run it wider.
- **Strict schema.** Every object pot builds is closed (`additionalProperties: false`): the producer is pot alone,
  so its test catches a field it adds without a version. Two objects stay open, because pot copies them from the
  root's `bindings` as written: a binding's `from` and `key` (their known keys are typed).
- **The schema is this release's.** A reader that validates against it would reject a later release's new value,
  so readers use the fixtures in their tests and, at run time, the rules below.

## For backend authors

**Inputs.** A backend reads `<run dir>/policy.json` (this contract) and `<run dir>/launch.json`, the launcher's
facts (`argv`, `cwd`, the caller's identity, `closure`, `potBin`, `hostTools`, the sibling and clone facts):
[launch.json/1](launch-1.md).

**The interface,** as `core/bwrap.py` implements it: `backend [--config CONFIG] VERB RUNDIR`.

| Verb | When | Does |
|---|---|---|
| `probe` | before pot composes (no policy.json yet) | prints `{ "guarantees": [ … ] }`, the guarantees it proves on this host, or `{ "guarantees": [ ], "error": "…" }`; pot refuses the run when none. `resource-limits` in the list means that what the backend starts stays in its caller's cgroup; pot claims it for a run only when the run is in its resource scope too (since 0.3.0; before, pot claimed it for every run in the scope). pot caches bubblewrap's and Seatbelt's per build; the container backend's answer depends on the caller (its NoNewPrivs, its engine's state), so pot must probe it for every run, never from a cache |
| `prepare` | after pot writes policy.json and launch.json | everything that can fail before the command starts: the mounts or rules, pins, placeholders, the run's files; exit non-zero (one `pot (<context>): …` line on stderr) to stop the run. `--print` prints the command instead (`pot explain --backend-command`) |
| `run` | then | starts the proxy and the bindings' helpers, runs `launch.json`'s `argv` sandboxed with the environment above, and exits with the command's status |
| `after` | after `run`, also for a run pot recovers | prints one `after` event ([runs/1](runs-1.md)) on stdout: what the run left (commits, host-run files, new git metadata, sealed leftovers; for a clone, the staging fetch) |
| `cleanup` | last, always | removes what `prepare` placed on the host (placeholders, sockets, sealed bases) |

**What a backend must uphold, or refuse** ([decisions](../docs/decisions.md), principle 6: one policy, many
backends, never weaker in silence):

1. Refuse a `version` it does not read, and a `backend.name` that is not its own.
2. Uphold every guarantee in `backend.proved` for this run, or refuse: `proved` is what pot showed the person and
   checked `requires` against.
3. Refuse an effective item of a kind it does not know in class `limits`, and an effective `see` or `reach` item it
   cannot carry out: never run without it. Skip items with `effective: false`.
4. Refuse a binding whose capability it does not carry out, and an effective `unsandboxed` item.
5. Never widen: nothing outside the effective items, `workspace`, `mounts` and `bindings` is visible, writable or
   reachable. Hidden paths stay hidden even under a bound folder; the network is the proxy's allowlist or nothing.
6. Never write a secret value to policy.json, launch.json, a log, the run's files or its arguments (the one file
   is a `via` `file` binding's own, in `/run/pot/secrets`); never show a `secret` env item's value.
7. Leave the host as it found it: `cleanup` removes what `prepare` placed, even when `prepare` stopped.

bubblewrap and Seatbelt are built with the pot that writes the policy and trust its composition: they do not make
checks 1 to 4 themselves. A backend built apart must.

## Fixtures

[fixtures/policy-1/](fixtures/policy-1/) holds policies that pot wrote in its own tests (`tests/run.sh`), with the
host's facts made synthetic: the home as `/home/u` (the tests resolve hidden paths against the real one), the host's
name as `test-host` and the tests' work folder as `/tmp/pot-test`. Readers test against these.

| Fixture | What it shows |
|---|---|
| `work-bindings.json` | `work`, in place, write: `status.relay`, `credential` (`via` `file`) and `mcp` (`via` `env`) bindings (`from.file`), an MCP server's host as network reach, a `reassert` file, a state mount, an included root kit, an `exec` row and a refused one |
| `pi-agent.json` | the pi kit's run: `dir` items and `fixed` `file` items, `status.relay` and `credential` bindings |
| `work-siblings.json` | a children table with `sibling` rows (one a pre-commit `hook`), an `exec` row and a refused one |
| `sibling.json` | a sibling's own policy: read access, the network its kit asked for |
| `dispatch-clone.json` | `dispatch`: a clone, `proposal` patterns, `same-path-clone` required, sibling rows in clone mode |
| `inspect.json` | `inspect`: the `view` item, read access, `closure-store` dropped |
| `git-fetch.json` | a `git.fetch` binding (`key.agent`) |
| `nix.json` | a `nix` binding: `closure-store` dropped |
| `seatbelt.json` | Seatbelt: `backend.name` `seatbelt` and its proved set, a clone without `same-path-clone` (it runs at its own path). Composed by `compose()` from the unit tests' inputs, since the test host is not a Mac; `tests/run.sh pot` on macOS validates what Seatbelt runs write. |

`invalid/` holds documents a reader must reject: each breaks one rule above.

pot's tests hold pot to the schema: `tests/run.sh contracts` validates the fixtures and refuses the invalid ones;
the `pot`, `dispatch`, `sibling` and `pi` sections (and `rollback`) validate every `policy.json` their runs wrote,
and `pot explain --json`.

## Sources

- [core/compose.py](../core/compose.py): `compose()` (the policy), `binding_entry`, `children_table`
- [core/pot.py](../core/pot.py): `gather`, `fixed_dirs`, writing policy.json and launch.json, the backend's verbs
- [core/bwrap.py](../core/bwrap.py), [core/seatbelt.py](../core/seatbelt.py): the two readers
- [tests/run.sh](../tests/run.sh): the `contracts` section, and `policies_valid`
- [run-env/1](run-env-1.md): the policy as a program in a run sees it
