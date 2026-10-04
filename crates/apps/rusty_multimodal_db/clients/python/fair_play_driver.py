"""FPL-FR-007 (ADR-0137): the live driver tests/server_fair_play_integration.rs runs.

Usage: python3 fair_play_driver.py HOST:PORT CARD_ID

Connects to a ``fair_play_server``-shaped server (``card`` primary,
``person`` and ``card_default`` beside it), reaches ``person`` and ``card``
through ``use_table``, and prints ``key=value`` lines the Rust test
asserts on. Nothing here is a library API.
"""

import sys
import uuid

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from rusty_multimodal_db import Client  # noqa: E402


def main() -> int:
    host, port = sys.argv[1].rsplit(":", 1)
    card_id = uuid.UUID(sys.argv[2])
    with Client.connect(host, int(port)) as c:
        print(f"negotiated={c.server_protocol_version}")
        names, primary = c.list_tables()
        print(f"tables={','.join(names)};{primary}")

        c.use_table("person")
        print(f"person_table={c.table}")
        print("person_fields=" + ",".join(f.name for f in c.schema.fields))
        ada = c.filter_eq("name", "Ada")
        print(f"person_filter_eq_ids={len(ada)}")
        print(f"person_filter_eq_first={ada[0] if ada else '-'}")
        record = dict(c.get(ada[0])) if ada else {}
        print(f"person_name={record.get('name', '-')}")
        print(f"person_player={record.get('player', '-')}")

        c.use_table("card")
        print(f"card_table={c.table}")
        print("card_fields=" + ",".join(f.name for f in c.schema.fields))
        card = c.get(card_id)
        print(f"card_get_fields={len(card)}")
        fields = dict(card)
        print(f"card_name={fields['name']}")
        print(f"card_number={fields['number'] if fields['number'] is not None else 'null'}")
        print(f"card_owner={fields['owner_id'] if fields['owner_id'] is not None else 'null'}")
        print(f"card_parent={fields['parent_card_id'] if fields['parent_card_id'] is not None else 'null'}")
        print("card_standards=" + "|".join(fields["minimum_standard_of_care"]))
        children = c.children(card_id)
        print(f"card_children={len(children)}")
        print("card_children_ids=" + ",".join(sorted(str(k) for k in children)))
        print(f"card_parent_of_child={c.parent(children[0]) if children else '-'}")
        print(f"card_get_missing={'none' if c.get(uuid.UUID(int=99)) is None else 'found'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
