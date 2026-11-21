---
status: working
---
# pot's contracts

pot meets other parties through seven contracts. A contract gets a version when its producer and its reader are
pinned by different parties: a provider's tag and a repository's producer, a repository's `inputs.pot` and a root's
pot, or the pots of two machines that share one grants file. A file that one build both writes and reads stays
internal, so the backend protocol (`launch.json`) and the root manifest have no version.

**Who owns a contract.** pot owns every contract that pot touches. A producer of kits, such as tuteur, owns its own
formats (tuteur's `provider` file and `status/1`), which pot never reads. Nothing a producer writes is read by pot:
the bundle a run reads is written by pot's own Nix library, which the producer calls.

**Versions before 0.1.0.** pot shipped inside graft until graft v2.21.x, so a version such as `v2.17.0` in these
contracts is graft's release that changed pot. pot's own versions start at 0.1.0, which reads and writes exactly
what graft v2.21.0's pot does ([CHANGELOG](../CHANGELOG.md)).

**What a contract is.** Prose, a JSON Schema (2020-12) and golden fixtures, all owned by the producer: the prose
says what the values mean, the schema what this release emits, and the fixtures are what readers test against.
The producer tests that what it really emits validates; a reader tests against the fixtures, never against the
producer's live output. `runs/1` and `trailer/1` have all three; the others are prose until their next change.

| Contract | Version | Producer | Reader | Owner |
|---|---|---|---|---|
| [`kit`](kit-1.md) | 1.x (1.0, 1.1 and 1.2) | providers (in tuteur's `tuteur.nix`, graft's `graft.nix`), a producer's repo kit, root kits | pot | pot |
| [`pot-lib`](pot-lib-1.md) | 1 | pot (`pot.lib.pot.kitsFor`) | any producer of a bundle: tuteur's mkFlake, through the repository's `inputs.pot`; a flake of your own | pot |
| [`pot-kits`](pot-kits-1.md) | 1 | `pot.lib.pot.kitsFor`, in the repository's evaluation | pot, at run start | pot |
| [`grants`](grants-1.md) | 1, canonical 1 (frozen) | `pot grant` | pot, on each machine | pot |
| [`run-env`](run-env-1.md) | 1 | pot's backends | tuteur's check inside a run, kit hooks, any program in a run | pot |
| [`runs`](runs-1.md) | 1 | pot | workmux, Windmill jobs, later grove | pot |
| [`trailer`](trailer-1.md) | 1 | `pot trailer` | tuteur's `ci signers`, later grove, any program that reads commits | pot |

## How the arrows point

An arrow means "depends on, or reads the contract of". pot never points at a producer: it reads only the bundle
its own library wrote, and git facts.

```
providers ──▶ a producer (tuteur's mkFlake) ──pot-lib/1 (the repository's inputs.pot)──▶ pot.lib.pot.kitsFor ──writes pot-kits/1──▶ pot
tuteur check, inside a run ──run-env/1 (pot why URL --json)──▶ pot
tuteur ci signers ──trailer/1 (git log)──▶ the commits pot trailer marked
pot ──▶ git facts: .gitmodules and gitlinks at HEAD, the tags at a submodule's URL
```

A repository whose flake has submodules but no `pot-kits` (its producer was given no `inputs.pot`) gets none of its
kits or protections in a run, and pot says so once at run start: `pot (CONTEXT): DIR's flake has no pot-kits; its
own kits and protections are not applied` ([pot-kits/1](pot-kits-1.md#finding-the-flake-directory)).

## How a contract changes

- **Reader first.** A reader accepts a new version in a release before any producer writes it.
- **Values, not fields.** Within one version, a release may add values: a capability name, a guarantee, a verb,
  a source in `policy.json`. A new field, or a changed meaning, is a new version.
- **A compatibility shim is deprecated in the release that adds it and removed in the next.** v2.16.0 added six;
  v2.17.0 removed them. Each old form now fails with an error that names its replacement:

  | Removed in v2.17.0 | Contract | Use |
  |---|---|---|
  | `packages.<system>.graft-kits`: mkFlake no longer writes it, and pot no longer reads it | `pot-kits/1` | `packages.<system>.pot-kits` (any graft v2.16.0 or later) |
  | `kit.packages`, at the source: kitsFor and mkRoot refuse it. pot still reads it in a bundle that graft v2.16.x built | `kit/1` | `kit.tools` |
  | `--account NAME` | pot's command line | `--instance NAME`, `CMD@NAME` |
  | `defaultAccount` | mkRoot, `programs.pot` | `defaultInstance.<kit>` |
  | the claude-code kit's `account` | the kit's arguments | (nothing: the root's `kits` key names the instance) |
  | the claude-code kit's `provider` and `region` | the kit's arguments | `api = pot.apis.<name>` |

- **pot's split waited for two quiet releases.** pot moved to its own repository only once `kit/1` and
  `pot-kits/1`, its two seams with graft, had held unchanged for two releases: graft v2.18.0 and v2.19.0 changed
  neither. pot 0.1.0 is that split, with every contract unchanged; `trailer/1` is new prose for a format v2.21.0
  already wrote. One text changed with it: `kits.just`'s header (part of pot-kits/1), which no longer names a
  producer ([pot-kits/1](pot-kits-1.md#shape)).

## Sources

- [lib/kits-for.nix](../lib/kits-for.nix): `pot-lib/1` and the bundle it writes
- [core/pot.py](../core/pot.py): finding the flake directory, reading the bundle, `pot why --json`, `pot trailer`
- [core/compose.py](../core/compose.py): kits, the bundle's checks, grants, the policy
- [graft's lib/mk-flake.nix at v2.21.0](https://github.com/danielnaab/graft/blob/v2.21.0/lib/mk-flake.nix): the producer's side of `pot-lib/1`, before tuteur
