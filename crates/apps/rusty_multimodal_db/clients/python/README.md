# Reference Python client

A standard-library-only Python 3 client for `rusty_multimodal_db`'s
server/query layer, written against `SERVER-002` (the wire specification)
at protocol version 36 (`PROTOCOL_VERSION` in `rusty_multimodal_db/protocol.py`). It exists to prove that specification is
sufficient (`ECO-FR-007`–`009`, ADR-0043); it is not a packaged product.

```python
import uuid
from rusty_multimodal_db import Client, CompareOp

with Client.connect("127.0.0.1", 7878) as c:
    c.schema.fields                    # what DescribeSchema reported
    c.get(uuid.UUID(int=1))            # [("label", "Ada Lovelace"), ...] or None
    c.filter_eq("label", "ada")        # ids, via the server's name index
    c.query(["label"], where=[("kind", CompareOp.Eq, "person")])
    c.join("relates_to", ["label"], ["label"])   # protocol 12
    c.insert(uuid.uuid4(), [("label", "Grace Hopper"), ("kind", "person"),
                            ("mention_count", 0), ("aliases", [])])  # protocol 13
    c.link(uuid.UUID(int=1), uuid.UUID(int=2), "mentored_by")  # protocol 14, any label
    c.replace(uuid.UUID(int=1), [("label", "Ada King"), ("kind", "person"),
                                ("mention_count", 9), ("aliases", [])])  # protocol 15, whole record; False if unknown
    c.list_tables()                    # (["memory", "entity"], "memory") on a two-table server — protocol 16
    c.use_table("entity")              # every following request is served from that table
    c.join("mentions", ["content"], ["label"])  # crosses tables when the relation's target_table says so
    c.delete(uuid.UUID(int=1))         # protocol 17: True when gone (with every edge touching it), False if unknown
    c.compact()                        # protocol 18: fold the logs, drop retired slots; returns the counts reclaimed
    c.replace_if(uuid.UUID(int=1), [...], ("mention_count", CompareOp.Lt, 5))  # protocol 19: "replaced" / "refused" / "notfound"
    c.page("mention_count", None, 100)  # protocol 20: one ordered keyset page; pass the last row's (value, id) as `after` for the next
    c.count_edges("relates_to")        # protocol 21: how many edges the table holds under a label, one round trip
    c.write_batch([("delete", uuid.UUID(int=1))], atomic=False)  # protocol 22: a batch of runtime writes in one round trip
    c.page_desc("mention_count", None, 100)       # protocol 28: the same page walked descending
    c.filtered_page("mention_count", [("kind", CompareOp.Eq, "person")], None, 100)  # protocol 26 (`filtered_page_desc`: 28)
    c.describe_nullable()              # protocol 32: which fields read and write as None
    c.metrics()                        # protocol 23: the server's Prometheus text
    c.backup("nightly")                # protocol 24: (files, bytes) copied under the server's backup root
    files = c.fetch_snapshot()         # protocol 25: replication token only
    c.fetch_since(epoch, after, 1000)  # protocol 34: change-log entries after `after`; ServerError Gone means re-fetch a snapshot
    c.fetch_snapshot_chunked(dir)      # protocol 36: a table over 8 MiB, streamed in 4 MiB chunks with SHA-256 checks
```

The client does not open transaction sessions (`Begin`/`BeginWith`/`Commit`);
the session bit constants (`SESSION_*`, including `SESSION_STRICT_COMMIT`,
protocol 35) are there for encoding and the wire fixture, and the Rust client
(`SchemaDrivenClient::begin_with`) is the session reference.

Verification, both in CI:

- offline: `python3 -m unittest discover -s clients/python/tests -v` —
  every line of `tests/fixtures/wire-vectors.txt` decodes and re-encodes
  byte-for-byte;
- live: `tests/server_python_client.rs` (under `cargo test
  --all-features`) starts a real `Entity` server and runs `driver.py`
  against it, at this build's protocol version and at a hand-negotiated 10.

The client is version-pinned: when the wire grows, the fixture and
`SERVER-002` grow in the same change; this client is updated when someone
wants the new shape and stays correct at the version it declares.
