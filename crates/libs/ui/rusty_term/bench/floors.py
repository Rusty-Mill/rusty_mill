#!/usr/bin/env python3
"""Wall-clock throughput floors (#889): the slowest of several runs must clear each floor.

Generates the 4 MB `ascii_throughput` and `scroll_heavy` workloads into a
temporary directory, runs `bench_vt_throughput <iterations> <file>` once per
workload per run, and judges the slowest run of each workload against its
floor. 4 MB is about 30k lines at 120 columns, so every run spends most of
its time with the 10,000-line scrollback full: the steady state the floors
are about. The 200 KB `quick` workloads CI uses never fill it.

Also prints each workload's peak RSS (the largest over the runs), for the
memory figure #889 asks for. It needs `os.wait4`, so it is Linux and macOS
only.

Usage (from crates/libs/ui/rusty_term):
  cargo build --release -p rusty_term --bin bench_vt_throughput
  python3 bench/floors.py --bin ../../../../target/release/bench_vt_throughput

Wall-clock numbers depend on the host, so CI does not run this; the
instruction-count gate (`ir_gate.py`) is the CI regression check.

Exit status: 0 every floor met, 1 a workload is below its floor, 2 a usage,
generator or bench error. Stdlib only.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
# MB/s: libghostty-vt at 10k lines of scrollback, backlog §2.1 and §4 item 2.
FLOORS = {"ascii_throughput": 58.0, "scroll_heavy": 16.0}
MBPS = re.compile(r"^(\S+)\s+\d+ bytes\s.*\s([\d.]+) MB/s$", re.MULTILINE)


def parse_mbps(stdout: str) -> dict[str, float]:
    """Workload name to MB/s, from `bench_vt_throughput`'s summary lines."""
    return {name: float(value) for name, value in MBPS.findall(stdout)}


def check(runs: dict[str, list[float]], floors: dict[str, float]) -> tuple[str, list[str]]:
    """A markdown table of the slowest run per workload and the failures."""
    rows = ["| workload | slowest MB/s | runs | floor | |", "|---|---:|---|---:|---|"]
    failures = []
    for name, floor in floors.items():
        values = runs.get(name, [])
        if not values:
            failures.append(f"{name}: has a floor but was not measured")
            continue
        slowest = min(values)
        ok = slowest >= floor
        spread = ", ".join(f"{v:.1f}" for v in values)
        rows.append(f"| {name} | {slowest:.1f} | {spread} | {floor:.1f} | {'ok' if ok else '**below floor**'} |")
        if not ok:
            failures.append(f"{name}: {slowest:.1f} MB/s is below the {floor:.1f} floor")
    return "\n".join(rows) + "\n", failures


def run_bench(binary: str, iterations: int, workload: Path) -> tuple[float, int]:
    """One run over `workload`: its MB/s and the process's peak RSS in KiB."""
    # Reaped with wait4, not Popen.wait, to get this child's own rusage.
    with subprocess.Popen([binary, str(iterations), str(workload)], stdout=subprocess.PIPE, text=True) as proc:
        stdout = proc.stdout.read() if proc.stdout else ""
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
    if proc.returncode != 0:
        raise RuntimeError(f"{workload.name}: exit {proc.returncode}")
    mbps = parse_mbps(stdout).get(workload.stem)
    if mbps is None:
        raise RuntimeError(f"{workload.name}: no MB/s line in the bench output")
    # ru_maxrss is KiB on Linux and bytes on macOS.
    rss_kib = usage.ru_maxrss // 1024 if sys.platform == "darwin" else usage.ru_maxrss
    return mbps, rss_kib


def generate(out_dir: Path) -> None:
    """The default-size (4 MB) workloads that have floors."""
    subprocess.run(
        [sys.executable, str(HERE / "gen_workloads.py"), "--size", "default",
         "--out-dir", str(out_dir), "--only", *FLOORS],
        check=True, stdout=subprocess.DEVNULL,
    )


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", required=True, help="release bench_vt_throughput binary")
    ap.add_argument("--runs", type=int, default=3, help="runs per workload (default 3)")
    ap.add_argument("--iterations", type=int, default=10, help="iterations per run (default 10)")
    args = ap.parse_args(argv)
    if args.runs < 1 or args.iterations < 1:
        print("--runs and --iterations must be at least 1", file=sys.stderr)
        return 2
    runs: dict[str, list[float]] = {name: [] for name in FLOORS}
    rss: dict[str, int] = {name: 0 for name in FLOORS}
    try:
        with tempfile.TemporaryDirectory() as tmp:
            generate(Path(tmp))
            for _ in range(args.runs):
                for name in FLOORS:
                    mbps, kib = run_bench(args.bin, args.iterations, Path(tmp) / f"{name}.vt")
                    runs[name].append(mbps)
                    rss[name] = max(rss[name], kib)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as e:
        print(e, file=sys.stderr)
        return 2
    table, failures = check(runs, FLOORS)
    print(table)
    print("| workload | peak RSS (MiB) |\n|---|---:|")
    for name, kib in rss.items():
        print(f"| {name} | {kib / 1024:.1f} |")
    for failure in failures:
        print(f"FLOORS: {failure}", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
