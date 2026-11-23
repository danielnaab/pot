"""C1 (decided 2026-11-23): pot claims resource-limits only where the run's backend upholds it, and says so plainly
where it doesn't. A backend upholds it when its probe lists resource-limits (what it starts stays in its caller's
cgroup: bubblewrap; the container backend under podman, never under docker); the run gets it when it is also in the
resource scope. Run with the other unit tests (pot-unit)."""
import unittest

import bwrap as B
import compose as C
from test_compose import inp

SCOPE = {"argv": ["systemd-run", "--user", "--scope", "--"], "env": {}, "limits": "MemoryMax=50% TasksMax=2048"}
PROBED = ["closure-store", "own-netns", "own-pidns", "no-tiocsti", "in-place", "root-files"]
DOCKER_WHY = "the container backend can't keep its run in pot's scope here"


class Resources(unittest.TestCase):
    def test_upheld_in_the_scope(self):
        g, res = C.run_resources(PROBED + ["resource-limits"], SCOPE, "bubblewrap")
        self.assertIn("resource-limits", g)
        self.assertEqual(res, {"limits": "MemoryMax=50% TasksMax=2048"})

    def test_not_upheld_in_the_scope(self):
        # docker: the run is in pot's scope, its container is not; never claimed, and said
        g, res = C.run_resources(PROBED, SCOPE, "container")
        self.assertNotIn("resource-limits", g)
        self.assertEqual(res, {"limits": None, "why": DOCKER_WHY})

    def test_at_a_terminal(self):
        g, res = C.run_resources(PROBED + ["resource-limits"], {}, "bubblewrap")
        self.assertNotIn("resource-limits", g)
        self.assertEqual(res, {"limits": None, "why": "at a terminal"})
        # a backend that can't uphold them says so at a terminal too: its runs have none either way
        self.assertEqual(C.run_resources(PROBED, {}, "container")[1], {"limits": None, "why": DOCKER_WHY})

    def test_not_a_run(self):
        # pot explain composes no run: a backend that upholds them has no word; one that can't still says so
        self.assertEqual(C.run_resources(PROBED + ["resource-limits"], None, "bubblewrap"), (PROBED, None))
        self.assertEqual(C.run_resources(PROBED, None, "container")[1], {"limits": None, "why": DOCKER_WHY})

    def test_no_user_systemd(self):
        g, res = C.run_resources(PROBED + ["resource-limits"], {"note": "no resource limits (no user systemd: pot doctor)",
                                                               "why": "no user systemd"}, "bubblewrap")
        self.assertNotIn("resource-limits", g)
        self.assertEqual(res, {"limits": None, "why": "no user systemd"})

    def test_the_probe_order_is_kept(self):
        g, _ = C.run_resources(["closure-store", "resource-limits", "own-netns"], SCOPE, "bubblewrap")
        self.assertEqual(g, ["closure-store", "own-netns", "resource-limits"])

    def test_bwrap_probe_lists_it(self):
        # bubblewrap's sandbox is pot's child, in its scope
        self.assertIn("resource-limits", B.GUARANTEES)


class InThePolicy(unittest.TestCase):
    def test_policy_and_explain(self):
        i = inp(probe=PROBED)
        i["backendName"] = "container"
        i["resources"] = {"limits": None, "why": DOCKER_WHY}
        policy, report = C.compose(i)
        self.assertEqual(policy["backend"]["resources"], {"limits": None, "why": DOCKER_WHY})
        self.assertNotIn("resource-limits", policy["backend"]["proved"])
        text = C.explain_text(policy, report, i["root"])
        self.assertIn(f"resources  none ({DOCKER_WHY})", text)

    def test_policy_with_limits(self):
        i = inp(probe=PROBED + ["resource-limits"])
        i["resources"] = {"limits": "MemoryMax=50% TasksMax=2048"}
        policy, report = C.compose(i)
        self.assertEqual(policy["backend"]["resources"], {"limits": "MemoryMax=50% TasksMax=2048"})

    def test_requires_names_why(self):
        i = inp(probe=PROBED)
        i["backendName"] = "container"
        i["resources"] = {"limits": None, "why": DOCKER_WHY}
        i["root"]["contexts"] = {"work": {"requires": ["resource-limits"]}}
        with self.assertRaises(C.Refusal) as e:
            C.compose(i)
        self.assertIn(f"(resource-limits: {DOCKER_WHY})", e.exception.message)


if __name__ == "__main__":
    unittest.main()
