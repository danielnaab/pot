---
status: working
---
# trailer/1: the Pot-Run: trailer

A commit message's `Pot-Run: <id>` trailer: the run that made the commit, as the run claims it. pot writes it; a
program that reads git history (tuteur's `ci signers`, later grove) reads it without asking pot.

| | |
|---|---|
| Version | 1: the key `Pot-Run` and the form of its value |
| Producer | `pot trailer`, git's `prepare-commit-msg` hook, which a run's generated `~/.gitconfig` wires (and a person may wire on the host) |
| Reader | tuteur's `ci signers` (a commit signed by an `agent@` key needs one); later grove; any program that reads commits |
| Owner | pot |
| Schema | [trailer-1.schema.json](trailer-1.schema.json): one value, as git's trailer parser gives it (JSON Schema 2020-12) |
| Fixtures | [fixtures/trailer-1/](fixtures/trailer-1/) |

## Shape

```
Fix the parser

The tokenizer kept a trailing space.

Pot-Run: 7f3a
```

- **The key** is `Pot-Run`, in the message's trailer block, as git defines one: the last paragraph, of `Key: value`
  lines. Read it with git's own parser, never with a search of the message:
  `git log --format='%(trailers:key=Pot-Run,valueonly)'`, or `git interpret-trailers --parse`. A `Pot-Run:` line
  anywhere else (the subject, the body) is no trailer.
- **The value** is the run's id: four lowercase hex digits, the `id` of [runs/1](runs-1.md). With `POT_RUN` set to
  anything else, `pot trailer` writes nothing.
- **Once.** `pot trailer` adds the trailer only when the message has none (`git interpret-trailers --if-exists
  doNothing`), so an amend keeps the run that made the commit first. A message that another tool edited may hold
  more than one; a reader takes the first.

## When pot writes it

`pot trailer MSGFILE [SOURCE [SHA]]`, with `POT_RUN` set (every run, and `pot host`, set it):

| `SOURCE` (git's) | Trailer |
|---|---|
| `message` (`-m`, `-F`), `commit` (`-c`, `-C`, `--amend`), `merge`, `squash` | added, unless the message already has one |
| none (an editor's message), `template` | none: the person writes it, and an emptied message must still abort the commit |
| any, with an empty message | none |

It never fails a commit: every error exits 0. A run's `~/.gitconfig` wires it as a config-based hook
(`hook.pot-trailer`), which git 2.54 or later runs; an older git in the run ignores it, and the commit has no
trailer.

## What it means, and what it does not

- **A claim, not evidence.** The command in the run sets its own environment and its own messages, so it can
  write any `Pot-Run:` it likes, or none. What ties a commit to a run is the run's `after` event, which the host
  writes ([runs/1](runs-1.md): a clone run's `clone`, a `pot host` run's `commits`). A reader that needs proof
  looks the id up with `pot runs ID --json` on the machine that ran it.
- **Not unique across machines.** An id is unique in one pot state folder, not between machines.
- **Absent is not "no run".** An editor's commit, a commit under an older git, and a commit made outside pot have
  none.

The fixtures: `message.txt` and `with-sign-off.txt` (beside another trailer) hold one; `none.txt` and
`in-subject.txt` (a `Pot-Run:` in the subject, which git never reads as a trailer) hold none. `values.json` maps each
fixture to the values git's parser gives. pot's tests hold `pot trailer` to them: from each fixture's message
without its trailer it writes the fixture back, and every value validates against the schema
(`tests/run.sh contracts`).

## Compatibility

- **Within version 1** the key and the value's form stay. A longer id, a second key, or a value with more in it is
  trailer/2, written beside version 1 for a release before version 1 goes.
- A reader that sees a value it does not know (not four hex digits) treats the commit as having no trailer.

## Sources

- [core/pot.py](../core/pot.py): `trailer`, and the run's `~/.gitconfig` that wires it
- [tests/test_v2210.py](../tests/test_v2210.py): when the trailer is added and when it is not
- [tests/run.sh](../tests/run.sh): the `contracts` section
- [pot's reference: trailer](../docs/reference/pot.md#trailer)
