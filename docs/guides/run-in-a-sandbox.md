---
status: working
---
# Run commands in pot

Run Claude Code, or any command, with pot, so that it can change your repository but cannot read your keys,
reach hosts that you did not grant, or change what decides the sandbox. `pot` is experimental in 0.1.0. The terms
are in [pot](../reference/pot.md#the-model).

## Before you begin

- Linux, with unprivileged user namespaces (the default on NixOS, Debian, Ubuntu and Fedora), or macOS, where pot
  uses Seatbelt and is weaker: no process or network isolation ([On macOS](../reference/pot.md#on-macos)).
- A root, the place where your grants live ([Set up a root](#set-up-a-root)).
- Claude Code installed on the host.
- tmux, if you want to see denials while the agent runs. Without it, you see them when the run ends.
- The steps use `team-kb`, a repository in `~/src`, a home-manager root in `~/.workstation`, and the account
  `personal`.

## Steps

1. Make a login token for the account, once per account:

   ```
   claude setup-token
   ```

   It prints a long-lived token. Save it in the file that your root's `credential` binding names, readable only by
   you, such as `~/.config/pot/claude-token-personal` (`chmod 600`). The file holds the token alone, or a line
   `CLAUDE_CODE_OAUTH_TOKEN=…`. Do not paste it on a command line. `pot` hides the file from every run.

2. Start Claude in the repository:

   ```
   cd ~/src/team-kb
   pot claude
   ```

3. The first time, `pot` shows the grant prompt before the run. It lists what the claude-code kit asks for,
   ordinary items together and each powerful item on its own:

   ```
   claude-code@personal   root kit (root:claude-code@personal), for every run that uses it   not granted
     ordinary:
       + capability status.relay  bound: workmux relay
       + network api.anthropic.com:443
       + network claude.ai:443
       + read ~/.local/share/claude
       + state claude
     grant these? [y/N]
   ```

   Type `y` for each set that you accept. Each powerful item, such as the login `credential`, asks on its own:
   `grant? [y/N]`. The repo kit and any submodule kits that need a grant come next, in the same prompt. The prompt
   appears only at a terminal; elsewhere the run stops with exit 125 and `pot grant KIT`.

4. Claude starts. The startup line names what differs from the usual shape, and `pot explain`:

   ```
   pot: work · team-kb · pot explain
   ```

5. `pot` prints `wrote ~/.workstation/pot/grants.json. Commit it with your root's next change.` Commit it:

   ```
   git -C ~/.workstation add pot/grants.json
   git -C ~/.workstation commit -m "pot: grant claude-code@personal"
   ```

   From now on, `pot claude` starts with no prompt, until a kit asks for more.

## Check it worked

From the repository's top, paste this. It prints `ok` only if the run is in `work`, can write the repository,
cannot write `justfile`, and cannot see `~/.ssh`:

```sh
pot bash -c 'test "$POT" = work && touch .pot-check && ! touch justfile 2>/dev/null && ! test -e ~/.ssh' &&
  test -e .pot-check && rm -f .pot-check && echo ok
```

If the repository has no `justfile`, write another [protected path](../reference/pot.md#protected-paths),
such as `flake.nix`.

## Choose a context

| You want to | Run | The run |
|---|---|---|
| Work on this repository | `pot claude` | writes the repository in place, reaches only what is granted |
| Diagnose the machine, change nothing | `pot inspect claude` | reads the machine except secrets and hidden paths, writes nothing, reaches only the model API |
| Hand a task off, or change a root repository | `pot dispatch claude` | works in a clone in pot's state; you review and fetch its commits ([Work in a clone](dispatch.md)) |
| Do what only the host may do | `pot host CMD`, or plain `claude` | has no sandbox |

`inspect` diagnoses; the host repairs. `sudo`, `systemctl restart`, `docker run` and `home-manager switch` are
host-only. To carry a diagnosis from a run to the host, as quoted data: `pot host --resume RUN claude`.

In workmux, set the agent command to `pot claude`, and turn off workmux's own Docker sandbox
(`workmux.sandbox.enable = false`; the home-manager module asserts it). The `status.relay` capability keeps the
pane's status working.

## Kit commands inside a run

A kit's commands (`check-kb`, `just check`, the pre-commit hook) are [wrapped](../reference/pot.md#wrapped-commands):
inside `pot claude` each runs beside the agent's run, as a sibling the host starts under the command's own policy,
so it cannot reach the agent's model API, token or MCP. The agent waits for it and gets its exit code; `^C` reaches
it. `git commit` inside the run runs the repository's hook command the same way, on Linux and macOS, and a failing
hook aborts the commit. `pot explain claude` lists them on its `children` row. A command that needs reach the
agent's run lacks is refused, naming where to run it:

```
pot (work): meeting-kit needs WM_TOKEN, git.push, which this run (claude) lacks: run it on the host (just meeting-run)
```

## When something is denied

A denial reaches you in three ways. Under tmux, a message shows on the agent's pane:

```
pot (work): the run tried pypi.org:443 (not in this run) · pot why pypi.org
```

The agent reads it from the denials file, through its hook, and tells you in its own words. When the run ends,
the exit line lists it:

```
pot (work): the run tried pypi.org:443 (not in this run).
  one run:   pot --net pypi.org …
  permanent: requests.network in the repo kit (flake.nix, perSystem)
             or programs.pot.kits.standing.requests.network, for every repository (home-manager switch)
```

1. Ask `pot` where the item would come from:

   ```
   pot why pypi.org
   ```

2. To allow it for one run, give the flag that `pot why` names:

   ```
   pot --net pypi.org claude --continue
   ```

   The exit line names the permanent fix for each flag that you used.

3. To allow it in every run of this repository, add it to the repo kit (see [Add a repo kit](#add-a-repo-kit)):

   ```nix
   kit.requests.network = [ "pypi.org" ];
   ```

   To allow it in every repository, add it to `standing`, the context's included kit
   (`programs.pot.kits.standing`, below), then `home-manager switch`.

4. Run `pot grant`, and accept the new item. Commit the grants file.

## When a kit asks for more

When an upgrade gives a kit new requests, or a new source for a powerful one, the next run shows the difference.
At a terminal, it is the grant prompt, inline: accept or refuse each item, and the run goes on. Elsewhere (an
agent, a script), the run stops with exit 125 and the difference:

1. Read the difference that `pot` printed, or run `pot grant --diff`. A stopped run is listed by `pot runs` as
   `stopped`, and a workmux pane that started it shows `waiting`.
2. If you accept it, run the command that `pot` printed (`cd ~/src/REPO && pot grant`) at a terminal: with no
   argument it offers every kit here that is stale, and every kit a stopped run waits for, showing only what
   changed. The kits whose source changed and that ask for no more are granted together, on one answer; the others
   are asked one by one. Or name one, `pot grant KIT`, with the short name
   that `pot` printed (`claude-code` names every instance). Add `--commit` to commit the grants file at once (only
   that file; it never pushes), or commit it with your root's next change. Otherwise, take back the upgrade.

Runs that the kit's requests do not apply to still start, with a note on the startup line:
`meeting-kit asks for more: pot grant`.

## Root repositories

Your root's own repository (`~/.workstation` here) decides what every run may do, so `pot` never writes it in
place. `pot claude` refuses there:

```
pot (work): ~/.workstation is a root: pot dispatch CMD (its commits come back for review), or pot inspect CMD
```

- To read it, use `pot inspect claude`.
- To change it, use `pot dispatch claude`: the change comes back as a branch for you to review, fetch and merge
  before `home-manager switch` ([Work in a clone](dispatch.md#change-a-root-repository)).
- In its worktrees, set workmux's agent command in `~/.workstation/.workmux.yaml` to anything but `pot claude`,
  because the project file overrides the global `agent`:

  ```yaml
  agent: claude
  ```

## Add a repo kit

A repository's own needs are its repo kit: the packages its work needs, paths to protect, and requests.

1. Give the repository a `pot-kits` bundle: its flake's `packages.<system>.pot-kits`, from pot's library
   `pot.lib.pot.kitsFor` ([pot-lib/1](../../contracts/pot-lib-1.md)). A producer such as tuteur's mkFlake (graft
   before 0.1.0) writes it for you from `perSystem.kit` when the repository's flake declares `inputs.pot`
   ([tuteur](https://github.com/danielnaab/tuteur)). By hand:

   ```nix
   {
     inputs.pot.url = "github:danielnaab/pot/v0.6.0";
     outputs = { pot, ... }: let pkgs = pot.inputs.nixpkgs.legacyPackages.x86_64-linux; in {
       packages.x86_64-linux.pot-kits = pot.lib.pot.kitsFor {
         inherit pkgs;
         self = {
           file = "flake.nix";
           kit = {
             tools = [ pkgs.nodejs_22 ];
             protect = [ "AGENTS.md" ];
             requests.network = [ "registry.npmjs.org" ];
             requests.capabilities = [ { name = "nix"; } ];   # for a tool that evaluates Nix in the run
           };
         };
       };
     };
   }
   ```

   A repository whose tools evaluate Nix inside a run (`tuteur check` evaluates each submodule's provider file)
   needs the `nix` capability. The capability needs your root to bind it (`nix."*" = { via = "daemon-untrusted"; };`),
   which pot's default root and `pot init`'s root do.

   pot finds this flake from `.gitmodules`: the nearest `flake.nix` at or above the folder that holds the
   submodules. A repository with no submodules has no repo kit from its flake, and pot never evaluates it: give it
   one with the root's `repoKits` (below). A repository with submodules and a flake but no `pot-kits` runs
   without its own kits and protections, and pot says so once at start:
   `pot (work): DIR's flake has no pot-kits; its own kits and protections are not applied`.

2. Read what a run would get: `pot explain claude`. The repo kit's items are labelled `(repo)`, and so are the
   paths that the bundle protects (the producer's own files, such as each `tuteur.nix` and the submodules folder).
3. Run `pot grant`, and accept the repo kit's requests. Commit the grants file in your root.

A powerful grant of a repo kit is fixed to its flake directory as the next commit holds it. When a change under it
moves that tree, the next run stops with exit 125, lists the changed paths and names the command
(`grant it: cd ~/src/REPO && pot grant`), and `pot grant` shows each changed kit's diff and grants them together on
one answer.

For a repository with no `pot-kits` bundle, the root declares its repo kit, keyed by its normalised origin:

```nix
programs.pot.repoKits."github.com/example/site".requests.network = [ "registry.npmjs.org" ];
```

If the repository's `ensure.sh` or `justfile` builds tools with `nix`, make it use the tools that `pot` built
([Move from v2.14.0's sandbox](https://github.com/danielnaab/graft/blob/v2/docs/guides/move-from-the-v2-14-sandbox.md#run-the-repositorys-tools-inside)).

## Set up a root

| Root | Use it when | Start it with |
|---|---|---|
| A home-manager root | You manage your home with home-manager. | `programs.pot` (below) |
| pot's default root | You have no root and want to try `pot`. | `nix run github:danielnaab/pot/v0.6.0 -- claude` |
| A minimal root of your own | You want a root without home-manager. | `pot init ~/src/pot-root`, then the steps that it prints |

pot's default root and the root that `pot init` writes work without home-manager. Each has a
`claude-code@personal` instance (the `claude` on your `PATH`), its token binding at
`~/.config/pot/claude-token-personal` (step 1), and the `nix` relay. pot's default root keeps its grants in
`~/.config/pot/grants.json`. `pot init DIR` writes `DIR/flake.nix`, with its grants file beside
it: track `DIR` in git. A home-manager root keeps its grants file in its own repository, where you review and
commit it.

A home-manager root:

```nix
{ inputs, pkgs, ... }:
{
  imports = [ inputs.pot.homeManagerModules.pot ];
  programs.pot = {
    enable = true;
    grantsFile = "~/.workstation/pot/grants.json";
    rootRepos = [ "~/.workstation" ];
    kits."claude-code@personal" = inputs.pot.kits.claude-code { inherit pkgs; };
    kits.standing.requests.network = [ "pypi.org" "registry.npmjs.org" ];
    tools = [ pkgs.gh ];                    # more commands on every run's PATH
    contexts.work.include = [ "standing" ];
    bindings = {
      "status.relay"."*" = { workmux = true; };
      nix."*" = { via = "daemon-untrusted"; };
      credential."api.anthropic.com kit=claude-code@personal" = {
        via = "env"; from.file = "~/.config/pot/claude-token-personal"; };
    };
  };
}
```

For a second Claude account, or Bedrock, add an instance. Each instance is granted on its own:

```nix
kits."claude-code@work" = inputs.pot.kits.claude-code {
  inherit pkgs; configDir = ".claude-accounts/work";
  api = inputs.pot.apis.bedrock { region = "us-east-1"; };
};
defaultInstance.claude-code = "personal";
```

`pot claude@work` runs that instance, and so does `pot --instance work claude`. With neither, pot picks the
instance whose variables the shell matches (`CLAUDE_CONFIG_DIR=~/.claude-accounts/work` and
`CLAUDE_CODE_USE_BEDROCK=1` pick `@work`), and otherwise the default. The startup line names an instance that is
not the default: `pot: work · team-kb · instance work · pot explain`.

For an Anthropic-compatible gateway that takes a key as a bearer token (v2.17.1), give its url, and bind its key
file to the credential:

```nix
kits."claude-code@gateway" = inputs.pot.kits.claude-code {
  inherit pkgs; configDir = ".claude-accounts/work";
  api = inputs.pot.apis.gateway { url = "https://llm.example.org"; };
};
bindings.credential."llm.example.org kit=claude-code@gateway" = {
  via = "env"; from.file = "~/.config/llm-gateway/key"; };   # the key alone, mode 600
```

The run reaches only the gateway's host, and Claude reads `ANTHROPIC_BASE_URL=https://llm.example.org/anthropic`.
A shell with that `ANTHROPIC_BASE_URL` and the instance's `CLAUDE_CONFIG_DIR` picks `@gateway`.

What the claude-code kit binds from your host into each run, read-only:

- your skills folder: links into the store or into a checkout under `skillSources`, and the real folders that
  `skillDirs` names (default `[ "synced" ]`; `skillDirs = [ ]` binds none);
- your plugins folder, whole (`plugins = false` leaves it out);
- your project memory for the workspace. `MEMORY.md` is a writable copy; the other files are read-only. New
  memory files, and a changed `MEMORY.md`, are listed on the exit line, with a copy in the run's directory, for
  you to read and copy by hand. Nothing is written back to your host memory. Claude is told this when the session
  starts.

Claude's synced claude.ai skills do not load in a run, even though `skills/synced` is bound: Claude loads them
only when it is signed in to claude.ai, and a run signs in with a token. Install the skills you need as ordinary
skill folders: a link into a checkout under `skillSources`, or a real folder that `skillDirs` names.

```nix
kits."claude-code@personal" = inputs.pot.kits.claude-code {
  inherit pkgs;
  skillSources = [ "~/src/agent-skills" ];   # your checked-out skills
  skillDirs = [ ];                           # no synced or hand-placed real folders
  plugins = false;                           # no plugins folder in the run
};
```

For the pi coding agent (v2.19.0), add the `pi` kit. When home-manager writes pi's `models.json` and
`settings.json`, give the kit the same values, so a run has the providers the host has and `/model` switches among
them. Here `config.pi.generated` stands for your own module's computed values (a read-only option it sets):

```nix
kits.pi = inputs.pot.kits.pi {
  inherit pkgs;
  files = config.home.file;                         # AGENTS.md and the other files under ~/.pi/agent
  inherit (config.pi.generated) settings endpoints; # settings.json; the APIs your extensions reach
  providers = builtins.removeAttrs config.pi.generated.providers [ "usai-local" ];   # less one on localhost
  skillSources = [ "~/src/agent-skills" ];
};
bindings.credential."openrouter.ai kit=pi" = {
  via = "env"; from.file = "~/.config/openrouter/key.env"; };   # export OPENROUTER_API_KEY=…, mode 600
```

The run reaches only the providers' hosts, with one credential per key (`pot explain pi` lists them; bind each).
A provider on the host's `localhost` fails evaluation, naming it: leave it out. `pot grant pi` asks once for the
ordinary set, then for each credential on its own. Inside, pi starts offline (no package installs or update
checks), sees your extensions, skills and themes read-only, and keeps its sessions in the run's state. OAuth
logins (`/login`) stay on the host, so give each provider a key, and a `/model` choice lasts one session: set
`defaultModel` in the root ([pi](../reference/pot.md#pi)).

A producer's own command (such as `tuteur`) is not in a run's standard tools. A run in a repository whose bundle
carries it (its `pot-kits` bundle's `bin/`) has it; `tools` gives every other run one too.

Run `home-manager switch`. `pot` is then on `PATH`, and `~/.config/pot/launcher` points at it. The options are
in [Home-manager options](../reference/pot.md#home-manager-options).

## If something goes wrong

- **Start with `pot doctor`.** It checks the backend, user systemd, the grants file, each binding's file (by path,
  never its contents), the instances and which one your shell selects ([pot doctor](../reference/pot.md#pot-doctor)).
- **`pot` fails before the command starts, in Docker,** because bubblewrap cannot create a user namespace.
  Docker's default seccomp profile blocks them. Run `pot` on the host or in a VM. `pot` never runs the command
  without the sandbox.
- **The run stops with exit 125 and a list of requests.** A kit asks for more than its grant, or was never
  granted where no prompt can appear (an agent, a script). Run `pot grant KIT` at a terminal
  ([When a kit asks for more](#when-a-kit-asks-for-more)).
- **A tool prints `CONNECT tunnel failed, response 403`, `tunnel error` or `Could not resolve host`.** The
  sandbox refused the host. Read `/run/pot/denials` inside, or run `pot why HOST`
  ([When something is denied](#when-something-is-denied)).
- **A test fails because `/run/current-system/sw/bin/…` is missing.** In `work`, `/run/current-system` is a
  link whose target the run does not see: without the `nix` capability, a run sees only its closure in the store.
  Make the test use the `PATH` it was given.
- **The startup line says `meeting-kit: gitlink is not a release tag at its URL; its kit is unverified and its grant
  does not apply`.** That submodule is not at a release tag, so the run goes on without its reach. Move it to a release
  (with your producer's upgrade, such as `tuteur upgrade NAME`), or try it on the host with `pot host CMD`.
- **A claude.ai skill is missing inside a run.** Synced claude.ai skills do not load in a run (above). Install the
  skill as an ordinary skill folder.
- **`pot: this root has no backend for darwin`.** Your root was built before Seatbelt's return (before graft
  v2.18.0): update pot and rebuild the root.
- **`pot: check-kb runs without pot (…)`.** A wrapped command found no root's pot to run it (no
  `~/.config/pot/launcher`, as on a Mac with pot off, or a root older than graft v2.18.0), and ran it unwrapped. The
  parenthesis names the fix.
- **`… not in this run's children table …`** (exit 126). A wrapper from another workspace's tools: run it in its
  own repository, or on the host.
- **`… 16 commands already run in this run's tree …`.** pot's cap on siblings at once: wait for one to end.
- **A commit stops at the hook.** Read its lines: a grant (run the `grant it:` line, `cd ~/src/REPO && pot grant`), or `git commit --no-verify` to skip
  the hook once.

## How it works

At start, `pot` finds the kits of the run: the command's, the repository's, its submodules', and the root kits its
context includes. It reads your grants file, and composes the run's policy: what the run sees, its limits, and
the reach that you granted. Then bubblewrap starts the command with only that. Network goes through a proxy on
the host that allows only granted hosts. Nothing that the run writes can widen a later run: the grants file and
`pot`'s state are hidden, and your root's repository is never written in place
([Decisions](../decisions.md#roots-grant-reach-kits-only-ask)). `pot dispatch` runs in a clone instead of your
tree, and its commits come back only when you fetch them ([Work in a clone](dispatch.md)).

## Sources

- [pot](../reference/pot.md): contexts, kits, requests, grants, flags, messages
- [Move from v2.14.0's sandbox](https://github.com/danielnaab/graft/blob/v2/docs/guides/move-from-the-v2-14-sandbox.md)
- [pot-lib/1](../../contracts/pot-lib-1.md): `kitsFor`, which a repository's flake calls
- [pot-kits/1](../../contracts/pot-kits-1.md): how pot finds a repository's flake
- [core/pot.py](../../core/pot.py): the grant prompt, the startup and exit lines
- [kits/claude-code.nix](../../kits/claude-code.nix): the claude-code kit
- [kits/pi.nix](../../kits/pi.nix): the pi kit
- [nix/hm.nix](../../nix/hm.nix): `programs.pot`
- [tests/run.sh](../../tests/run.sh): the `pot` section
