#!/usr/bin/env python3
"""Record the fidelity corpus: run each recipe in an 80x24 PTY, send its
scripted keystrokes, and save every byte the program wrote as `<name>.vt`.

The recording stops when the recipe's script ends (the program is killed),
so the final screen is whatever the program showed at that moment. Recordings
depend on the host (program versions, terminfo), which is why the `.vt` files
are committed: re-record only to add or deliberately refresh a case, then
regenerate the goldens (`UPDATE_GOLDEN=1 cargo test -p rusty_term_core --test
fidelity`).

Usage: python3 record.py [NAME ...]    # default: every recipe
Stdlib only.
"""

import fcntl
import os
import pty
import select
import signal
import struct
import sys
import tempfile
import termios
import time
from pathlib import Path

COLS, ROWS = 80, 24
HERE = Path(__file__).resolve().parent
CORPUS = HERE / "corpus"
ENV = {
    "TERM": "xterm-256color",
    "LANG": "C.UTF-8",
    "LC_ALL": "C.UTF-8",
    "HOME": tempfile.gettempdir(),
    "PATH": "/usr/bin:/bin",
    "COLUMNS": str(COLS),
    "LINES": str(ROWS),
}

ENTER = b"\r"
ESC = b"\x1b"

# Synthetic edge cases, written with printf so they need no program.
SAMPLE = HERE / "sample.txt"
PRINTF = {
    "wide_combining": r"wide: 日本語テキスト 한국어 漢字\n"
    r"emoji: 😀 👍🏽 👨‍👩‍👧 🇯🇵 ❤️\n"
    r"combining: e\xcc\x81 a\xcc\x88 n\xcc\x83 o\xcc\x82\xcc\x83\n"
    r"rtl: שלום עולם مرحبا بالعالم\n"
    r"edge: 0123456789012345678901234567890123456789012345678901234567890123456789012345678日本\n",
    "sgr_matrix": r"\e[1mbold\e[0m \e[2mdim\e[0m \e[3mitalic\e[0m \e[4munder\e[0m "
    r"\e[4:3mcurly\e[0m \e[5mblink\e[0m \e[7mreverse\e[0m \e[8mhidden\e[0m \e[9mstrike\e[0m\n"
    r"\e[31mred\e[32mgreen\e[33myellow\e[34mblue\e[35mmagenta\e[36mcyan\e[37mwhite\e[0m\n"
    r"\e[91mbred\e[92mbgreen\e[94mbblue\e[0m \e[41m bg \e[42m bg \e[0m \e[101m bbg \e[0m\n"
    r"\e[38;5;196m196\e[38;5;46m46\e[38;5;21m21\e[38;5;244m244\e[48;5;226m bg226 \e[0m\n"
    r"\e[38;2;255;128;0mtruecolor\e[48;2;0;64;128m bg \e[0m \e[58;5;1m\e[4mulcolor\e[0m\n",
    "scroll_regions": r"\e[2J\e[H" + "".join(rf"line {i:02}\n" for i in range(1, 23))
    + r"\e[5;15r\e[15;1H" + r"\nscrolled A\nscrolled B\nscrolled C"
    + r"\e[5;1H\eMreverse-indexed\e[r\e[20;1H\e[2Linserted-after-IL\e[24;1H",
    "wrap_tabs": r"\e[2J\e[H" + "x" * 78 + r"abcdef\n" + r"a\tb\tc\td\te\n"
    + r"\e[?7l" + "y" * 85 + r"\e[?7h\n" + r"\e[3;10H\e[1K\e[4;1Hclear-to-eol\e[5G\e[K"
    + r"\e[6;1Hinsert\e[6;3H\e[2@\e[7;1Hdelete-me\e[7;1H\e[3P\e[8;1Herase\e[8;2H\e[2X",
    "alt_screen": r"primary 1\nprimary 2\n\e[?1049h\e[2J\e[Halt screen text\e[5;5Hmid"
    r"\e[?1049l\nback on primary\n",
}


def spawn(argv):
    """Fork `argv` on a fresh 80x24 PTY; return (pid, master fd)."""
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(HERE)
        os.execvpe(argv[0], argv, ENV)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    return pid, fd


# Primary Device Attributes queries (`CSI c`, `CSI 0 c`) block vttest until
# answered; reply as a VT100 with the advanced video option, as xterm does.
DA1_QUERIES = (b"\x1b[c", b"\x1b[0c")
DA1_REPLY = b"\x1b[?1;2c"


def drain(fd, out, seconds):
    """Read everything the child writes for `seconds`; answer DA1 queries."""
    end = time.monotonic() + seconds
    while (left := end - time.monotonic()) > 0:
        ready, _, _ = select.select([fd], [], [], left)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:  # child exited: EIO on Linux
            return False
        if not chunk:
            return False
        out.extend(chunk)
        for query in DA1_QUERIES:
            for _ in range(chunk.count(query)):
                os.write(fd, DA1_REPLY)
    return True


def record(name, argv, steps, settle=1.0):
    """Run one recipe. `steps` is a list of (seconds to wait, bytes to send)."""
    pid, fd = spawn(argv)
    out = bytearray()
    alive = drain(fd, out, settle)
    for wait, keys in steps:
        if not alive:
            break
        if keys:
            os.write(fd, keys)
        alive = drain(fd, out, wait)
    if alive:
        os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    os.close(fd)
    (CORPUS / f"{name}.vt").write_bytes(out)
    print(f"{name:24} {len(out):>8} bytes")


def vttest(name, *menu):
    """Drive vttest's menus: each entry is typed then Enter; end on that screen."""
    record(name, ["vttest"], [(0.6, m.encode() + ENTER) for m in menu])
    (HERE / "vttest.log").unlink(missing_ok=True)


def recipes():
    yield "wide_combining", lambda: record("wide_combining", ["printf", PRINTF["wide_combining"]], [], 0.5)
    for key in ("sgr_matrix", "scroll_regions", "wrap_tabs", "alt_screen"):
        yield key, (lambda k=key: record(k, ["printf", PRINTF[k]], [], 0.5))
    # vttest: menu 1 (cursor movement) screens 1-3, menu 2 (screen features)
    # screens 1-4, menu 8 (VT102 insert/delete) screens 1-2.
    for n in range(1, 4):
        yield f"vttest_1_{n}", (lambda n=n: vttest(f"vttest_1_{n}", "1", *[""] * (n - 1)))
    for n in range(1, 5):
        yield f"vttest_2_{n}", (lambda n=n: vttest(f"vttest_2_{n}", "2", *[""] * (n - 1)))
    for n in range(1, 3):
        yield f"vttest_8_{n}", (lambda n=n: vttest(f"vttest_8_{n}", "8", *[""] * (n - 1)))
    yield "vim_edit", lambda: record(
        "vim_edit",
        ["vim", "-u", "NONE", "-i", "NONE", "-n", "-N", "-c", "syntax on", "-c", "set number", SAMPLE.name],
        [(0.5, b"10G"), (0.5, b"o"), (0.3, "inserted 日本 line".encode()), (0.3, ESC),
         (0.3, b":split\r"), (0.5, b"gg"), (0.5, b"/wide\r"), (0.5, b"")],
    )
    yield "less_page", lambda: record(
        "less_page", ["less", SAMPLE.name], [(0.5, b" "), (0.5, b"/emoji\r"), (0.5, b"")]
    )
    # htop limited to its own PID so no host process list is recorded.
    yield "htop_self", lambda: record(
        "htop_self", ["sh", "-c", "exec htop -d 100 -p $$"], [(1.5, b""), (0.5, b"t"), (1.0, b"")]
    )
    yield "tmux_split", record_tmux


def record_tmux():
    """Two panes side by side; status bar stripped of host name and clock."""
    pane = "cat sample.txt; sleep 30"
    try:
        record(
            "tmux_split",
            ["tmux", "-f", "/dev/null", "-L", "fidelity", "new-session", pane,
             ";", "set", "-g", "status-right", "", ";", "set", "-g", "status-left", "[f] ",
             ";", "split-window", "-h", pane],
            [(1.5, b"")],
        )
    finally:
        os.spawnvp(os.P_WAIT, "tmux", ["tmux", "-L", "fidelity", "kill-server"])


def write_sample():
    lines = [f"{i:03} plain ascii line with some words to wrap and scroll" for i in range(1, 9)]
    lines += ["wide: 日本語テキスト 漢字 한국어", "emoji: 😀 👍🏽 🇯🇵", "combining: é ü ñ",
              "rtl: שלום עולם", "tabs:\ta\tb\tc"]
    lines += [f"{i:03} filler {'.' * (i % 60)}" for i in range(14, 60)]
    SAMPLE.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main(names):
    CORPUS.mkdir(exist_ok=True)
    write_sample()
    for name, run in recipes():
        if not names or name in names:
            run()


if __name__ == "__main__":
    main(set(sys.argv[1:]))
