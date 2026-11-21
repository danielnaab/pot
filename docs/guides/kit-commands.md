---
status: working
---
# Declare kit commands, recipes and hook commands

A kit declares the programs it ships as `commands`. The bundle that pot's library builds from it
([pot-kits/1](../../contracts/pot-kits-1.md)) holds a **wrapper** of each command's name, so `pot` runs your
command under its own policy: on the host, from `just`, from a git hook, and beside an agent's run, as a sibling the
host starts. The wrapped command gets only what it requests and the repo kit's reach, never the agent's token or MCP,
nor the context's standing reach. Its `recipes` become just recipes in the bundle's `kits.just`.

This page is about the kit. How a kit reaches the bundle is the producer's: with tuteur, a provider returns `kit`
from its `tuteur.nix`, the consumer's flake declares `inputs.pot` so that tuteur's mkFlake calls
`pot.lib.pot.kitsFor`, and the consumer's `justfile` imports the `kits.just` that tuteur writes beside its
submodules. See [tuteur](https://github.com/danielnaab/tuteur). A flake of your own can call `kitsFor` directly
([pot-lib/1](../../contracts/pot-lib-1.md)).

## Before you begin

- A kit, in a file that your producer hands to `kitsFor` (the bundle names that file in pot's messages).
- `recipes` in the attrs form and `hook` are kit 1.2. pot reads 1.2 from graft v2.17.0 on, and every pot release
  since 0.1.0. Under a root whose pot predates wrapped commands (before graft v2.18.0), a wrapper runs the program
  unwrapped, with one line on stderr ([Wrapped commands](../reference/pot.md#wrapped-commands)).

## Steps

1. Declare each program as a command, with the least `access` it needs and no `requests` unless it reaches
   something outside the workspace and the store. A kit with no requests needs no grant: it runs with no prompt.

   ```nix
   # notebook-kit's kit, trimmed
   kit = {
     version = "1.2";
     commands = {
       check-kb = { program = lib.getExe scripts.check-kb; access = "read";
                    description = "Check links, lifecycle status, and Sources"; };
       nb-index = { program = lib.getExe scripts.nb-index; access = "write"; };
       nb-pre-commit = { program = lib.getExe preCommit; access = "read"; hook = "pre-commit";
                         description = "markdownlint on staged Markdown, then check-kb"; };
     };
     recipes = {
       check-kb = { run = "check-kb"; description = "Check links, lifecycle status, and Sources"; };
       index = { run = "nb-index"; description = "Rebuild the indexes"; };
     };
   };
   ```

   `program` is a file in the store (`lib.getExe drv`), not a package. A recipe's `run` starts with one of the
   kit's own commands; the rest are literal words (letters, digits and `. _ / = : , @ + % -`, one space apart). A
   recipe's name is letters, digits, `_` and `-`, starting with a letter.

2. Make a hook command call other programs by store path, not through `just` or another wrapper, so one commit
   is one sandbox start: put them in the hook script's `runtimeInputs`.

3. Try it in a repository whose bundle holds the kit. Check:
   - `head -3 "$(nix build --no-link --print-out-paths .#pot-kits)/wrappers/bin/check-kb"` is a wrapper (its
     comment says `pot (pot-kits/1)`);
   - the bundle's `kits.just` starts with pot's header, `# Generated from the kits' recipes (pot-kits/1) by the
     bundle's producer, which writes and compares it:`, and has one recipe per entry of `recipes`;
   - `just --list` shows the recipes, once the producer has written `kits.just` where the `justfile` imports it;
   - `pot explain claude` lists your commands on its `children` row as `sibling`.

4. Release a minor version of the kit's repository: a new field, nothing removed.

## Keep older repositories working

- Keep a hand-written `.just` file and hook script while repositories whose producer writes no `kits.just` use
  them. With `set allow-duplicate-recipes`, just keeps the first of two recipes with one name, so a hand-written
  one imported first runs. Make it call the tool from the repository's tools, so it runs the same wrapper.
- Other Nix code uses your unwrapped tools, as before. Never call a wrapper from a Nix build or a check.

## Check it worked

- On the host, `check-kb` in the repository runs quietly; `pot runs --all` lists it (plain `pot runs` hides
  wrapped runs that exited 0).
- Inside `pot claude`, the same command is a sibling: it cannot reach the agent's hosts, and the agent waits for it
  and gets its exit code.
- `git commit` in the repository runs your hook command.
- On macOS, the command runs under Seatbelt, on the host and as a sibling alike. A row that needs reach the run
  lacks is refused, naming your recipe (`run it on the host (just check-kb)`), so keep the recipe's name meaningful.
- A sibling's standard input and output are the caller's terminal or pipes; a file redirected into it arrives
  through a pipe. Its `/tmp` is its own: don't hand it the caller's temporary files.

## If something goes wrong

- **`pot: FILE: recipes.N: runs X, which is not one of this kit's commands (A, B)`.** Declare `X` as a command,
  or point the recipe at one.
- **`pot: FILE: recipes.N: literal words only (letters, digits and . _ / = : , @ + % -, one space apart)`.** A recipe passes the caller's
  arguments itself (`"$@"`); write only fixed words.
- **`pot: FILE: recipes.N: a recipe's name is letters, digits, _ and - (starting with a letter)`.** Rename it.
- **`… needs X, which this run (claude) lacks: run it on the host (just RECIPE)`.** Your command requests reach
  the agent's run lacks. Drop the request if it is not needed; otherwise that command runs on the host only.
- **`pot: kitsFor: two recipes are named X (FILE and FILE)`,** or `two kits ship a command named X (self and
  scions.a): a wrapper has one name`. Two kits in one bundle use one name. Rename yours.
- **`pot: kitsFor: a command name is letters, digits, . _ and - (X)`.** Rename the command.
- **`pot: X runs without pot (your root's pot predates wrapped commands: take pot 0.1.0 or later in your root
  (home-manager switch))`.** The root's pot is too old to run wrappers; update the root.

## Sources

- [kit/1](../../contracts/kit-1.md): `commands`, `recipes`, `hook`
- [pot-kits/1](../../contracts/pot-kits-1.md): the wrapper, `kits.just`, the manifests
- [pot-lib/1](../../contracts/pot-lib-1.md): `kitsFor`, which a producer calls
- [lib/kits-for.nix](../../lib/kits-for.nix): what kitsFor builds
- [pot's reference: Wrapped commands](../reference/pot.md#wrapped-commands)
