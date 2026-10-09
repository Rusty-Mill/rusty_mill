#!/usr/bin/env python3
"""Summarise a BoGo run: counts, and every failure with the first lines of why."""
import collections
import json
import re
import sys

results, log, config = sys.argv[1:4]
tests = json.load(open(results))["tests"]
counts = collections.Counter(t["actual"] for t in tests.values())
disabled = len(json.load(open(config)).get("DisabledTests", {}))
print(f"BoGo: {counts['PASS']} passed, {counts['FAIL']} failed, "
      f"{counts['SKIP']} skipped (not implemented by the shim or disabled; "
      f"{disabled} disable patterns in config.json)")

text = open(log).read()
parts = re.split(r"^(FAILED|PASSED|UNIMPLEMENTED|SKIPPED|DISABLED) \((.*)\)$", text, flags=re.M)
failed = {parts[i + 1]: parts[i + 2].strip() for i in range(1, len(parts) - 2, 3) if parts[i] == "FAILED"}
for name in sorted(failed):
    lines = [l.strip() for l in failed[name].splitlines() if l.strip()]
    print(f"FAIL {name}: " + " | ".join(lines[:6])[:300])
