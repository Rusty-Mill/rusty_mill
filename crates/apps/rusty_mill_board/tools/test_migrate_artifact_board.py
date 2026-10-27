"""Tests for migrate_artifact_board.py: python3 -m unittest test_migrate_artifact_board (from tools/)."""

import json
import tempfile
import unittest
from pathlib import Path

import migrate_artifact_board as m


def doc(n, status="next", deps=(), **extra):
    return {
        "n": n, "title": f" card {n} ", "summary": "S", "prompt": "", "notes": "N",
        "status": status, "kind": "build", "track": "agents", "order": 10 * n,
        "deps": list(deps), **extra,
    }


class FakeTick:
    """Enough of rusty_tick for the migration: tasks, tags, lists, board documents."""

    def __init__(self):
        self.tasks, self.docs, self.lists, self.writes = {}, {}, [], 0
        self.knows_leaf = True

    def send(self, method, path, body, headers):
        status, reply = self.answer(method, path, body, headers)
        return status, json.loads(json.dumps(reply))  # a reply is a copy, as over HTTP

    def answer(self, method, path, body, headers):
        if method == "GET" and path == "/snapshot":
            return 200, {"tasks": list(self.tasks.values())}
        if method == "DELETE" and path.startswith("/tasks/"):
            # As rusty_tick answers it: `leaf` checked first where known, then the task,
            # then If-Match; an older one ignores `leaf` and trashes with the subtasks.
            tid, _, query = path.split("/")[2].partition("?")
            params = dict(p.split("=") for p in query.split("&") if p)
            assert "permanent" not in params, "rollback never purges"
            leaf = self.knows_leaf and params.get("leaf") == "true"
            if self.knows_leaf and params.get("leaf") not in (None, "true", "false"):
                return 400, {"error": "bad_request"}
            task = self.tasks.get(tid)
            if task is None:
                return 404, {"error": "not_found"}
            if "If-Match" in headers and headers["If-Match"] != task["etag"]:
                return 412, {"current": task}
            children = [t for t in self.tasks.values() if t.get("parentId") == tid]
            if leaf and children:
                return 409, {"error": "conflict"}
            self.writes += 1
            for t in [task] + children:
                t.update(deletedMs=t["deletedMs"] or 1, etag=str(int(t["etag"]) + 1))
            return 204, None
        if method == "GET" and path == "/lists":
            return 200, {"lists": self.lists}
        if method == "POST" and path == "/lists":
            self.lists.append({"id": "L", "name": body["name"]})
            return 201, self.lists[-1]
        if method == "POST" and path == "/tasks":
            self.writes += 1
            tid = f"t{len(self.tasks)}"
            self.tasks[tid] = {"parentId": None, **body, "id": tid, "status": "open", "etag": "1", "deletedMs": None}
            return 201, self.tasks[tid]
        if method == "GET" and path.startswith("/tasks/"):
            return 200, self.tasks[path.split("/")[2]]
        if method == "PATCH":
            self.writes += 1
            task = self.tasks[path.split("/")[2]]
            assert headers["If-Match"] == task["etag"], "a stale If-Match is refused"
            task.update(body, etag=str(int(task["etag"]) + 1))
            return 200, task
        if method == "GET" and path.startswith("/docs/board/"):
            found = self.docs.get(path.split("/")[3])
            return (200, found) if found else (404, None)
        if method == "PUT" and path.startswith("/docs/board/"):
            self.writes += 1
            tid = path.split("/")[3]
            task = self.tasks[tid]
            assert headers["If-Task-Match"] == task["etag"], "a stale If-Task-Match is refused"
            task["etag"] = str(int(task["etag"]) + 1)  # a board document moves its task
            self.docs[tid] = {"body": body, "etag": str(self.writes)}
            return 200, self.docs[tid]
        raise AssertionError(f"unexpected {method} {path}")


class Migration(unittest.TestCase):
    def test_a_card_maps_to_a_marked_task(self):
        card = m.card_of(doc(7, status="waiting", deps=[3], tags=["CI", "agent", "board:x"]))
        self.assertEqual(card.title, "card 7")
        self.assertEqual(
            card.tags,
            ("agent", "board:blocked", "migrated:mill-board", "kind:build", "track:agents", "ci"),
        )
        self.assertEqual(card.notes, "## Summary\n\nS\n\n## Notes\n\nN\n\nMigrated from Mill Board card #7")
        self.assertEqual((card.sort_order, card.deps, card.done), (70, (3,), False))

    def test_an_unknown_status_or_dependency_stops_it(self):
        with self.assertRaises(m.MigrationError):
            m.card_of(doc(1, status="someday"))
        with tempfile.TemporaryDirectory() as d:
            Path(d, "t1.json").write_text(json.dumps(doc(1, deps=[9])))
            with self.assertRaises(m.MigrationError):
                m.load_cards(Path(d))

    def test_a_second_run_writes_nothing(self):
        cards = [m.card_of(doc(1, status="done")), m.card_of(doc(2, deps=[1]))]
        fake = FakeTick()
        m.run(cards, m.Tick(fake.send), lambda _: None)
        first = fake.writes
        self.assertEqual(fake.docs["t1"]["body"]["deps"], ["t0"])
        self.assertEqual(fake.tasks["t0"]["status"], "done")
        m.run(cards, m.Tick(fake.send), lambda _: None)
        self.assertEqual(fake.writes, first)

    def test_only_the_exact_agent_tag_is_the_boards(self):
        card = m.card_of(doc(1, tags=["agent", "agents", "agentic", "agent-tools", "claim:x"]))
        self.assertEqual(card.tags[-3:], ("agents", "agentic", "agent-tools"))
        self.assertEqual(card.tags.count("agent"), 1)

    def test_a_card_rusty_tick_would_refuse_stops_the_run_before_any_write(self):
        too_much = [
            doc(1, title=" "),
            doc(1, title="t" * 501),
            doc(1, notes="n" * 100_001),
            doc(1, tags=["x" * 65]),
            doc(1, tags=[f"t{i}" for i in range(16)]),
            doc(1, order=2**63),
        ]
        for bad in too_much:
            with tempfile.TemporaryDirectory() as d:
                Path(d, "t0.json").write_text(json.dumps(doc(0)))
                Path(d, "t1.json").write_text(json.dumps(bad))
                with self.assertRaises(m.MigrationError):
                    m.load_cards(Path(d))
        m.card_of(doc(1, title="t" * 500, order=2**63 - 1))

    def test_a_marker_is_found_wherever_the_notes_put_it_and_a_missing_one_stops_it(self):
        fake = FakeTick()
        cards = [m.card_of(doc(1))]
        m.run(cards, m.Tick(fake.send), lambda _: None)
        fake.tasks["t0"]["notes"] += "\n\nAdded on the phone."
        m.run(cards, m.Tick(fake.send), lambda _: None)
        self.assertEqual(len(fake.tasks), 1, "not created again")
        for notes in ("marker removed", cards[0].notes + "\nMigrated from Mill Board card #2"):
            fake.tasks["t0"]["notes"] = notes
            with self.assertRaises(m.MigrationError):
                m.run(cards, m.Tick(fake.send), lambda _: None)

    def test_rollback_trashes_what_it_made_and_never_purges(self):
        fake = FakeTick()
        m.run([m.card_of(doc(1)), m.card_of(doc(2))], m.Tick(fake.send), lambda _: None)
        m.rollback(m.Tick(fake.send), lambda _: None)
        self.assertTrue(all(t["deletedMs"] is not None for t in fake.tasks.values()))
        self.assertEqual(len(fake.tasks), 2, "still there, in the trash")
        m.run([m.card_of(doc(1)), m.card_of(doc(2))], m.Tick(fake.send), lambda _: None)
        self.assertEqual(len(fake.tasks), 2, "a trashed card counts as migrated and is not made again")

    def test_rollback_refuses_while_a_migrated_task_has_a_subtask_it_did_not_make(self):
        fake = FakeTick()
        m.run([m.card_of(doc(1))], m.Tick(fake.send), lambda _: None)
        fake.send("POST", "/tasks", {"title": "mine", "notes": "", "tags": [], "parentId": "t0"}, {})
        before = fake.writes
        with self.assertRaises(m.MigrationError):
            m.rollback(m.Tick(fake.send), lambda _: None)
        self.assertEqual(fake.writes, before, "nothing trashed")
        self.assertIsNone(fake.tasks["t1"]["deletedMs"])

    def test_a_subtask_added_after_the_rollback_check_is_never_trashed(self):
        fake = FakeTick()
        m.run([m.card_of(doc(1))], m.Tick(fake.send), lambda _: None)

        def owner_adds_a_subtask_after_the_check(method, path, body, headers):
            if method == "DELETE" and "leaf=true" in path and "t1" not in fake.tasks:
                fake.send("POST", "/tasks", {"title": "mine", "notes": "", "tags": [], "parentId": "t0"}, {})
            return fake.send(method, path, body, headers)

        with self.assertRaises(m.MigrationError):
            m.rollback(m.Tick(owner_adds_a_subtask_after_the_check), lambda _: None)
        self.assertIsNone(fake.tasks["t1"]["deletedMs"])
        self.assertIsNone(fake.tasks["t0"]["deletedMs"], "the refused parent stays too")

    def test_rollback_refuses_a_rusty_tick_without_leaf_deletes(self):
        fake = FakeTick()
        m.run([m.card_of(doc(1))], m.Tick(fake.send), lambda _: None)
        fake.knows_leaf = False
        with self.assertRaises(m.MigrationError):
            m.rollback(m.Tick(fake.send), lambda _: None)
        self.assertIsNone(fake.tasks["t0"]["deletedMs"])

    def test_the_leaf_probe_changes_nothing_on_any_rusty_tick_even_when_its_id_exists(self):
        for knows_leaf in (True, False):
            fake = FakeTick()
            fake.knows_leaf = knows_leaf
            nil = "00000000-0000-0000-0000-000000000000"
            fake.tasks[nil] = {"id": nil, "parentId": None, "title": "the owner's", "notes": "",
                               "tags": [], "status": "open", "etag": "3", "deletedMs": None}
            fake.send("POST", "/tasks", {"title": "its subtask", "notes": "", "tags": [], "parentId": nil}, {})
            before = json.dumps(fake.tasks, sort_keys=True)
            self.assertEqual(m.knows_leaf_deletes(m.Tick(fake.send)), knows_leaf)
            self.assertEqual(json.dumps(fake.tasks, sort_keys=True), before, "no task or version changed")

    def test_dependencies_are_deduplicated_and_must_fit_one_board_document(self):
        self.assertEqual(m.card_of(doc(2, deps=[1, 1, 3, 1])).deps, (1, 3))
        m.card_of(doc(2, deps=list(range(1, 1600))))
        with self.assertRaises(m.MigrationError):
            m.card_of(doc(2, deps=list(range(1, 1700))))

    def test_two_tasks_for_one_card_stop_it(self):
        fake = FakeTick()
        card = m.card_of(doc(1))
        for _ in range(2):
            fake.send("POST", "/tasks", {"tags": list(card.tags), "notes": card.notes}, {})
        with self.assertRaises(m.MigrationError):
            m.run([card], m.Tick(fake.send), lambda _: None)


if __name__ == "__main__":
    unittest.main()
