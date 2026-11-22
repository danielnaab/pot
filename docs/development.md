---
status: working
---
# Development

How to change pot, test it, and release it.

## Layout

| Path | Contents |
|---|---|
| `flake.nix` | `lib.mkRoot`, `lib.pot.kitsFor`, `kits.claude-code`, `kits.pi`, `apis`, `homeManagerModules.pot` (and `.default`), pot with its default root (`packages.default`, `apps.default`, both also named `pot`), and the checks. It declares no `inputs.self.submodules`: pot is other flakes' input. |
| `core/` | pot's Python: the command (`pot.py`, which holds `VERSION`), composition and checks (`compose.py`), the backends (`bwrap.py`, `seatbelt.py`), the proxy, the broker, clones, runs, siblings and the shims. Only its `.py` files reach the store, so a change elsewhere changes no root. |
| `lib/` | `kits-for.nix` (`kitsFor`, [pot-lib/1](../contracts/pot-lib-1.md): the bundle a producer gives) and `kit.nix` (a kit as a manifest). |
| `nix/` | `root.nix` (`mkRoot`, the standard set) and `hm.nix` (`homeManagerModules.pot`). |
| `kits/` | The agent kits, `claude-code.nix` and `pi.nix`, and `apis.nix`. |
| `contracts/` | [pot's contracts](../contracts/README.md): the formats pot shares, each versioned, with schemas and fixtures where they have them. |
| `tests/` | The unit tests (`test_*.py`, with `output/`, the expected text), the golden grant test (`golden.nix`, `golden.py`, `golden.json`), and the end-to-end test, `run.sh`. |
| `dev/flake.nix` | Development-only inputs (knowledge-base-kit) and the docs check, kept out of pot's lock and so out of every root's and producer's. |
| `docs/`, `README.md`, `AGENTS.md` | Documentation, checked against knowledge-base-kit's conventions. |

`backends/container/` is pot's first Rust code, the [container backend](reference/container.md) (draft): a member of
the Cargo workspace at the root (`Cargo.toml`, `Cargo.lock`, `deny.toml`, `rustfmt.toml`), built by `nix/rust.nix`
with crane, on grove's conventions (clippy pedantic with warnings denied, `unsafe` forbidden, few dependencies with
a reason each, trycmd transcripts under `docs/reference/container/`). `nix develop` has cargo, clippy, rustfmt and
cargo-deny.

## Test

```
nix flake check                  # bash -n and shellcheck, the unit tests, the golden grant test, roots and kits build
nix flake check ./dev            # the docs check
tests/run.sh                     # contracts kits pot pi dispatch sibling, against throwaway repositories and roots
tests/run.sh kits pot            # some sections only
tests/run.sh rollback            # graft v2.21.0's pot on this pot's state (a release check; fetches graft)
KEEP=1 tests/run.sh              # the same, and keep the work directory to debug a failure
```

`tests/run.sh` takes minutes; do not run two at once. Runs need unprivileged user namespaces; without them, the
parts that start runs are skipped, saying so. On macOS the `pot` section runs the Seatbelt backend's own part
(`pot_seatbelt`).

`nix flake check` runs `shellcheck`, the container backend's build, tests (its unit tests and transcripts),
clippy, rustfmt and cargo-deny (`pot-container`, `-test`, `-clippy`, `-fmt`, `-deny`; in `nix develop`, `cargo test`
and `TRYCMD=overwrite cargo test` to rewrite a transcript's output), the unit tests (`pot-unit`), the golden grant test (`pot-golden`: the
canonical grant items and digests of the default root's `claude-code@personal` and of `tests/golden.nix`'s root,
equal `tests/golden.json` byte for byte; never regenerate it to make the check pass), and checks that roots, the
kits and the home-manager module build (`pot-root`, `pot-root-claude`, `pot-gateway`, `pot-pi`, `pot-root-pi`,
`pot-removed`, `pot-markers`, `pot-hm`).

**A test-only fault point.** `POT_TEST_FAULT=after` (an error in the host's look at a clone) and
`POT_TEST_FAULT=after-exit` (a backend that prints no `after` event) let `tests/run.sh dispatch` check that such a
run lands in `review` with `fetch failed: the host's look failed (…)`, listed and kept. The backend reads it from
the environment of the pot that starts the run, on the host; a run's own environment never reaches it. It can only
make a run's look fail, which lands the run in `review`: it widens nothing.

**The docs check** is in `dev/flake.nix`, so that knowledge-base-kit is in no root's or producer's lock. It reads
the whole repository, so run it from the checkout as a git flake, `nix flake check ./dev`, never as `path:`; like
any git flake it sees only what git tracks, so `git add` a new page before you check it. To move knowledge-base-kit,
set its tag in `dev/flake.nix` and run `XDG_CACHE_HOME=$(mktemp -d) nix flake update knowledge-base-kit --flake ./dev`
(an empty fetch cache, so a tag is locked from the forge, never from a tree a local checkout left in the cache),
then commit both.

## Release checks

pot's grants and state outlive any one release, and a root must be able to move back. Before a release:

1. **The golden grant test, against graft v2.21.0.** `tests/golden.json` is byte-identical to graft v2.21.0's
   `pot/tests/golden.json`, and `nix flake check` (`pot-golden`) passes against it. The same canonical items and
   digests mean every grant a person made under graft's pot still applies:

   ```
   git -C ../graft show v2.21.0:pot/tests/golden.json | cmp - tests/golden.json
   nix build .#checks.x86_64-linux.pot-golden
   ```

   A release that must change a digest is a new canonical version ([grants/1](../contracts/grants-1.md)), never an
   edit of the fixture.
2. **Rollback.** `tests/run.sh rollback` builds two roots with the same arguments, one from this checkout and one
   from graft v2.21.0 (`POT_ROLLBACK_FROM`, default `github:danielnaab/graft/v2.21.0`), on one grants file and one
   state folder in its work directory (never your own). This pot grants, runs and dispatches; graft's pot then runs
   with no grant asked again, lists and fetches this pot's runs, and leaves the grants file unchanged; then this pot
   reads what graft's pot wrote. So reverting a root to graft's pot works ([CHANGELOG](../CHANGELOG.md)). For a
   later release, point `POT_ROLLBACK_FROM` at the previous pot release (`github:danielnaab/pot/v0.1.0`).
3. `nix flake check`, `nix flake check ./dev` and `tests/run.sh` pass.

## Release

1. Set `VERSION` in `core/pot.py` and add the CHANGELOG section `## vX.Y.Z - DATE` (`tests/run.sh dispatch`
   checks that `pot doctor` prints the CHANGELOG's latest). `pot init` pins `github:danielnaab/pot/vVERSION`.
2. Run the [release checks](#release-checks).
3. Tag it, annotated and signed: `git tag -s vX.Y.Z -m "pot vX.Y.Z"`. Never move a tag: a fix is the next patch.

Below 1.0, a contract may change in a new `0.y`, never in a patch; a reader changes first
([pot's contracts](../contracts/README.md#how-a-contract-changes)).

## Sources

- [flake.nix](../flake.nix): the outputs and checks
- [tests/run.sh](../tests/run.sh): the sections, `rollback`
- [tests/golden.py](../tests/golden.py): the golden grant test
- [core/pot.py](../core/pot.py): `VERSION`
