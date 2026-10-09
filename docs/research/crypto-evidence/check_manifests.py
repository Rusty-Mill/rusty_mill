#!/usr/bin/env python3
"""Checks every vectors directory against its MANIFEST.txt; used by collect.sh.

usage: check_manifests.py VECTORS_DIR...
A manifest line is `<sha256>  <file>`; other lines are prose. Fails (exit 1) on a
file whose hash differs from the manifest, a listed file that is missing, and a
`*.json`/`*.txt` file (other than MANIFEST.txt) that the manifest does not list,
so a replaced, edited, deleted or smuggled-in vector cannot yield a record that
says "all checks passed" (Codex round 3 on #540).
"""
import hashlib
import os
import re
import sys

LINE = re.compile(r"^([0-9a-f]{64})  (\S+)$")


def check(directory):
    """Returns (checked_file_count, problems) for one vectors directory."""
    manifest = os.path.join(directory, "MANIFEST.txt")
    if not os.path.isfile(manifest):
        return 0, [f"{directory}: no MANIFEST.txt"]
    pinned = {}
    with open(manifest) as f:
        for line in f:
            m = LINE.match(line.rstrip("\n"))
            if m:
                pinned[m.group(2)] = m.group(1)
    problems = []
    if not pinned:
        problems.append(f"{directory}: MANIFEST.txt pins no files")
    present = {n for n in os.listdir(directory) if n.endswith((".json", ".txt")) and n != "MANIFEST.txt"}
    for name in sorted(pinned.keys() - present):
        problems.append(f"{directory}: {name} is listed in MANIFEST.txt but missing")
    for name in sorted(present - pinned.keys()):
        problems.append(f"{directory}: {name} is not listed in MANIFEST.txt")
    for name in sorted(pinned.keys() & present):
        with open(os.path.join(directory, name), "rb") as f:
            actual = hashlib.sha256(f.read()).hexdigest()
        if actual != pinned[name]:
            problems.append(f"{directory}: {name} hash {actual} != manifest {pinned[name]}")
    return len(pinned.keys() & present), problems


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    bad = False
    for d in argv[1:]:
        n, problems = check(d)
        print(f"manifest {d}: {n} file(s) match" + ("" if not problems else f", {len(problems)} problem(s)"))
        for p in problems:
            print(f"!! {p}")
        bad |= bool(problems)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
