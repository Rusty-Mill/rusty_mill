#!/usr/bin/env python3
"""Check the synthetic #445 fixtures encode the observed compatibility seams."""

import json
from pathlib import Path

ROOT = Path(__file__).parent / "fixtures"


def load(name: str) -> dict:
    with (ROOT / name).open(encoding="utf-8") as fixture:
        return json.load(fixture)


def main() -> None:
    nexus_push = load("nexus-push.json")
    remind_push = load("remind-me-push.json")
    nexus_pull = load("nexus-pull.json")
    remind_pull = load("remind-me-pull.json")

    assert set(nexus_push) == {"node_id", "records"}
    assert set(remind_push) == {"node_id", "records"}
    nexus_record = nexus_push["records"][0]
    remind_record = remind_push["records"][0]

    shared = set(nexus_record) & set(remind_record)
    assert {"id", "content", "created_at", "updated_at", "node_id"} <= shared
    assert {"deleted_at", "sensitive", "remind_at"} <= set(remind_record) - set(nexus_record)
    assert nexus_record["status"] == "deleted" and remind_record["deleted_at"] is not None
    assert nexus_record["memory_type"] == "semantic"
    assert remind_record["memory_type"] == "decision"

    nexus_query = nexus_pull["request"]["query"]
    remind_query = remind_pull["request"]["query"]
    assert {"since", "since_id"} <= set(nexus_query)
    assert "since_seq" not in nexus_query
    assert "since_seq" in remind_query and remind_pull["response"]["records"][0]["hub_seq"] == 445
    print("memory sync fixtures: compatible envelope and expected semantic gaps verified")


if __name__ == "__main__":
    main()
