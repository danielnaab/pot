# Agent entrypoint

pot runs a command, such as a coding agent, in a sandbox whose reach each repository's kits ask for and a root
grants. Read [README.md](README.md), then [Concepts](docs/concepts.md). This file is for agents working on pot
itself.

## Orientation

- **Configuration:** [knowledge-base.yaml](knowledge-base.yaml): the docs' conventions are knowledge-base-kit's
  (v2.1.0), taken as a development input in [dev/flake.nix](dev/flake.nix), never as a dependency of pot's flake
- **Docs:** [guides](docs/guides/), [reference](docs/reference/pot.md), [decisions](docs/decisions.md),
  [development](docs/development.md) (layout, tests, release checks)
- **Code:** `core/` (Python), `lib/` (kitsFor), `nix/` (mkRoot, the home-manager module), `kits/` (agent kits)
- **Contracts:** [contracts/](contracts/README.md): what pot shares with producers and readers, each versioned

## Rules

1. **pot depends on nothing of tuteur's (or graft's).** Its flake never names them; it reads only bundles that its
   own `kitsFor` wrote, and git facts. A producer calls pot ([pot-lib/1](contracts/pot-lib-1.md)), never the reverse.
2. **Behaviour and docs change together.** A change to a command, argument, message or file format updates its
   reference page, and its contract when it touches one, in the same commit.
3. **Contracts change reader first,** and a new field is a new version ([How a contract changes](contracts/README.md#how-a-contract-changes)).
   `tests/golden.json` is never regenerated to make a check pass: a moved digest re-asks every grant.
4. Do not change `CHANGELOG.md` sections of released versions. pot's version is `VERSION` in `core/pot.py`, alone.
5. Test first for a change in behaviour: a failing case in `tests/run.sh` or `tests/test_*.py`, then the change.

## Verify

1. `nix flake check`: shellcheck, the unit tests, the golden grant test, roots and kits build.
2. `nix flake check ./dev`: the docs check. Run it from this checkout, never as `path:`, and `git add` new pages
   first.
3. `tests/run.sh` after a change to `core/`, `lib/`, `nix/` or `kits/`. It takes minutes; do not run two at once.
   Before a release, also `tests/run.sh rollback` ([Release checks](docs/development.md#release-checks)).
