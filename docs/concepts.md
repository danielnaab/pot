---
status: working
---
# Concepts

pot runs one command as a **run**, under one policy that it composes at each start, and enforces it with a
**backend**: bubblewrap on Linux, Seatbelt on macOS (weaker). A command can change the repository it works on but
cannot read the person's keys, reach hosts the person did not grant, or change what decides the sandbox. The full
model is in [pot](reference/pot.md#the-model); this page is the orientation.

## Where authority lives

- **Root:** the host-side authority of one person: contexts, root kits, grants, bindings, backends. A root is a
  home-manager configuration (`programs.pot`, from `homeManagerModules.pot`), a flake that calls `pot.lib.mkRoot`
  (`pot init DIR` writes a minimal one), or pot's default root (`nix run github:danielnaab/pot`). A run never
  writes its root in place.
- **Grant:** a person's approval of a kit's request set, recorded by the kit's identity and a digest of its
  requests in the root's grants file ([grants/1](../contracts/grants-1.md)). `pot grant` writes it, at a terminal;
  an agent, a script or a run cannot. A kit that asks for more stops until it is granted again.
- **Binding:** how the root carries out a granted capability, such as a credential from a file or `nix` through
  the host's daemon. A grant says *whether*; a binding says *how*.
- **Context:** where a run starts from. `work` writes the repository in place, `dispatch` works in a throwaway
  clone whose commits come back for review, `inspect` reads the machine and changes nothing, and `host` has no
  sandbox. A context carries no reach of its own; it may include the root's standing kits.

## What a run needs: kits

A **kit** is data: tools and fixed values that a run sees, paths it protects, and **requests** for reach (a host
path, a network host, a variable, persistent state, or a **capability** such as `nix` or `git.fetch`). A request
does nothing until it is granted. A kit's schema is [kit/1](../contracts/kit-1.md). By place:

- **Root kit:** defined in the root, such as an agent kit (`pot.kits.claude-code`, `pot.kits.pi`) or a standing
  kit that a context includes. Identity `root:<name>[@<instance>]`.
- **Repo kit:** a repository's own needs, the bundle's `self`, or the root's `repoKits."<origin>"` for a repository
  with no bundle. Identity `repo:<normalised origin>`.
- **Submodule kit:** the kit of a git submodule pinned at a release tag, which the bundle carries beside the repo
  kit. pot verifies the release from git facts (the gitlink is a tag at the submodule's URL), and keys its grant by
  that URL, so a move of the submodules folder asks nothing again. Identity `git:<url>`.

**Reach follows the command.** `pot claude` gets the claude-code kit's grant, the repo kit's, and the standing reach
of the root kits its context includes. A kit's declared commands run as **wrapped commands**: each under its own
kit's policy, beside an agent's run as a **sibling**, never inside it ([Declare kit commands](guides/kit-commands.md)).

## How a repository presents its kits

A repository's kits reach pot as a **bundle**, its flake's `packages.<system>.pot-kits`
([pot-kits/1](../contracts/pot-kits-1.md)), which pot's own library writes: any flake may call
`pot.lib.pot.kitsFor` ([pot-lib/1](../contracts/pot-lib-1.md)), and that flake is the bundle's **producer**.
tuteur's mkFlake (graft before 0.1.0) is one: when a consumer's flake declares `inputs.pot`, it hands each
provider's kit to `kitsFor` unread, and protects its own files. pot never reads a producer's files or conventions.

pot finds the flake from git facts: the nearest `flake.nix` at or above a submodule path that `.gitmodules` names
at `HEAD`. A repository with no submodules has no repo kit from its flake, and its flake is never evaluated. A
repository with submodules and a flake but no `pot-kits` gets none of its producer's kits or protections, and pot
says so once at each start.

## What a run leaves

Each run has a record computed from its log on the host ([runs/1](../contracts/runs-1.md)): `pot runs` lists runs,
and `pot review` shows those that need a person, such as a dispatch with commits to fetch. A run's commits carry a
`Pot-Run:` trailer ([trailer/1](../contracts/trailer-1.md)), a claim that readers such as tuteur's `ci signers`
check; the run's log is the evidence. **Output is untrusted:** read the tree after a run, or a fetched dispatch
branch, as you would a stranger's pull request.

**Protected paths** are read-only in a run: what decides the sandbox (`flake.nix`, `flake.lock`, `.gitmodules`, the
flake directory pot evaluates), what the host runs (`.envrc`, `justfile`, git hooks and config), agent
configuration, and whatever the bundle and the kits protect.

## Sources

- [pot](reference/pot.md): the model, roots, kits, grants, contexts, protected paths
- [pot's contracts](../contracts/README.md)
- [core/pot.py](../core/pot.py): finding the flake, the bundle, runs and review
- [core/compose.py](../core/compose.py): composition, grants, the policy
- [lib/kits-for.nix](../lib/kits-for.nix): `kitsFor`, the bundle
- graft's [concepts](https://github.com/danielnaab/graft/blob/v2/docs/concepts.md), "What runs in a sandbox",
  which this page replaces for pot
