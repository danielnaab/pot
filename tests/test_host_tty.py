"""A command in the terminal's foreground (fg.py), for pot host's CMD and bubblewrap's run: at a terminal, the
command is its own process group, in the terminal's foreground, so a terminal multiplexer names the window after it
(tmux reads the foreground group's leader), not after pot's python3; the terminal comes back to pot's group when it
ends. Off a terminal, it runs in pot's group, as before. bubblewrap's argv[0] is the command's name, as bwrap stays
the group leader. Run with the other unit tests (pot-unit); on any system with a pty."""
import os
import pty
import select
import subprocess
import sys
import unittest

import bwrap
import fg
import pot

PROBE = "import os; print('fg' if os.tcgetpgrp(0) == os.getpgrp() else 'bg', os.getpgrp() == os.getpid())"


def in_pty(fn):
    """Run fn() in a child whose controlling terminal is a new pty (pty.fork: a new session, so the child's group is
    the terminal's foreground); return what it wrote."""
    try:
        pid, fd = pty.fork()
    except OSError as e:          # a builder without /dev/ptmx
        raise unittest.SkipTest(f"no pty: {e}")
    if pid == 0:
        try:
            fn()
        finally:
            sys.stdout.flush()
            os._exit(0)
    out = b""
    while True:
        r, _, _ = select.select([fd], [], [], 20)
        if not r:
            break
        try:
            chunk = os.read(fd, 4096)
        except OSError:            # EIO: the child closed the pty
            break
        if not chunk:
            break
        out += chunk
    os.waitpid(pid, 0)
    os.close(fd)
    return out.decode().replace("\r", "")


class Foreground(unittest.TestCase):
    def test_cmd_leads_the_foreground_group(self):
        def body():
            status = pot.host_wait([sys.executable, "-I", "-c", PROBE], dict(os.environ))
            back = os.tcgetpgrp(0) == os.getpgrp()
            print(f"status={status} back={back}")
        out = in_pty(body)
        self.assertIn("fg True", out)
        self.assertIn("status=0 back=True", out)

    def test_exit_status_passes_through(self):
        out = in_pty(lambda: print(f"status={pot.host_wait([sys.executable, '-c', 'raise SystemExit(3)'], dict(os.environ))}"))
        self.assertIn("status=3", out)

    def test_off_a_terminal_cmd_stays_in_pots_group(self):
        r = subprocess.run([sys.executable, "-I", "-c",
                            f"import sys; sys.path[:0] = {sys.path!r}; import os, pot;"
                            f"pot.host_wait([sys.executable, '-c', 'import os; print(os.getpgrp() == {os.getpgrp()})'], dict(os.environ))"],
                           stdin=subprocess.DEVNULL, capture_output=True, text=True, start_new_session=False)
        self.assertEqual(r.stdout.strip(), "True", r.stderr)


class Shared(unittest.TestCase):
    def test_wait_hands_the_terminal_over_and_back(self):
        def body():
            f = fg.in_foreground()
            p = subprocess.Popen([sys.executable, "-I", "-c", PROBE], process_group=fg.popen_group(f))
            status = fg.wait(p, f)
            print(f"was={f} status={status} back={os.tcgetpgrp(0) == os.getpgrp()}")
        out = in_pty(body)
        self.assertIn("fg True", out)
        self.assertIn("was=True status=0 back=True", out)

    def test_off_a_terminal(self):
        self.assertIsNone(fg.popen_group(False))
        p = subprocess.Popen([sys.executable, "-c", "raise SystemExit(4)"])
        self.assertEqual(fg.wait(p, False), 4)


class WindowName(unittest.TestCase):
    def test_the_commands_name(self):
        self.assertEqual(bwrap.window_name({"argv": ["/nix/store/x-claude-code/bin/claude", "--resume"]}), "claude")

    def test_no_command(self):
        self.assertEqual(bwrap.window_name({"argv": []}), "bwrap")
        self.assertEqual(bwrap.window_name({}), "bwrap")


if __name__ == "__main__":
    unittest.main()
