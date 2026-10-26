#!/usr/bin/env python3
"""Check that every valgrind taint report is the branch on a final verdict.

usage: verdict_sites.py BINARY REPORT_FILE [EXPECTED_COUNT] [--output FUNCTION]...

The ECDH code folds all secret-dependent validity (scalar in range, result not the point at
infinity) into one value and passes it to `ecdh::ensure`; the caller then branches on the
returned `Result`, which is the only branch allowed to depend on the secret. For each
`at 0xADDR: function` line of each diagnostic in REPORT_FILE this script maps the address to the
binary (valgrind loads a PIE at 0x108000; the function name in the report must match the symbol
found there, which cross-checks that assumption) and requires the instruction to be a
conditional jump with a `call ...ecdh::ensure` within the three instructions before it and no
other jump, call or return in between. Anything else (a report in `mul`, `select`, a limb loop,
a mis-mapped address) fails. Exits 1 on any failure or if EXPECTED_COUNT differs.

`--output FUNCTION` (repeatable) names a function that only handles values the caller publishes
anyway, such as ECDSA's `r` and `s`: a report whose call stack passes through it is accepted
and listed, but not counted against EXPECTED_COUNT, which counts verdict branches only. A
report with no such frame must still be a verdict branch.

Drift detection and a narrowing of what the taint run may report; not a proof.
"""
import re
import subprocess
import sys

BASE = 0x108000
DIAG = re.compile(r"^==\d+== (Conditional|Use of)")
AT = re.compile(r"^==\d+==    at 0x([0-9A-Fa-f]+): (.*?) \(in ")
BY = re.compile(r"^==\d+==    by 0x[0-9A-Fa-f]+: (.*?) \(in ")
HEAD = re.compile(r"^([0-9a-f]+) <(.*)>:$")
INSN = re.compile(r"^\s+([0-9a-f]+):\s+(\S+)\s*(.*)$")


def disassemble(binary):
    out = subprocess.run(["objdump", "-d", "--no-show-raw-insn", "-C", binary],
                         check=True, capture_output=True, text=True).stdout
    insns, func_of, cur = [], {}, None
    for line in out.splitlines():
        m = HEAD.match(line)
        if m:
            cur = m.group(2)
            continue
        m = INSN.match(line)
        if m and cur:
            addr = int(m.group(1), 16)
            insns.append((addr, m.group(2), m.group(3)))
            func_of[addr] = cur
    return insns, func_of


def reports(path):
    """(address, function, stack) of every diagnostic: the top frame's address and function,
    and the function of every frame (top first)."""
    found, want_at = [], False
    for line in open(path, encoding="utf-8", errors="replace"):
        if DIAG.match(line):
            want_at = True
            continue
        m = AT.match(line) if want_at else None
        if m:
            found.append((int(m.group(1), 16), m.group(2), [m.group(2)]))
            want_at = False
            continue
        m = BY.match(line)
        if m and found:
            found[-1][2].append(m.group(1))
    return found


def through_output(stack, outputs):
    """The output function the stack passes through, or None."""
    for frame in stack:
        for name in outputs:
            if name in frame:
                return name
    return None


def check_site(addr, name, insns, func_of, index):
    file_addr = addr - BASE
    i = index.get(file_addr)
    if i is None:
        return f"0x{addr:x}: no instruction at 0x{file_addr:x} (is the load base still 0x{BASE:x}?)"
    sym = func_of[file_addr]
    if name.strip("<>") not in sym and sym.strip("<>") not in name:
        return f"0x{addr:x}: report says {name!r} but the binary has {sym!r} there"
    if not insns[i][1].startswith("j") or insns[i][1].startswith("jmp"):
        return f"0x{addr:x} in {sym}: not a conditional jump ({insns[i][1]})"
    for back in range(1, 4):
        if i - back < 0:
            break
        _, op, args = insns[i - back]
        if op.startswith("call"):
            return None if "ecdh::ensure" in args else f"0x{addr:x} in {sym}: call to {args}, not ensure"
        if op.startswith("j") or op.startswith("ret"):
            break
    return f"0x{addr:x} in {sym}: no call to ecdh::ensure just before this jump"


def main(argv):
    args, outputs = [], []
    rest = iter(argv[1:])
    for a in rest:
        if a == "--output":
            name = next(rest, None)
            if not name:
                sys.exit(__doc__)
            outputs.append(name)
        else:
            args.append(a)
    if len(args) not in (2, 3):
        sys.exit(__doc__)
    insns, func_of = disassemble(args[0])
    index = {a: n for n, (a, _, _) in enumerate(insns)}
    found = reports(args[1])
    verdicts, bad = [], []
    for a, n, stack in found:
        via = through_output(stack, outputs)
        if via:
            print(f"   report at 0x{a:x} in {n} (output, via {via})")
            continue
        print(f"   report at 0x{a:x} in {n}")
        verdicts.append(a)
        e = check_site(a, n, insns, func_of, index)
        if e:
            bad.append(e)
    for e in bad:
        print(f"FAIL {e}")
    if len(args) == 3 and len(verdicts) != int(args[2]):
        print(f"FAIL {len(verdicts)} verdict reports, expected {args[2]}")
        bad.append("count")
    if not found:
        print("FAIL no reports to check")
        bad.append("none")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main(sys.argv)
