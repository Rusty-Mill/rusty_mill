#!/usr/bin/env python3
"""Deterministic throughput gate: instructions per input byte, under cachegrind.

For each workload in the ceilings file, runs
`bench_vt_throughput --once <workload>.vt` under valgrind's cachegrind
(instruction counting only, no cache simulation), divides the instruction
count by the workload size, and fails if it exceeds the workload's ceiling.

Instruction counts are deterministic for a given binary and input (two runs
differ by well under 0.01%), unlike wall-clock MB/s on shared runners. One
host-dependent input remains: glibc picks its memmove/memcpy variant from the
CPU, and on hosts with ERMS/FSRM the `rep movsb` variant counts about 8x more
instructions on scroll-heavy input. `HWCAPS_MASK` pins the vector variant on
every host so the count does not depend on the runner's CPU.

Usage (from crates/libs/ui/rusty_term):
  python3 bench/gen_workloads.py --size quick
  python3 bench/ir_gate.py --bin ../../../../target/release/bench_vt_throughput

Exit status: 0 all within ceilings, 1 a workload is over its ceiling or
missing, 2 a usage, ceilings-file or valgrind error. Stdlib only.
"""

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
HWCAPS_MASK = (
    "glibc.cpu.hwcaps=-ERMS,-FSRM,-Prefer_ERMS,-Prefer_FSRM,"
    "-AVX512F,-AVX512VL,-AVX512BW,-AVX2,-AVX"
)
REFS = re.compile(r"I\s+refs:\s+([\d,]+)")


def parse_ceilings(text: str) -> list[tuple[str, float]]:
    """`<workload> <max instructions per byte>` per line; `#` comments allowed."""
    ceilings = []
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) != 2:
            raise ValueError(f"line {n}: expected `<workload> <max instructions/byte>`")
        try:
            limit = float(parts[1])
        except ValueError:
            raise ValueError(f"line {n}: `{parts[1]}` is not a number") from None
        if limit <= 0:
            raise ValueError(f"line {n}: ceiling must be positive")
        ceilings.append((parts[0], limit))
    if not ceilings:
        raise ValueError("no ceilings defined")
    return ceilings


def parse_refs(valgrind_stderr: str) -> int:
    """The `I refs:` total from cachegrind's summary."""
    match = REFS.search(valgrind_stderr)
    if not match:
        raise ValueError("no `I refs:` line in valgrind output")
    return int(match.group(1).replace(",", ""))


def check(measured: dict[str, float], ceilings: list[tuple[str, float]]) -> tuple[str, list[str]]:
    """A markdown table and the failures: over a ceiling, or not measured."""
    rows = ["| workload | instructions/byte | ceiling | |", "|---|---:|---:|---|"]
    failures = []
    for name, limit in ceilings:
        if name not in measured:
            failures.append(f"{name}: has a ceiling but was not measured")
            continue
        value = measured[name]
        ok = value <= limit
        rows.append(f"| {name} | {value:.1f} | {limit:.1f} | {'ok' if ok else '**over ceiling**'} |")
        if not ok:
            failures.append(f"{name}: {value:.1f} instructions/byte is over the {limit:.1f} ceiling")
    return "\n".join(rows) + "\n", failures


def measure(binary: str, workload: Path) -> float:
    """Instructions per byte for one `--once` pass over `workload`."""
    env = dict(os.environ, GLIBC_TUNABLES=HWCAPS_MASK)
    proc = subprocess.run(
        ["valgrind", "--tool=cachegrind", "--cache-sim=no", "--cachegrind-out-file=/dev/null",
         binary, "--once", str(workload)],
        env=env, capture_output=True, text=True, check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"{workload.name}: exit {proc.returncode}\n{proc.stderr[-2000:]}")
    return parse_refs(proc.stderr) / workload.stat().st_size


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", required=True, help="release bench_vt_throughput binary")
    ap.add_argument("--ceilings", default=str(HERE / "ir_ceilings.txt"))
    ap.add_argument("--workloads", default=str(HERE / "workloads"))
    args = ap.parse_args(argv)
    try:
        ceilings = parse_ceilings(Path(args.ceilings).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        print(f"{args.ceilings}: {e}", file=sys.stderr)
        return 2
    measured = {}
    for name, _ in ceilings:
        path = Path(args.workloads) / f"{name}.vt"
        if not path.is_file():
            continue  # reported as "not measured" by check()
        try:
            measured[name] = measure(args.bin, path)
        except (OSError, RuntimeError, ValueError) as e:
            print(e, file=sys.stderr)
            return 2
    table, failures = check(measured, ceilings)
    print(table)
    for failure in failures:
        print(f"IR GATE: {failure}", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
