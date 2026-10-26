"""Pick superseded `main` rust-cache entries for deletion.

Reads `gh cache list --json id,key,createdAt` output on stdin and prints the
ids to delete, one per line. Swatinem/rust-cache keys end in a hash of
Cargo.lock and the manifests, so every change to `main` makes the daily sweep
save a new entry beside the old one; the repository's 10 GB budget then
evicts whatever was used least recently, which has included the live clippy
cache. Within a family (the key without that last hash) only the newest
entry is ever restored first, so the others are dead weight.
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class Cache:
    id: int
    key: str
    created_at: str

    @property
    def family(self) -> str:
        return self.key.rsplit("-", 1)[0]


def superseded(caches: list[Cache]) -> list[int]:
    """Return the ids of every entry older than the newest in its family."""
    newest: dict[str, Cache] = {}
    for cache in caches:
        kept = newest.get(cache.family)
        if kept is None or (cache.created_at, cache.id) > (kept.created_at, kept.id):
            newest[cache.family] = cache
    keep = {cache.id for cache in newest.values()}
    return sorted(cache.id for cache in caches if cache.id not in keep)


def parse(text: str) -> list[Cache]:
    """Parse `gh cache list --json id,key,createdAt` output."""
    return [Cache(int(row["id"]), str(row["key"]), str(row["createdAt"])) for row in json.loads(text)]


def main() -> None:
    for cache_id in superseded(parse(sys.stdin.read())):
        print(cache_id)


if __name__ == "__main__":
    main()
