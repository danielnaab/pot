"""pot: the host's look at a clone run, for the container backend (pot phase 2).

    python3 -I -S clonelook.py GIT RUNDIR SANDBOX...

bwrap.py's own `clone_after`, run for each clone RUNDIR/before.json lists (`pins.py before` wrote it; none for a sibling
in its caller's clone): the clone's branches staged into RUNDIR/staging through an upload-pack that runs sandboxed, with
fsck, and the after event's clone fields read from staging, never from the clone. Only the sandbox differs: SANDBOX is
the argv that runs `git ARGS` in a container with the clone read-only at /clone (`pot-container --config CONFIG git`,
then RUNDIR CLONE CWD -- ARGS), where bubblewrap runs bwrap and Seatbelt sandbox-exec. Host GIT runs only in the
staging repositories pot creates. Prints those fields as JSON; a clone whose look failed is `fetch_failed` (K1), never
"no commits". Reads RUNDIR's files never through a link. The standard library only.
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bwrap as B  # noqa: E402
import pins as P  # noqa: E402


def main(argv):
    if len(argv) < 3:
        print("usage: clonelook.py GIT RUNDIR SANDBOX...", file=sys.stderr)
        return 2
    git, rundir, sandbox = argv[0], argv[1], argv[2:]

    def sandboxed_git(cfg, rd, clone, args, cwd="/clone", timeout=600):
        return list(sandbox) + [rd, clone, cwd, "--"] + list(args)
    B.sandboxed_git = sandboxed_git
    try:
        policy, launch = P.read_own(rundir, "policy.json"), P.read_own(rundir, "launch.json")
        if policy is None or launch is None:
            raise OSError("policy.json or launch.json is missing")
        before = P.read_own(rundir, "before.json") or {}
    except (OSError, ValueError) as e:
        print(P.visible(f"clonelook.py: {e}"), file=sys.stderr)
        return 1
    ev = {"commits": [], "unsigned": 0, "touches_host_run": [], "repo_kit_change": [], "root_repo_change": False,
          "new_git_metadata": [], "agent_config": []}
    for b in before.get("clones") or []:
        try:
            B.clone_after({"git": git}, rundir, policy, launch, b, ev)
        except Exception as e:      # K1 (bwrap's after): never "no commits" for a look that failed
            B.FETCH_FAILED.append(P.visible(f"the host's look failed ({type(e).__name__}: {e})")[:300])
            print(P.visible(f"pot: the host's look at the clone failed: {type(e).__name__}: {e}"), file=sys.stderr)
            ev["fetch_failed"] = "; ".join(B.FETCH_FAILED)
    print(json.dumps(ev))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
