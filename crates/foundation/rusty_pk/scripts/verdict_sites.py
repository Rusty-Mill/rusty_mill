#!/usr/bin/env python3
"""Check that every valgrind taint report is the branch on a final verdict.

usage: verdict_sites.py BINARY REPORT_FILE [EXPECTED_COUNT] [--output FUNCTION]...

The ECDH code folds all secret-dependent validity (scalar in range, result not the point at
infinity) into one value and passes it to `ecdh::ensure`; the caller then branches on the
returned `Result`, which is the only branch allowed to depend on the secret. For each
`at 0xADDR: function` line of each diagnostic in REPORT_FILE this script maps the address to the
binary and requires the instruction to be a
conditional jump with a `call ...ecdh::ensure` within the three instructions before it and no
other jump, call or return in between. Anything else (a report in `mul`, `select`, a limb loop,
a mis-mapped address) fails. Exits 1 on any failure or if EXPECTED_COUNT differs.

Load mapping: the load base is not assumed (valgrind has placed PIEs at different addresses across
releases). REPORT_FILE must contain exactly one `verdict_sites: load anchor SYMBOL at 0xADDR` line,
printed by the program at run time; the base is ADDR minus SYMBOL's address in BINARY. A missing,
repeated or conflicting anchor, a SYMBOL that is absent or not unique in BINARY, or a base that is
negative or not page-aligned fails before any report is classified. The function name in each
report must still match the symbol found at the mapped address, which cross-checks the base.

`--output FUNCTION` (repeatable) names a function that only handles values the caller publishes
anyway, such as ECDSA's `r` and `s`: a report whose call stack passes through it is accepted
and listed, but not counted against EXPECTED_COUNT, which counts verdict branches only. A
report with no such frame must still be a verdict branch.

Drift detection and a narrowing of what the taint run may report; not a proof.
"""
import re
import subprocess
import sys

PAGE = 0x1000
ANCHOR = re.compile(r"^verdict_sites: load anchor (\S+) at 0x([0-9a-fA-F]+)$")
DIAG = re.compile(r"^==\d+== (Conditional|Use of)")
AT = re.compile(r"^==\d+==    at 0x([0-9A-Fa-f]+): (.*?) \(in ")
BY = re.compile(r"^==\d+==    by 0x[0-9A-Fa-f]+: (.*?) \(in ")
HEAD = re.compile(r"^([0-9a-f]+) <(.*)>:$")
INSN = re.compile(r"^\s+([0-9a-f]+):\s+(\S+)\s*(.*)$")


class MappingError(Exception):
    """The runtime-to-binary address mapping cannot be established unambiguously."""


def disassemble(binary):
    """(instructions, function of each address, start addresses of each function name)."""
    out = subprocess.run(["objdump", "-d", "--no-show-raw-insn", "-C", binary],
                         check=True, capture_output=True, text=True).stdout
    return parse_disassembly(out)


def parse_disassembly(out: str):
    insns, func_of, starts, cur = [], {}, {}, None
    for line in out.splitlines():
        m = HEAD.match(line)
        if m:
            cur = m.group(2)
            starts.setdefault(cur, []).append(int(m.group(1), 16))
            continue
        m = INSN.match(line)
        if m and cur:
            addr = int(m.group(1), 16)
            insns.append((addr, m.group(2), m.group(3)))
            func_of[addr] = cur
    return insns, func_of, starts


def anchors(path: str) -> list[tuple[str, int]]:
    """Every `(symbol, runtime address)` load anchor line in a report file."""
    found = []
    for line in open(path, encoding="utf-8", errors="replace"):
        m = ANCHOR.match(line.rstrip("\n"))
        if m:
            found.append((m.group(1), int(m.group(2), 16)))
    return found


def load_base(found: list[tuple[str, int]], starts: dict[str, list[int]]) -> int:
    """The load base implied by exactly one anchor; MappingError if it is missing or ambiguous."""
    if len(found) != 1:
        raise MappingError(f"want exactly 1 load anchor line, found {len(found)}")
    sym, runtime = found[0]
    at = starts.get(sym, [])
    if len(at) != 1:
        raise MappingError(f"anchor symbol {sym!r} appears {len(at)} times in the binary, want 1")
    base = runtime - at[0]
    if base < 0 or base % PAGE:
        raise MappingError(f"anchor {sym!r} at 0x{runtime:x} vs 0x{at[0]:x} in the binary gives "
                           f"base {base:#x}, not a non-negative multiple of 0x{PAGE:x}")
    return base


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


def check_site(addr, name, insns, func_of, index, base):
    file_addr = addr - base
    i = index.get(file_addr)
    if i is None:
        return f"0x{addr:x}: no instruction at 0x{file_addr:x} (load base 0x{base:x})"
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
    insns, func_of, starts = disassemble(args[0])
    index = {a: n for n, (a, _, _) in enumerate(insns)}
    try:
        base = load_base(anchors(args[1]), starts)
    except MappingError as e:
        print(f"FAIL load mapping: {e}")
        sys.exit(1)
    print(f"   load base 0x{base:x}")
    found = reports(args[1])
    verdicts, bad = [], []
    for a, n, stack in found:
        via = through_output(stack, outputs)
        if via:
            print(f"   report at 0x{a:x} in {n} (output, via {via})")
            continue
        print(f"   report at 0x{a:x} in {n}")
        verdicts.append(a)
        e = check_site(a, n, insns, func_of, index, base)
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
