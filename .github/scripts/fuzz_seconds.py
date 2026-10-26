"""Validate the long fuzz run's seconds-per-target input before any job starts.

    fuzz_seconds.py <value>

Prints the value and exits 0 when it is a whole number in 1..MAX_SECONDS, else
exits 1 with an error. Zero is refused because libFuzzer reads
`-max_total_time=0` as no limit. Checked as text first, so no input is too
large to parse.
"""

from __future__ import annotations

import re
import sys

# The 6 h hosted-runner limit, less the job's build time (tls-fuzz-long.yml).
MAX_SECONDS = 19800


def parse(value: str) -> int:
    """Return `value` as seconds, or raise ValueError saying why it is refused."""
    if not re.fullmatch(r"[1-9][0-9]{0,4}", value):
        raise ValueError(f"seconds must be a whole number from 1 to {MAX_SECONDS}, got {value!r}")
    seconds = int(value)
    if seconds > MAX_SECONDS:
        raise ValueError(f"at most {MAX_SECONDS} s per target fits the job limit, got {seconds}")
    return seconds


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: fuzz_seconds.py <value>", file=sys.stderr)
        return 1
    try:
        print(parse(argv[1]))
    except ValueError as error:
        print(f"::error::{error}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
