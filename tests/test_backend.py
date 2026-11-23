"""pot phase 2: pot chooses the run's backend from the root (select_backend): --backend NAME, else the context's
backend, else the root's defaultBackend; a sibling runs under its caller's. A backend pot has not, or one the root
does not offer, is refused (exit 2); a container backend whose probe proves nothing (its engine missing) is refused
naming the engine's error and the way out; pot explain says why. Run with the other unit tests (pot-unit)."""
import os
import stat
import tempfile
import types
import unittest

import compose as C
import pot

ROOT = {"backends": {"bubblewrap": {"kind": "bwrap", "program": "/b/bwrap"},
                     "container": {"kind": "container", "program": "/b/container", "engine": "podman"}},
        "defaultBackend": "container", "contexts": {"onbw": {"extends": "work", "backend": "bubblewrap"}}}


def host(root=ROOT):
    return types.SimpleNamespace(root=root)


def ctx(name="work"):
    return C.resolve_context(ROOT, name)


class Select(unittest.TestCase):
    def test_the_roots_default(self):
        b = pot.select_backend(host(), {}, ctx())
        self.assertEqual((b["name"], b["kind"], b["engine"], b["why"]), ("container", "container", "podman", "the root's default"))

    def test_auto_is_the_roots_default(self):
        self.assertEqual(pot.select_backend(host(), {"backend": "auto"}, ctx())["name"], "container")

    def test_the_contexts_backend_over_the_roots_default(self):
        b = pot.select_backend(host(), {}, ctx("onbw"))
        self.assertEqual((b["name"], b["why"]), ("bubblewrap", 'context onbw (backend = "bubblewrap")'))

    def test_the_flag_over_the_contexts(self):
        b = pot.select_backend(host(), {"backend": "container"}, ctx("onbw"))
        self.assertEqual((b["name"], b["why"]), ("container", "--backend container"))

    def test_a_sibling_says_its_callers(self):
        b = pot.select_backend(host(), {"backend": "bubblewrap", "siblingOf": {}}, ctx())
        self.assertEqual((b["name"], b["why"]), ("bubblewrap", "its caller's backend (a sibling runs under it)"))

    def test_an_unknown_backend(self):
        with self.assertRaises(C.Refusal) as e:
            pot.select_backend(host(), {"backend": "lxc"}, ctx())
        self.assertEqual(e.exception.message, "pot (work): --backend lxc: pot has no backend lxc (its backends: bubblewrap, container, seatbelt)")
        self.assertEqual(e.exception.status, 2)

    def test_a_backend_not_offered(self):
        root = {"backends": {"bubblewrap": ROOT["backends"]["bubblewrap"]}, "defaultBackend": "bubblewrap"}
        with self.assertRaises(C.Refusal) as e:
            pot.select_backend(host(root), {"backend": "container"}, ctx())
        self.assertEqual(e.exception.message, "pot (work): --backend container: this root does not offer the container backend "
                         "(it offers bubblewrap); a root offers it with pot.lib.mkRoot's container = { engine = \"podman\"; } "
                         "(programs.pot.container)")
        self.assertEqual(e.exception.status, 2)

    def test_a_roots_default_not_offered(self):
        root = {"backends": {"bubblewrap": ROOT["backends"]["bubblewrap"]}, "defaultBackend": "seatbelt"}
        with self.assertRaises(C.Refusal) as e:
            pot.select_backend(host(root), {}, ctx())
        self.assertIn("the root's defaultBackend = \"seatbelt\": this root does not offer the seatbelt backend", e.exception.message)


class Probe(unittest.TestCase):
    def test_a_container_engine_missing(self):
        with tempfile.TemporaryDirectory() as d:
            prog = os.path.join(d, "backend")
            with open(prog, "w") as f:
                f.write("#!/bin/sh\necho '{\"error\":\"podman: can'\"'\"'t run /x/podman: not found\",\"guarantees\":[]}'\n")
            os.chmod(prog, stat.S_IRWXU)
            b = pot.select_backend(host(), {}, ctx())
            with self.assertRaises(C.Refusal) as e:
                pot.probe_now(prog, d, b, host())
        self.assertEqual(e.exception.message,
                         "pot (work): the container backend (podman, the root's default) proves nothing here: podman: can't run "
                         "/x/podman: not found. No run starts under it, and pot never runs one unsandboxed: --backend bubblewrap "
                         "chooses another of this root's backends, pot host CMD runs unsandboxed; pot doctor shows each backend.")


    def test_a_container_probe_is_never_cached(self):
        # container.md: the probe's answer depends on the caller and the engine's state (and, with no program named,
        # on PATH), so pot probes it for every run: a working engine once never vouches for a missing one later
        with tempfile.TemporaryDirectory() as d:
            prog = os.path.join(d, "backend")

            def says(out):
                with open(prog, "w") as f:
                    f.write("#!/bin/sh\necho '" + out + "'\n")
                os.chmod(prog, stat.S_IRWXU)
            h = types.SimpleNamespace(root=ROOT, state=d)
            b = pot.select_backend(h, {}, ctx())
            says('{"guarantees":["own-netns"]}')
            self.assertEqual(pot.probe(prog, d, h, b), ["own-netns"])
            says('{"error":"podman: gone","guarantees":[]}')
            with self.assertRaises(C.Refusal) as e:
                pot.probe(prog, d, h, b)
            self.assertIn("proves nothing here: podman: gone", e.exception.message)

    def test_bubblewraps_probe_is_cached_per_build(self):
        with tempfile.TemporaryDirectory() as d:
            prog = os.path.join(d, "backend")
            with open(prog, "w") as f:
                f.write("#!/bin/sh\necho '{\"guarantees\":[\"own-netns\"]}'\n")
            os.chmod(prog, stat.S_IRWXU)
            h = types.SimpleNamespace(root=ROOT, state=d)
            b = pot.select_backend(h, {"backend": "bubblewrap"}, ctx())
            self.assertEqual(pot.probe(prog, d, h, b), ["own-netns"])
            os.remove(prog)
            self.assertEqual(pot.probe(prog, d, h, b), ["own-netns"])

class Validate(unittest.TestCase):
    def test_a_contexts_backend_the_root_does_not_offer(self):
        root = {"backends": {"bubblewrap": {}}, "contexts": {"ci": {"extends": "work", "backend": "container"}}}
        with self.assertRaises(C.KitError) as e:
            C.validate_root(root)
        self.assertIn('the root\'s contexts: ci: backend = "container": the root offers bubblewrap', e.exception.message)


if __name__ == "__main__":
    unittest.main()
