# Reference Python client

A standard-library-only Python 3 client for `rusty_multimodal_db`'s
server/query layer, written against `SERVER-002` (the wire specification)
at protocol version 18. It exists to prove that specification is
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
```

Verification, both in CI:

- offline: `python3 -m unittest discover -s clients/python/tests -v` —
  every line of `tests/fixtures/wire-vectors.txt` decodes and re-encodes
  byte-for-byte;
- live: `tests/server_python_client.rs` (under `cargo test
  --all-features`) starts a real `Entity` server and runs `driver.py`
  against it, at protocol 18 and at a hand-negotiated 10.

The client is version-pinned: when the wire grows, the fixture and
`SERVER-002` grow in the same change; this client is updated when someone
wants the new shape and stays correct at the version it declares.
