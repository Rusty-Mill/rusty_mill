"""ECO-FR-008 (ii): the live driver tests/server_python_client.rs runs.

Usage: python3 driver.py HOST:PORT HELLO_VERSION

Connects to an Entity server, performs one of each read shape plus one
write, and prints ``key=value`` lines the Rust test asserts on. Nothing
here is a library API; it exists so a Rust test can prove the Python
client against a real server without parsing JSON.
"""

import sys
import uuid

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from rusty_multimodal_db import Client, AggregateFn, CompareOp, ServerError, UnsupportedError  # noqa: E402


def main() -> int:
    host, port = sys.argv[1].rsplit(":", 1)
    hello = int(sys.argv[2])
    with Client.connect(host, int(port), protocol_version=hello) as c:
        print(f"negotiated={c.server_protocol_version}")
        print("schema_fields=" + ",".join(f.name for f in c.schema.fields))
        print("relations=" + ",".join(sorted(r.name for r in c.relations)))

        ada = c.get(uuid.UUID(int=1))
        print(f"get_fields={len(ada)}")
        aliases = dict(ada).get("aliases")
        print("get_aliases=" + ("|".join(aliases) if aliases is not None else "-"))
        print(f"get_missing={'none' if c.get(uuid.UUID(int=99)) is None else 'found'}")

        ids = c.filter_eq("label", "ADA")
        print(f"filter_eq_ids={len(ids)}")
        print(f"filter_eq_first={ids[0].int if ids else '-'}")

        rows = c.query(["label"], where=[("kind", CompareOp.Eq, "person")])
        print(f"query_rows={len(rows)}")

        groups = c.aggregate(["kind"], [(AggregateFn.Count, None)])
        print(f"aggregate_groups={len(groups)}")

        print(f"update={str(c.update(uuid.UUID(int=1), 'mention_count', 42)).lower()}")
        try:
            c.update(uuid.UUID(int=1), "label", "x")
            print("update_label=allowed")
        except UnsupportedError:
            print("update_label=unsupported")

        print(f"neighbors={len(c.neighbors(uuid.UUID(int=1)))}")

        try:
            joined = c.join("relates_to", ["label"], ["label"])
            print(f"join_rows={len(joined)}")
        except UnsupportedError as e:
            print(f"join_rows=unsupported:{e}")

        # Protocol 13 (INS-FR-008): one insert, then the duplicate refusal.
        new = uuid.UUID(int=77)
        try:
            c.insert(new, [("label", "Grace Hopper"), ("kind", "person"), ("mention_count", 1), ("aliases", ["Amazing Grace"])])
            print("insert=ok")
            try:
                c.insert(new, [("label", "Grace Hopper"), ("kind", "person"), ("mention_count", 1), ("aliases", [])])
                print("insert_again=ok")
            except ServerError as e:
                print(f"insert_again={e.code.name}")
            print(f"insert_get={len(c.get(new))}")
        except UnsupportedError as e:
            print(f"insert=unsupported:{e}")

        # Protocol 14 (LNK-FR-012): link the new entity to Ada under a
        # brand-new label, then read it back from both ends.
        try:
            c.link(uuid.UUID(int=1), new, "mentored_by")
            c.link(uuid.UUID(int=1), new, "mentored_by")  # insert-or-ignore
            print("link=ok")
            print(f"link_neighbors={len(c.neighbors(new, 'mentored_by'))}")
            print(f"link_kinds={','.join(sorted(c.relation_kinds()))}")
        except UnsupportedError as e:
            print(f"link=unsupported:{e}")

        # Protocol 15 (REP-FR-007): replace the new entity whole — a new
        # alias resolves, the edge from Ada survives; an unknown id is
        # False.
        try:
            replaced = c.replace(new, [("label", "Grace Hopper"), ("kind", "person"), ("mention_count", 2), ("aliases", ["Grandma COBOL"])])
            print(f"replace={'ok' if replaced else 'notfound'}")
            print(f"replace_by_alias={len(c.filter_eq('label', 'grandma cobol'))}")
            print(f"replace_neighbors={len(c.neighbors(new, 'mentored_by'))}")
            print(f"replace_unknown={'ok' if c.replace(uuid.UUID(int=4242), [('label', 'x'), ('kind', 'x'), ('mention_count', 0), ('aliases', [])]) else 'notfound'}")
        except UnsupportedError as e:
            print(f"replace=unsupported:{e}")

        # Protocol 19 (GRD-FR-006): a guarded replace — mention_count is 2
        # after the replace above, so a guard of "stored < 5" holds and one
        # of "stored < 1" is refused with nothing written; an unknown id is
        # notfound.
        try:
            fields = [("label", "Grace Hopper"), ("kind", "person"), ("mention_count", 3), ("aliases", ["Grandma COBOL"])]
            print(f"replace_if_holds={c.replace_if(new, fields, ('mention_count', CompareOp.Lt, 5))}")
            print(f"replace_if_refused={c.replace_if(new, fields, ('mention_count', CompareOp.Lt, 1))}")
            print(f"replace_if_stored={dict(c.get(new))['mention_count']}")
            print(f"replace_if_unknown={c.replace_if(uuid.UUID(int=4242), fields, ('mention_count', CompareOp.Lt, 5))}")
        except UnsupportedError as e:
            print(f"replace_if=unsupported:{e}")

        # Protocol 20 (PAG-FR-005): ordered keyset pages by mention_count —
        # two pages of three walk every entity in (mention_count, id) order.
        try:
            first = c.page("mention_count", None, 3)
            rest = c.page("mention_count", (dict(first[-1][1])["mention_count"], first[-1][0]), 100)
            counts = [dict(f)["mention_count"] for _, f in first + rest]
            print(f"page_total={len(first) + len(rest)}")
            print(f"page_sorted={'yes' if counts == sorted(counts) else 'no'}")
            print(f"page_disjoint={'yes' if not {r for r, _ in first} & {r for r, _ in rest} else 'no'}")
        except UnsupportedError as e:
            print(f"page=unsupported:{e}")

        # Protocol 21 (CNT-FR-004): the edge count under a label — the one
        # runtime edge under mentored_by, the samples' relates_to, and an
        # unknown label Malformed.
        try:
            print(f"count_edges={c.count_edges('mentored_by')}")
            print(f"count_edges_relates_to={c.count_edges('relates_to')}")
            try:
                c.count_edges("no_such_label")
                print("count_edges_unknown=ok")
            except ServerError as e:
                print(f"count_edges_unknown={e.code.name}")
        except UnsupportedError as e:
            print(f"count_edges=unsupported:{e}")

        # Protocol 16 (TBL-FR-009): a one-table server lists itself; Use of
        # its own name is Ok, of another Malformed.
        try:
            names, primary = c.list_tables()
            print(f"tables={','.join(names)};{primary}")
            c.use_table(primary)
            print(f"use_self={c.table}")
            try:
                c.use_table("customer")
                print("use_unknown=ok")
            except ServerError as e:
                print(f"use_unknown={e.code.name}")
        except UnsupportedError as e:
            print(f"tables=unsupported:{e}")

        # Protocol 17 (DEL-FR-008): insert a throwaway entity, delete it,
        # and see the repeat answered False.
        try:
            gone = uuid.UUID(int=78)
            c.insert(gone, [("label", "Throwaway"), ("kind", "test"), ("mention_count", 0), ("aliases", [])])
            print(f"delete={'ok' if c.delete(gone) else 'notfound'}")
            print(f"delete_again={'ok' if c.delete(gone) else 'notfound'}")
            print(f"delete_get={'-' if c.get(gone) is None else 'present'}")
        except UnsupportedError as e:
            print(f"delete=unsupported:{e}")

        # Protocol 18 (CMP-FR-007): compact the entity table — the throwaway
        # entity's slot and the runtime edge logs are what it reclaims.
        try:
            r = c.compact()
            print(f"compact={r.records};{r.slots_reclaimed};{r.log_entries_folded};{r.edge_logs_folded}")
        except UnsupportedError as e:
            print(f"compact=unsupported:{e}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
