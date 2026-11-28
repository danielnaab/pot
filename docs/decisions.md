---
status: working
---
# Decisions

Why pot is the way it is. The first two decisions were made while pot shipped inside graft (graft v2.14.0 to
v2.22.0), and moved here with it; their version numbers are graft's releases.

## Enforce the boundary in a sandbox, not in rules

Since graft v2.14.0, pot (then graft's sandbox) runs agents and other commands in a sandbox that enforces what they may read, write and
reach. Rules in `knowledge-base.yaml`, `AGENTS.md` and an agent tool's permission list are context: an agent can
misread them, and a program that the agent starts never reads them. A sandbox stops a write or a connection
whatever the agent read.

How it does so changed in graft v2.15.0: [Roots grant reach; kits only ask](#roots-grant-reach-kits-only-ask)
replaces v2.14.0's launcher. These parts of v2.14.0's decision still hold there:

- **A run never writes what decides the sandbox, or what the host runs.** `flake.nix`, the producer's files
  (each `tuteur.nix`, or graft's `graft.nix`), `.gitmodules`, `.envrc`, `justfile`, `.pre-commit-config.yaml`, the git metadata and agent configuration are
  read-only inside, at any depth.
- **The terminal stays the caller's.** A command that shares a terminal could push input into it (`TIOCSTI`).
  A seccomp filter refuses it, whatever the kernel's `dev.tty.legacy_tiocsti` says.
- **Every value names its source.** v2.14.0's `--explain` became `pot explain` and `pot why`.
- **Never fall back unsandboxed.** `pot host` and a granted `unsandboxed` request are the only ways out, and both
  are explicit.

## Roots grant reach; kits only ask

Since graft v2.15.0, `pot CMD` runs a command as a run, under a policy that it composes at each start from **kits** and
the **grants** in the person's **root** ([pot](reference/pot.md#the-model)). A repository, a provider and
an agent each ship a kit: what a run sees, its limits, and the reach it requests. A request does nothing until the
person grants it with `pot grant`, at a terminal, into a grants file that no run can see. A **context** (`work`,
`inspect`, `host`) is where a run starts from.

The design's principles:

1. **Anyone may add see and limits; only the root adds reach.** Reach enters a run through a grant that the root
   holds, or a flag that the person typed for one run. A binding carries out a granted capability; it is not a
   third source.
2. **Approval is fixed to what was approved.** A grant records a kit's request set by digest, and for powerful
   items on code that the root does not pin, its source. Any change stops that kit's runs until it is granted
   again. Asking for less needs no approval.
3. **Nothing a run writes widens a later run, or is taken by the host as fact.** A run never writes a root, its
   grants file or `pot`'s state. `pot` decides who a kit is from host facts (the root's store, a verified release
   tag, the repository's origin), never from what a repository says about itself.
4. **Output is untrusted until it crosses as data.** In place, protected paths are defence in depth: the tree
   after a run is like a stranger's pull request.
5. **Delegation only narrows,** except by an allowance that the root granted.
6. **One policy, many backends, guarantees probed, never weaker in silence.** A context may require guarantees.
   Nothing falls back to a weaker policy, backend or no sandbox unless the root or the person says so.
7. **Every value and denial names its source and its fix.**

Why:

- **A repository granted itself.** In graft v2.14.0, a consumer's `flake.nix` named the scions whose requests applied
  (`sandbox.grant`). The repository that a run works on decided its own reach, and an agent on the host could add
  a grant in a commit. Now only the person's root grants, and a repository's own needs are a kit like any other.
- **A grant by name followed every upgrade.** In v2.14.0, an upgrade pull request that moved a granted scion
  applied its new requests too, shown as one gitlink. Now a grant is fixed to a digest, and a kit that asks for
  more stops until it is granted again.
- **Reach belongs to the command, not the repository.** One `NODE_OPTIONS` or `ANTHROPIC_BASE_URL` from a kit
  would steer the agent that holds the reach. So what configures a program follows that program's command, and a
  repo kit's tools are its only see items that reach every run.
- **The agent's needs were declared in every repository.** v2.14.0's guide made each repository list Claude
  Code's paths, hosts and token, and made `~/.claude` writable, with every project's transcripts. Now the root
  defines the agent kit once, and each run gets its own agent home, with state scoped per account, workspace and
  context.
- **Agents could not see a denial.** Most clients print only `CONNECT tunnel failed, response 403`. `pot` writes
  each denial to a file that the run can read, gives it to Claude through a hook, and shows it on the person's
  tmux pane. Writing it to the run's terminal corrupts a full-screen program.
- **Diagnosis needed a host shell.** `inspect` reads the machine except secrets and changes nothing, so an agent
  can diagnose a failed switch without unrestricted access.

Costs accepted:

- **Experimental, and Linux only from v2.15.0 to v2.17.1.** On macOS, `pot` refused with a message; v2.14.0 was
  the last release with a Seatbelt sandbox until the port (a Seatbelt backend, weaker: no process or network
  isolation, a dispatch clone at its own path). v2.15.0, a minor version, broke v2.14.0's sandbox, and its
  CHANGELOG says so.
- **Wrapped kit commands start a sandbox each,** and run unwrapped, with a stderr line, where there is no root's
  pot (no launcher, or a root older than v2.18.0). A deployment with no root takes the unwrapped tools on
  purpose. A wrapped start costs 0.3–0.5 s on platform-vm (8 ms unwrapped), and a sibling started from inside a
  run about 45 ms more (a nested child took about 0.2 s until v2.19.2).
- **Siblings, not nesting (v2.20.0).** A kit command with its own, narrower policy is started by the host beside the
  run that asked, under its own grant only: a sibling holds nothing of its caller's, and its caller nothing of the
  sibling's, on Linux and macOS alike. The price is that `narrower()` is now the only containment between them:
  every dimension nesting held for free is checked and tested on its own
  ([siblings, not nesting](reference/pot-siblings.md)). No run keeps user namespaces. A sibling is
  non-interactive, never outlives its request, and has no access to its caller's `/tmp`.
- **A person needs a root:** a home-manager module, pot's default root, or one that `pot init` writes.
- **Grants are readable.** Hidden means unmounted and unwritable. A grants file is readable through the root's git
  history wherever that `.git` is readable, `inspect` included. Grants hold no secrets.
- **`pot grant` is a speed bump.** It refuses agents and reads from `/dev/tty`, but an unrestricted host agent can
  edit the grants file or script a terminal. Plain `claude` stays unrestricted by choice.
- **In place, the repository's history is not protected.** A `work` run can commit, and can delete refs. Push what
  you need to keep. New git metadata, gitlinks and changed host-run files are reported after the run, not
  prevented. **`dispatch` removes this cost** for unattended work: the run works in a clone in pot's state, which
  carries only `user.*` and the origin URLs, so it never reaches the repository's history, hooks or config. Its
  commits reach the host only through a sandboxed `upload-pack` with fsck, which runs no hook, config or filter of
  the clone's, and only into `refs/remotes/pot-<id>/*` when the person runs `pot review ID --fetch`. A nested
  repository planted in the clone stays there.
- **What `dispatch` does not remove:** the fetched commits are still untrusted output. The host's look after the
  run flags host-run files, unsigned commits, new gitlinks, and repo-kit and root-repository changes for review; it
  does not prevent them. Read the branch as you would a stranger's pull request before you merge it.
- **A root repository changes only through a reviewed branch.** `work` refuses there. `dispatch` returns the change
  as a branch, and `pot review` flags changes to the grants file and to `programs.pot`. Review it before the
  switch.
- **Clones take disk space** until `pot review ID --discard`, or until `keepDays` or the size cap removes them. Those
  remove only clones that were fetched, discarded or have no commits: pot never deletes unfetched commits on its
  own, and names the runs still waiting for review when the cap is reached.
- **Resource limits cover only runs without a terminal,** and only where user systemd runs: a `systemd-run --user
  --scope` with a memory and a task limit. An interactive run has none.
- **The `nix` capability depends on experimental Nix features,** and shows the run the whole store.
- **Linux needs unprivileged user namespaces.** Docker's default seccomp profile blocks them.

Reopen when unprivileged user namespaces are no longer available on the machines that run pot, or when a kit's
real needs cannot be expressed as requests that a root grants.

Not yet: agents starting agents and siblings that outlive their request (with `pot serve`); brokers for `git.push` and proxy credentials; widening delegation through a supervisor; a VM
backend for code that no one has reviewed.
## Two repositories: tuteur produces kits, pot runs them

Since 0.1.0, pot is its own repository and flake, `github:danielnaab/pot`, with its own version line, starting
again below 1.0. graft became two: tuteur, which pins other repositories as git submodules at release tags and
wires what they ship into a flake, and pot. The rule: **pot depends on nothing of tuteur's; tuteur only produces
kits for pot**, as kit/1 values. pot's flake never names tuteur or graft.

From graft v2.16.0 the two shared one repository with one-way arrows: graft called one function of pot's,
`kitsFor`, and pot found a repository's kits from git facts alone. graft's decision named the condition for the
split: `kit/1` and `pot-kits/1`, the two seams, unchanged for two releases. graft v2.18.0 and v2.19.0 were those
releases ([pot's contracts](../contracts/README.md)), so the condition is met.

How the seams fall (Daniel, 2026-11-21):

1. **The consumer declares `inputs.pot`.** tuteur's mkFlake calls `inputs.pot.lib.pot.kitsFor` when a consumer's
   flake brings pot, and writes no `pot-kits` when it does not. tuteur never depends on pot, and a consumer that
   runs nothing under pot locks nothing of pot's. `lib.pot.kitsFor` keeps the path that
   [pot-lib/1](../contracts/pot-lib-1.md) promised, and the contract is open to any producer: a flake of one's own
   can call it.
2. **A consumer's `inputs.pot` does not follow the root's pot.** The producer's pot writes the bundle, the root's
   pot reads it, and [pot-kits/1](../contracts/pot-kits-1.md) is what lets the two be pinned apart.
3. **pot-kits/1 and runs/1 keep the field `scions`** (and runs/1 `scionCommits`) until each has another reason to
   reach /2: values, not fields, change within a version. pot's prose and messages say "submodule kit", because
   a git submodule at a release tag is pot's own git fact, not a tuteur concept.
4. **The `Pot-Run:` trailer is a contract,** [trailer/1](../contracts/trailer-1.md), because tuteur's `ci signers`
   reads it (an `agent@` signature needs one). `ci signers` stays in tuteur for now.
5. **knowledge-base-kit is a development input only** (`dev/flake.nix`), as it was in graft. pot's own flake then
   names no tool of its docs, and there is no cycle: tuteur's tests take pot, and pot's docs check takes only the
   kit.
6. **pot warns when a repository's producer gave it nothing.** Without `inputs.pot`, a tuteur consumer has no
   bundle, and so none of the protections that tuteur has pot apply (its provider files and its submodules
   folder). pot cannot tell why, but it can see the shape: a repository with submodules and a flake but no
   `pot-kits` gets one line at each start, `pot (work): DIR's flake has no pot-kits; its own kits and protections
   are not applied`, never silenced by `POT_QUIET`. It needs no tuteur convention to say so.

Also:

- **pot's grants and state do not move.** 0.1.0 reads and writes exactly what graft v2.21.0's pot does: grants/1
  (canonical 1, frozen), runs/1, and the layout of its state folder. The golden grant test's digests are
  byte-identical, so every existing grant still applies, and a root can go back to graft's pot. Both are release
  checks ([Development](development.md)).
- **Names that were graft's go,** with two kept on purpose for 0.1.x: pot still refuses `GRAFT_SANDBOX*` in a
  kit's `env.set` (graft v2.14.0's variables), until 0.2.0; and the refusal of a flake with graft v2.15.0's
  `graft-kits` still names graft, which wrote those bundles.
- **Costs:** a consumer that runs under pot adds one input and keeps it current (tuteur's `upgrade` moves an input
  named `pot`, patches only below 1.0). A root and a consumer can hold two pots, which only pot-kits/1 joins.

Reopen when a second runner of kits appears, or when a producer needs to read a pot format.

## A grant follows a pin move to a person-signed release

Since 0.6.0, a root may name **release signers** (`programs.pot.releaseSigners`, an `allowed_signers` file). A
kit's powerful grant then holds, without asking, when its source changed only by moving pins to release tags
that one of those signers signed: a submodule kit's gitlink, or a repo kit's `flake.lock` inputs, `flake.nix`'s tag
literals and gitlinks ([Release moves](reference/pot.md#release-moves)). Its requests are still compared, so a kit
that asks for more asks.

Why:

- **The grant asked again what the tag had answered.** A grant asks "do I trust this code with these powers". When
  the code is a release of the person's own provider and the person signed that tag, the re-grant adds no
  information: its diffstat showed a `flake.lock`, nothing a release review had not seen. Each routine upgrade
  needed a `pot grant` at a terminal, which kept agents from landing upgrades alone.
- **Principle 3 holds.** Who signs is a host fact: the root names the signers, never a repository, and the tag
  objects are fetched from each input's URL into pot's state and verified there with `ssh-keygen -Y verify -n git`.
- **The grants file stays a person's.** The acceptance is computed at each start, recorded in the run's `start`
  event and shown by `pot explain`; nothing is written to the grants file, which still holds only what a person
  granted (and signed, with the root's commit).

Costs accepted:

- **Every moved node must be a signed release, or what one locks.** A transitive input is accepted only when it is
  locked exactly as a moved release's own `flake.lock` locks it. A `nix flake update` that also moves nixpkgs, a
  branch or commit pin, or an added input asks, as before.
- **`flake.nix` may change in its release tag literals.** A literal is also a value its code can read; the code
  around it is the granted code, so a literal swap cannot add code, but it can choose among what the granted code
  already does.
- **Only the moves since the grant are judged.** A tree whose other changes were accepted by the narrowed
  comparison (the kit as built unchanged) and then moves a pin asks once, since the two rules do not combine.
- **A signer's key is the whole trust.** A leaked or retired key is handled by editing `releaseSigners`; the signers
  file is read at each check, so cached tags stop verifying too. An agent-signed tag is not a release signer's, and
  falls back to `pot grant`, as it should.
- **The first start after an upgrade fetches,** once per tag; offline it asks. The fetch is the host's git, with the
  host's credentials, as `ls-remote` already was for submodule kits.

Reopen when a release flow signs tags with a key that is not a person's (a lander's `release@` key under a signed
plan), or when grants gain delegation.

## Sources

- [pot](reference/pot.md), [siblings, not nesting](reference/pot-siblings.md)
- [pot's contracts](../contracts/README.md)
- [core/pot.py](../core/pot.py): finding the flake, the bundle and the missing-bundle warning, grants, runs;
  `release_moves`, `scion_accepts`
- [core/releases.py](../core/releases.py): fetching and verifying release tags
- [lib/kits-for.nix](../lib/kits-for.nix): `kitsFor`, which any producer calls
- graft's [decisions](https://github.com/danielnaab/graft/blob/v2/docs/decisions.md): "Enforce the boundary in a
  sandbox, not in rules", "Roots grant reach; kits only ask" and "One repository, one-way arrows between graft and
  pot", which the sections above replace
- The split's plan and Daniel's decisions (2026-11-21), in the flexion-assistant notebook: "pot: what moves to its
  own repository" and "the consumer migration"
