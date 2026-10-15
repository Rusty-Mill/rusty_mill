"""Cross-check docs/research/SERDE-STACK-INVENTORY.md against cargo metadata.

usage: python -I verify.py <meta.json> <inventory.md>
"""
import json, re, sys
from collections import Counter

meta = json.load(open(sys.argv[1], encoding="utf-8"))
doc = open(sys.argv[2], encoding="utf-8").read()
DEPS = {"serde": "s", "serde_json": "sj", "toml": "t", "serde_yaml": "y"}
ENGINE = "rusty_multimodal_db_engine"
members = set(meta["workspace_members"])

actual = {}
engine = set()
for p in meta["packages"]:
    if p["id"] not in members:
        continue
    found = {}
    for d in p["dependencies"]:
        name = d.get("rename") or d["name"]
        real = d["name"]
        if real in DEPS:
            kind = d.get("kind") or "normal"
            found.setdefault(real, set()).add(kind)
        if real == ENGINE:
            engine.add(p["name"])
    if found:
        actual[p["name"]] = found

rows = {}
for line in doc.splitlines():
    m = re.match(r"\| `([^`]+)` \| `([^`]+)` \| ([^|]+) \|.*\| ([^|]+) \| ([^|]+) \|$", line)
    if m:
        rows[m.group(1)] = dict(deps=m.group(3).strip(), extra=m.group(4).strip(), ticket=m.group(5).strip())

print("doc rows:", len(rows), "actual crates:", len(actual))
print("in actual, missing from doc:", sorted(set(actual) - set(rows)))
print("in doc, not in actual:", sorted(set(rows) - set(actual)))

totals = Counter()
for n, f in actual.items():
    for k in f:
        totals[k] += 1
print("totals:", dict(totals))

dep_mismatch = []
for n, f in actual.items():
    if n not in rows:
        continue
    want = set()
    for k, kinds in f.items():
        tag = DEPS[k]
        want.add(tag + ("(dev)" if kinds == {"dev"} else ""))
    have = set(rows[n]["deps"].split())
    if want != have:
        dep_mismatch.append((n, sorted(want), sorted(have)))
print("dep-column mismatches:", dep_mismatch)

doc_engine = {n for n, r in rows.items() if "engine" in r["extra"].split(", ")}
print("engine users per metadata (in doc set):", sorted(engine & set(rows)))
print("engine flag missing:", sorted((engine & set(rows)) - doc_engine))
print("engine flag extra:", sorted(doc_engine - engine))
print("tickets:", Counter(r["ticket"] for r in rows.values()))
print("engine crates' tickets:", {n: rows[n]["ticket"] for n in sorted(doc_engine)})
