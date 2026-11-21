---
status: working
---
# pot-lib/1: kitsFor

| | |
|---|---|
| Version | 1 |
| Producer | pot: [lib/kits-for.nix](../lib/kits-for.nix), the flake output `pot.lib.pot.kitsFor` |
| Reader | any producer of a repository's bundle, once per system: tuteur's mkFlake (through the repository's own `inputs.pot`), or a flake of your own |
| Owner | pot |

The only Nix edge between a producer and pot. A producer hands pot its kits, each with the file it is written in,
and its tools. pot validates each kit and writes the bundle that a run reads ([pot-kits/1](pot-kits-1.md)). The
producer need read no field of a kit.

**Open to any producer.** Nothing in kitsFor knows tuteur, graft or any convention of theirs: a repository's own
`flake.nix` may call it ([below](#a-producer-of-your-own)). The attribute is `lib.pot.kitsFor` in pot's flake, the
path graft's flake gave it while pot shipped there, so a producer written against graft's flake needs only its
input renamed. The producer's pot (the repository's `inputs.pot`) and the root's pot are pinned separately and meet
only through [pot-kits/1](pot-kits-1.md); the producer's pot need not follow the root's.

## Shape

```nix
pot.lib.pot.kitsFor {
  pkgs;                # the system's nixpkgs: the bundle's tools and its python come from it
  self ? null;         # the repo kit: { kit = <kit>; file = "flake.nix (perSystem)"; }, or null
  scions ? { };        # scions.<name> = { kit = <kit>; file = ".patterns/<name>/tuteur.nix"; }: submodule kits
  tools ? [ ];         # packages for the bundle's bin/; kitsFor adds the repo kit's own packages
  files ? [ ];         # what the producer has pot protect in every run here
  recipes ? { };       # the producer's own recipes, for kits.just (from v2.18.0)
}                      # → a derivation: packages.<system>.pot-kits, with passthru.wrappers
```

| Argument | Type | Meaning |
|---|---|---|
| `pkgs` | nixpkgs | the system's packages |
| `self` | `{ kit; file; }` or `null` | the repo kit. `kit` is a [kit/1](kit-1.md) value. `file` is a string, where the kit is written, as pot's messages name it. |
| `scions.<name>` | `{ kit; file; }` | each submodule kit (a git submodule at a release tag), by its folder name. pot ties it to its submodule from `.gitmodules`, never from this name alone ([kit/1: Identity](kit-1.md#identity)). The argument's name is version 1's. |
| `tools` | list of packages | the workspace's tools, for the bundle's `bin/`. A producer passes its merged tools and its own command. |
| `files` | list of strings | what every run here sees read-only: paths relative to the flake directory, or `**/NAME` anywhere in the repository. A producer passes its own files (tuteur: `**/tuteur.nix`, `**/graft.nix` and its submodules folder). |
| `recipes` | attrs, default `{ }` | the producer's own recipes, in [kit/1's recipe form](kit-1.md#recipes), written into the same `kits.just` as the kits' recipes (tuteur's are `outdated`, `upgrade` and a check). Unlike a kit's, a producer's recipe may run any tool. Written into `kits.just` since v2.18.0. Another shape: `pot: kitsFor: recipes.X is "<command> <args>" or { run; description; }`. |

The result has `passthru.wrappers`, the declared commands' wrappers as one package, which a producer puts over the
programs in its tools (tuteur's mkFlake does, keeping the tools without them as `tools.passthru.unwrapped`), and
`passthru.kitsJust`, the text of `kits.just`, a plain string with no store paths, so a producer can write the file
and compare it from `nix eval` alone. In graft v2.17.x `wrappers` was an empty package.

tuteur's mkFlake's call, for a consumer whose submodules folder is `.patterns`:

```nix
inputs.pot.lib.pot.kitsFor {
  inherit pkgs;
  self = { kit = perSystem.kit; file = "flake.nix (perSystem)"; };
  scions.meeting-kit = { kit = <its tuteur.nix's kit>; file = ".patterns/meeting-kit/tuteur.nix"; };
  tools = [ <the merged tools> <the repository's tuteur> ];
  files = [ "**/tuteur.nix" "**/graft.nix" ".patterns" ];
}
```

### A producer of your own

Any flake can give `packages.<system>.pot-kits`. This one has a repo kit, one submodule kit and its own recipe:

```nix
{
  inputs.pot.url = "github:danielnaab/pot/v0.1.0";
  outputs = { pot, ... }:
    let
      system = "x86_64-linux";
      pkgs = pot.inputs.nixpkgs.legacyPackages.${system};
      lint = pkgs.writeShellScriptBin "lint" "echo linted";
    in {
      packages.${system}.pot-kits = pot.lib.pot.kitsFor {
        inherit pkgs;
        self = { file = "flake.nix"; kit = { protect = [ "AGENTS.md" ]; tools = [ pkgs.jq ]; }; };
        scions.style = { file = "vendor/style/kit.nix"; kit = import ./vendor/style/kit.nix { inherit pkgs; }; };
        tools = [ lint ];
        files = [ "vendor" ];
        recipes.lint = { run = "lint --all"; description = "Lint everything"; };
      };
    };
}
```

pot builds it at run start when the repository's submodules put this `flake.nix` above them
([pot-kits/1](pot-kits-1.md#finding-the-flake-directory)); `nix build .#pot-kits` shows what a run will read.

The derivation fails when a kit is invalid, with pot's message, which names the kit's `file`:
`pot: <file>: <field> is not a kit field (…)`. So a bad kit fails the producer's build, not a run. A malformed
argument fails evaluation: `pot: kitsFor: <self|scions.NAME> is { kit; file; } (file: a string naming where the kit
is written)`.

## Compatibility

- kitsFor takes no `...`: an argument it does not know is Nix's own error. Version 1 adds arguments only with a
  default, so a producer written for an earlier pot keeps evaluating.
- The repository's lock pins this library, and a root pins its own pot, so the two pots meet only through
  [pot-kits/1](pot-kits-1.md).
- **A producer reads no pot format.** Until graft v2.17.0, mkFlake built v2.15.0's `graft-kits` from the `kit.json`
  that kitsFor wrote; that bundle is gone, and with it the one place a producer read pot's shapes.
- **No `pot-kits`, no kits.** A producer that calls kitsFor only when the repository brings pot (tuteur's mkFlake
  with no `inputs.pot`) gives no bundle; pot then runs with no repo kit, submodule kits or protections from it, and
  says so ([pot-kits/1](pot-kits-1.md#finding-the-flake-directory)).
- **A kit with `packages` fails the build:** `pot: FILE: packages is tools' old name, gone in v2.17.0: give tools`
  ([kit/1](kit-1.md#compatibility)).

## Sources

- [lib/kits-for.nix](../lib/kits-for.nix): the function, its arguments and the bundle
- [tests/run.sh](../tests/run.sh): the `kits` section calls kitsFor directly, as a producer of its own would
- [graft's lib/mk-flake.nix at v2.21.0](https://github.com/danielnaab/graft/blob/v2.21.0/lib/mk-flake.nix): the call, before tuteur
