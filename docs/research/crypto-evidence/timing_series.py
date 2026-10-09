#!/usr/bin/env python3
"""Repeated timing runs for collect.sh: build, run, summarise, and judge them.

Reads one `crate|test|label` per line on stdin. For every crate it builds that
package's `timing` test target, takes the executable from Cargo's own
compiler-artifact JSON (so a custom CARGO_TARGET_DIR, stale binaries and
same-named tests in other crates cannot be mixed up), and runs each requested
test REPS times. Prints the record; exits 1 after printing a `!! FAILED` line
for anything that makes the record untrustworthy.

Alarms are counted from the printed |t| values (>= THRESHOLD), not from process
exit codes: the A/A calibration tests only print |t| and never fail at the
threshold. Exit codes are tracked separately, as execution failures.
"""
import hashlib
import json
import os
import subprocess
import sys

THRESHOLD = 4.5  # rusty_ct_check::timing::THRESHOLD; the leak tests fail at |t| >= this
MARKER = "|t| = "


class Failure(Exception):
    """The record cannot be trusted; the message goes on a `!! FAILED` line."""


def package_name(package_id: str) -> str:
    """`path+file:///x/rusty_pk#0.1.0` and `registry+...#rusty_pk@0.1.0` -> `rusty_pk`."""
    head, _, tail = package_id.rpartition("#")
    if "@" in tail:
        return tail.split("@")[0]
    return head.rstrip("/").rsplit("/", 1)[-1]


def resolve(package: str, cargo: str = "cargo") -> str:
    """Build `package`'s `timing` test target and return the executable Cargo reports."""
    cmd = [cargo, "test", "--release", "-p", package, "--test", "timing",
           "--no-run", "--message-format=json"]
    done = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if done.returncode != 0:
        raise Failure(f"{package}: building the timing test failed (exit {done.returncode})")
    found = []
    for line in done.stdout.splitlines():
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
            continue
        if msg["target"]["name"] != "timing" or not msg.get("profile", {}).get("test"):
            continue
        if package_name(msg["package_id"]) == package:
            found.append(msg["executable"])
    if len(found) != 1:
        raise Failure(f"{package}: expected exactly one timing test executable from cargo, got {len(found)}")
    if not os.path.isfile(found[0]):
        raise Failure(f"{package}: cargo reported {found[0]}, which does not exist")
    return found[0]


def identity(package: str, exe: str) -> str:
    with open(exe, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()[:16]
    return f"{package}: timing test executable {exe} (sha256 {digest}...)"


def has_test(exe: str, test: str) -> bool:
    done = subprocess.run([exe, "--ignored", "--list"], stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL, text=True)
    return any(line == f"{test}: test" for line in done.stdout.splitlines())


def observe(exe: str, test: str):
    """One run: (|t| or None, exit status)."""
    done = subprocess.run([exe, "--ignored", "--nocapture", test], stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True)
    for line in done.stdout.splitlines():
        if MARKER in line and "panicked" not in line:
            try:
                return float(line.split(MARKER, 1)[1].split()[0]), done.returncode
            except (ValueError, IndexError):
                break
    return None, done.returncode


def alarms(values) -> int:
    return sum(1 for v in values if v >= THRESHOLD)


def summary(values) -> str:
    s = sorted(values)
    n = len(s)
    return (f"n={n} min={s[0]} median={s[(n + 1) // 2 - 1]} p90={s[max(int(n * 0.9) - 1, 0)]} "
            f"max={s[-1]} at_or_above_{THRESHOLD}={alarms(s)}")


def run(specs, reps: int, cargo: str = "cargo", out=sys.stdout) -> int:
    failed = []

    def fail(msg):
        failed.append(msg)
        print(f"!! FAILED: {msg}", file=out)

    if reps < 1:
        fail(f"REPS must be a positive number of repetitions, got {reps}")
        return 1
    exes = {}       # crate -> executable, or None after a failure
    baseline = {}   # crate -> alarms in that crate's A/A series
    for crate, test, label in specs:
        if crate not in exes:
            try:
                exes[crate] = resolve(crate, cargo)
                print(identity(crate, exes[crate]), file=out)
            except Failure as e:
                exes[crate] = None
                fail(str(e))
        exe = exes[crate]
        if exe is None:
            fail(f"{label}: skipped, no executable for {crate}")
            continue
        if not has_test(exe, test):
            fail(f"{label}: test {test} not found in {exe}")
            continue
        values, bad_exit, unparsed = [], 0, 0
        for _ in range(reps):
            v, status = observe(exe, test)
            if v is None:
                unparsed += 1
                continue
            values.append(v)
            if status != 0 and v < THRESHOLD:
                bad_exit += 1  # failed, but not by crossing the threshold
        crossings = alarms(values)
        if values:
            print(f"{label}: {summary(values)}; runs failing without a threshold crossing: {bad_exit}", file=out)
            print("   raw: " + " ".join(str(v) for v in values), file=out)
        if unparsed or len(values) != reps:
            fail(f"{label}: {len(values)} of {reps} runs produced a |t| ({unparsed} did not run)")
        if bad_exit:
            fail(f"{label}: {bad_exit} run(s) exited non-zero below the threshold")
        if label.startswith("A/A"):
            baseline[crate] = crossings
        elif crate not in baseline:
            fail(f"{label}: no A/A baseline was measured for {crate}")
        else:
            # A leak test fails the record if it alarms clearly more often than the no-leak baseline.
            limit = 2 * baseline[crate] + 1
            if crossings > limit:
                fail(f"{label}: {crossings} alarms of {reps} exceeds the baseline allowance "
                     f"({limit}; baseline {baseline[crate]})")
    return 1 if failed else 0


def main() -> int:
    reps = int(os.environ.get("REPS", "40"))
    specs = [tuple(line.split("|", 2)) for line in sys.stdin.read().splitlines() if line.strip()]
    return run(specs, reps)


if __name__ == "__main__":
    sys.exit(main())
