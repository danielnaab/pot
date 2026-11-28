# pot

pot runs a command, such as a coding agent, in a sandbox whose reach you grant. A repository's kits ask for
what their commands need (a host, a variable, a folder); your root, the place where your grants live, decides what
they get; everything else is hidden, read-only or unreachable. It runs on Linux with bubblewrap, and on macOS with
Seatbelt, which is weaker ([On macOS](docs/reference/pot.md#on-macos)).

pot is experimental, and below 1.0: each `0.y` is its own series, so a `0.(y+1)` may change a contract
([contracts](contracts/README.md)). Before 0.1.0, pot shipped inside [graft](https://github.com/danielnaab/graft);
0.1.0 reads and writes exactly what graft v2.21.0's pot does (the same grants, run records and state).

## Try it

With Nix (flakes enabled) and Claude Code on your `PATH`:

```
claude setup-token          # once; save the token in ~/.config/pot/claude-token-personal (chmod 600)
cd ~/src/some-repository
nix run github:danielnaab/pot/v0.5.0 -- claude
```

That is pot's default root: Claude Code as `claude-code@personal`, its token file, and Nix through the host's
daemon. The first run asks you to grant what the kits request, on your terminal. `pot explain claude` shows what a
run would see and reach, `pot why PATH|HOST` explains one item, and `pot doctor` checks the machine.

## A root of your own

A root is a flake that builds your `pot` with `pot.lib.mkRoot`. `pot init DIR` writes a minimal one to start from:

```nix
{
  inputs.pot.url = "github:danielnaab/pot/v0.5.0";
  inputs.nixpkgs.follows = "pot/nixpkgs";
  outputs = { pot, nixpkgs, ... }: {
    packages.x86_64-linux.default = pot.lib.mkRoot {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      grantsFile = "~/src/pot-root/grants.json";
      kits."claude-code@personal" = pot.kits.claude-code { pkgs = nixpkgs.legacyPackages.x86_64-linux; binary = "host"; };
      defaultInstance.claude-code = "personal";
      bindings.credential."api.anthropic.com kit=claude-code@personal" = {
        via = "env"; from.file = "~/.config/pot/claude-token-personal"; refresh = "claude setup-token"; };
    };
  };
}
```

The agent kits are `pot.kits.claude-code` and `pot.kits.pi`, and `pot.apis` points one at an API
(`pot.apis.anthropic`, `pot.apis.bedrock { region; }`, `pot.apis.gateway { url; }`).

### In home-manager

```nix
{
  inputs.pot.url = "git+https://github.com/danielnaab/pot?ref=refs/tags/v0.5.0&exportIgnore=1";
  # in your home configuration's modules:
  #   inputs.pot.homeManagerModules.default    (also named homeManagerModules.pot)
}
```

```nix
programs.pot = {
  enable = true;
  grantsFile = "~/.workstation/pot/grants.json";   # tracked with the rest of your root
  kits."claude-code@personal" = inputs.pot.kits.claude-code { inherit pkgs; };
  defaultInstance.claude-code = "personal";
};
```

The module puts the root's `pot` on your `PATH` and links `~/.config/pot/launcher` to it, which kit commands'
wrappers use. Its options are mkRoot's ([Home-manager options](docs/reference/pot.md#home-manager-options)).

## A repository's kits

A repository asks for reach through its flake's `packages.<system>.pot-kits`, a bundle that pot's own library
builds ([pot-lib/1](contracts/pot-lib-1.md), [pot-kits/1](contracts/pot-kits-1.md)). pot finds that flake from the
repository's git submodules, builds the bundle at run start, and reads only it. Any flake can produce one:

```nix
{
  inputs.pot.url = "github:danielnaab/pot/v0.5.0";
  outputs = { pot, ... }:
    let
      system = "x86_64-linux";
      pkgs = pot.inputs.nixpkgs.legacyPackages.${system};
    in {
      packages.${system}.pot-kits = pot.lib.pot.kitsFor {
        inherit pkgs;
        # the repo kit: what every run here may ask for, and what it may not touch
        self = { file = "flake.nix"; kit = { protect = [ "AGENTS.md" ]; requests.network = [ "api.example.org" ]; }; };
        # a submodule kit (a git submodule at a release tag), by its folder name
        scions.style = { file = "flake.nix (style)"; kit = { requests.network = [ "lint.example.org" ]; }; };
        files = [ "vendor" ];   # protected in every run here
      };
    };
}
```

[tuteur](https://github.com/danielnaab/tuteur), which pins other repositories as submodules at release tags, is
one producer: its `mkFlake` calls `kitsFor` when the repository declares `inputs.pot`. A repository with submodules
and a flake but no `pot-kits` runs with none of its own kits or protections, and pot says so at start.

## Documentation

| Page | For |
|---|---|
| [Run commands in pot](docs/guides/run-in-a-sandbox.md) | Set up a root, grant, run Claude Code |
| [Work in a clone](docs/guides/dispatch.md) | `pot dispatch` and `pot review` |
| [Declare kit commands](docs/guides/kit-commands.md) | A kit's commands, recipes and hooks |
| [Concepts](docs/concepts.md) | Roots, kits, grants, contexts, runs |
| [pot](docs/reference/pot.md) | The reference: every option, verb, message and limit |
| [pot: siblings](docs/reference/pot-siblings.md) | Kit commands as siblings of a run |
| [Contracts](contracts/README.md) | The formats pot shares: kit/1, pot-lib/1, pot-kits/1, grants/1, run-env/1, runs/1, trailer/1 |
| [Decisions](docs/decisions.md) | Why pot works this way |
| [Development](docs/development.md) | Change pot, test it, release it |
| [CHANGELOG](CHANGELOG.md) | Releases, from 0.1.0 |
