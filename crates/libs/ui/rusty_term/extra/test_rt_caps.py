"""Interruption tests for rt-caps.sh, run under a pseudo-terminal.

Nothing answers the script's queries, so each one waits out its read timeout
in raw mode: a signal sent then lands while the tty is raw. The tests check
that the EXIT trap restores the tty and unwinds the modes the section owned.

    python3 -m unittest discover -s extra -p 'test_*.py' -v
"""

from __future__ import annotations

import fcntl
import os
import pty
import select
import signal
import subprocess
import sys
import termios
import time
import unittest
from dataclasses import dataclass
from pathlib import Path

SCRIPT = Path(__file__).with_name("rt-caps.sh")
TIMEOUT_S = 30.0


@dataclass
class Run:
    """A running rt-caps.sh with the master side of its pty."""

    proc: subprocess.Popen[bytes]
    master: int
    slave: int
    out: bytearray

    def read_until(self, needle: bytes) -> None:
        """Collect output until `needle` appears; fail after TIMEOUT_S."""
        deadline = time.monotonic() + TIMEOUT_S
        while needle not in self.out:
            if time.monotonic() > deadline:
                raise AssertionError(f"never saw {needle!r}; got {bytes(self.out)!r}")
            self._pump(0.1)

    def interrupt(self) -> int:
        """Send SIGINT to the script's process group, as Ctrl+C would; return its exit code."""
        os.killpg(self.proc.pid, signal.SIGINT)
        return self.finish()

    def finish(self) -> int:
        """Drain output until the script exits and return its exit code."""
        deadline = time.monotonic() + TIMEOUT_S
        while self.proc.poll() is None:
            if time.monotonic() > deadline:
                self.proc.kill()
                raise AssertionError("rt-caps.sh did not exit")
            self._pump(0.1)
        self._pump(0.2)
        return self.proc.returncode

    def lflag(self) -> int:
        """The tty's local mode flags as the script left them."""
        return termios.tcgetattr(self.slave)[3]

    def _pump(self, wait_s: float) -> None:
        ready, _, _ = select.select([self.master], [], [], wait_s)
        if not ready:
            return
        try:
            self.out += os.read(self.master, 65536)
        except OSError:  # EIO once the slave side has closed
            pass


def start(*sections: str, **env: str) -> Run:
    """Start rt-caps.sh on a fresh pty that is its controlling terminal."""
    master, slave = pty.openpty()

    def own_tty() -> None:
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    proc = subprocess.Popen(
        ["sh", str(SCRIPT), *sections],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        preexec_fn=own_tty,
        env={**os.environ, "RT_NOPAUSE": "1", "TERM": "xterm-256color", **env},
    )
    return Run(proc, master, slave, bytearray())


@unittest.skipUnless(sys.platform.startswith("linux"), "needs a Linux pty")
class InterruptTest(unittest.TestCase):
    def tearDown(self) -> None:
        for fd in (self.run_.master, self.run_.slave):
            os.close(fd)

    def test_interrupt_in_a_query_restores_the_tty(self) -> None:
        self.run_ = start("widths")
        self.run_.read_until(b"cell widths")
        time.sleep(0.1)  # inside the first raw-mode query
        self.assertEqual(self.run_.interrupt(), 130)
        flags = self.run_.lflag()
        self.assertTrue(flags & termios.ECHO, "echo left off")
        self.assertTrue(flags & termios.ICANON, "canonical mode left off")

    def test_interrupt_unwinds_mode_2027(self) -> None:
        self.run_ = start("graphemes")
        self.run_.read_until(b"\x1b[?2027h")
        mark = len(self.run_.out)
        self.run_.interrupt()
        self.assertIn(b"\x1b[?2027l", self.run_.out[mark:])

    def test_interrupt_pops_kitty_keyboard_flags(self) -> None:
        self.run_ = start("input")
        for prompt in (b"Click, drag", b"Paste something", b"Switch focus"):
            self.run_.read_until(prompt)
            os.write(self.run_.master, b"q")
        self.run_.read_until(b"kitty keyboard gives")
        mark = len(self.run_.out)
        self.run_.interrupt()
        self.assertIn(b"\x1b[<u", self.run_.out[mark:])

    def test_clipboard_is_left_alone_without_opt_in(self) -> None:
        self.run_ = start("osc")
        self.assertEqual(self.run_.finish(), 0)
        self.assertNotIn(b"\x1b]52;", self.run_.out)

    def test_clipboard_demo_runs_with_opt_in(self) -> None:
        self.run_ = start("osc", RT_CLIPBOARD="1")
        self.assertEqual(self.run_.finish(), 0)
        self.assertIn(b"\x1b]52;c;", self.run_.out)


if __name__ == "__main__":
    unittest.main()
