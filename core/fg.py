"""A command in the terminal's foreground: pot host's CMD, and bubblewrap's run.

When pot is the terminal's foreground, the command is started as its own process group (`process_group=0`) and
given the terminal while it runs, so a terminal multiplexer names the window after it (tmux reads the foreground
group leader's name, which was pot's python3), ^C and ^Z reach it alone, and the terminal comes back to pot's
group when it ends. Off a terminal (a script, a sibling, --detach), the command runs in pot's group as before.
Seatbelt's run does the same with its own signal forwarding (seatbelt.wait_job)."""
import os
import signal


def in_foreground():
    """Whether this process's group is the foreground of the terminal on stdin."""
    try:
        return os.isatty(0) and os.tcgetpgrp(0) == os.getpgrp()
    except OSError:
        return False


def popen_group(fg):
    """Popen's process_group for a command started under FG (in_foreground())."""
    return 0 if fg else None


def wait(proc, fg):
    """Wait for PROC, its exit code (negative: the signal that ended it). Under FG, PROC was started with
    popen_group(fg): the terminal is its while it runs and this group's again after; when it stops (^Z) this group
    stops too, so the shell sees the job stopped, and when the shell resumes it, PROC resumes."""
    if not fg:
        return proc.wait()
    ttou = signal.signal(signal.SIGTTOU, signal.SIG_IGN)   # tcsetpgrp from the background, at the end

    def hand(pgid):
        try:
            os.tcsetpgrp(0, pgid)
        except OSError:          # PROC already gone: its group with it
            pass

    def resume():
        try:
            os.killpg(proc.pid, signal.SIGCONT)
        except ProcessLookupError:
            pass
    try:
        hand(proc.pid)
        resume()                 # in case it read the terminal before it was in the foreground
        while True:
            _, st = os.waitpid(proc.pid, os.WUNTRACED)
            if not os.WIFSTOPPED(st):
                proc.returncode = os.waitstatus_to_exitcode(st)
                return proc.returncode
            hand(os.getpgrp())
            os.killpg(os.getpgrp(), signal.SIGSTOP)
            hand(proc.pid)
            resume()
    finally:
        hand(os.getpgrp())
        signal.signal(signal.SIGTTOU, ttou)
