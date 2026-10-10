#!/usr/bin/env python3
"""Check that every valgrind taint report is the branch on a final verdict.

usage: verdict_sites.py BINARY REPORT_FILE [EXPECTED_COUNT]

The ECDH code folds all secret-dependent validity (scalar in range, result not the point at
infinity) into one value and passes it to `ecdh::ensure`; the caller then branches on the
returned `Result`, which is the only branch allowed to depend on the secret. For each
`at 0xADDR: function` line of each diagnostic in REPORT_FILE this script maps the address to the
binary (valgrind loads a PIE at 0x108000; the function name in the report must match the symbol
found there, which cross-checks that assumption) and requires the instruction to be a
conditional jump with a `call ...ecdh::ensure` within the three instructions before it and no
other jump, call or return in between. Anything else (a report in `mul`, `select`, a limb loop,
a mis-mapped address) fails. Exits 1 on any failure or if EXPECTED_COUNT differs.

Drift detection and a narrowing of what the taint run may report; not a proof.
"""
import re
import subprocess
import sys

BASE = 0x108000
DIAG = re.compile(r"^==\d+== (Conditional|Use of)")
AT = re.compile(r"^==\d+==    at 0x([0-9A-Fa-f]+): (.*?) \(in ")
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
    """The (address, function) of the top frame of every diagnostic."""
    found, want_at = [], False
    for line in open(path, encoding="utf-8", errors="replace"):
        if DIAG.match(line):
            want_at = True
            continue
        m = AT.match(line) if want_at else None
        if m:
            found.append((int(m.group(1), 16), m.group(2)))
            want_at = False
    return found


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
    if len(argv) not in (3, 4):
        sys.exit(__doc__)
    insns, func_of = disassemble(argv[1])
    index = {a: n for n, (a, _, _) in enumerate(insns)}
    found = reports(argv[2])
    bad = [e for e in (check_site(a, n, insns, func_of, index) for a, n in found) if e]
    for a, n in found:
        print(f"   report at 0x{a:x} in {n}")
    for e in bad:
        print(f"FAIL {e}")
    if len(argv) == 4 and len(found) != int(argv[3]):
        print(f"FAIL {len(found)} reports, expected {argv[3]}")
        bad.append("count")
    if not found:
        print("FAIL no reports to check")
        bad.append("none")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main(sys.argv)
