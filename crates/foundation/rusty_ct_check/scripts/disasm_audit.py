#!/usr/bin/env python3
"""Count conditional jumps and divisions in named functions of one binary.

usage: disasm_audit.py BINARY LIMITS
LIMITS lines: `<symbol-substring> <max-jcc>`; `div`/`idiv` are always zero.
Every function whose demangled name contains the substring is checked, and a
substring that matches nothing is an error (a renamed function must not pass
silently). A budget of 0 means straight-line code; a non-zero budget pins a
reviewed loop-back count so a new branch shows up as a failure.
Valid for this binary only: rerun after any rustc, flag or target change.
"""
import re
import subprocess
import sys

JCC = re.compile(r"\sj(?!mp)[a-z]+\s")
DIV = re.compile(r"\si?div[a-z]?\s")
HEAD = re.compile(r"^[0-9a-f]+ <(.*)>:$")


def functions(binary):
    out = subprocess.run(["objdump", "-d", "--no-show-raw-insn", "-C", binary],
                         check=True, capture_output=True, text=True).stdout
    name, body = None, []
    for line in out.splitlines():
        m = HEAD.match(line)
        if m:
            if name:
                yield name, body
            name, body = m.group(1), []
        elif name:
            body.append(line)
    if name:
        yield name, body


def main(argv):
    if len(argv) != 3:
        sys.exit(__doc__)
    limits = []
    for raw in open(argv[2]):
        raw = raw.split("#")[0].strip()
        if raw:
            sub, n = raw.rsplit(None, 1)
            limits.append((sub, int(n)))
    funcs = list(functions(argv[1]))
    bad = False
    for sub, budget in limits:
        hits = [(n, b) for n, b in funcs if sub in n]
        if not hits:
            print(f"FAIL no function matches {sub!r}")
            bad = True
        for name, body in hits:
            jcc = sum(1 for l in body if JCC.search(l))
            div = sum(1 for l in body if DIV.search(l))
            ok = jcc <= budget and div == 0
            bad |= not ok
            print(f"{'ok  ' if ok else 'FAIL'} {name[:90]}: jcc={jcc} (max {budget}) div={div}")
    sys.exit(1 if bad else 0)


main(sys.argv)
