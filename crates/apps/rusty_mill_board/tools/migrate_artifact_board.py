#!/usr/bin/env python3
"""Move the Mill Board artifact's cards into rusty_tick, once (ADR-0012, part 7).

    migrate_artifact_board.py EXPORT_DIR                          # dry run, offline
    RUSTY_TICK_TOKEN=... migrate_artifact_board.py EXPORT_DIR --tick-url URL          # dry run against rusty_tick
    RUSTY_TICK_TOKEN=... migrate_artifact_board.py EXPORT_DIR --tick-url URL --run    # write
    RUSTY_TICK_TOKEN=... migrate_artifact_board.py --tick-url URL --rollback --yes    # trash what it wrote

EXPORT_DIR holds the artifact's `tasks` collection, one JSON document per card
(`n`, `title`, `summary`, `prompt`, `notes`, `status`, `kind`, `track`, `order`,
`deps`, optional `tags`). The artifact is only read, never written.

Every card is checked against rusty_tick's limits before anything is written.
Every task it creates carries the tag `migrated:mill-board` and a notes line
`Migrated from Mill Board card #n`, in the create request itself. Each run
rebuilds the card-to-task map from those markers (trash included; a tagged task
without exactly one marker stops it), creates only the cards missing from it,
then writes every task's `board` document (its dependencies) and closes the done
ones; both are the same write however often they run. So a crash anywhere is
finished by running again.

Rollback moves every marked task to the trash with `DELETE ?leaf=true`, which
rusty_tick refuses (409) in the same step if the task has a subtask, so nothing
it did not create goes with it. It checks for subtasks first, so a refusal
normally writes nothing, and refuses a rusty_tick too old to know `leaf`. The
trashed tasks keep their markers, so a later run does not create them again:
restore them in rusty_tick, or delete them from its trash, before migrating again.

Standard library only.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Optional

MARKER_TAG = "migrated:mill-board"
MARKER_LINE = "Migrated from Mill Board card #"
LIST_NAME = "rusty_mill"
COLUMNS = {
    "backlog": "backlog",
    "next": "ready",
    "doing": "building",
    "waiting": "blocked",
    "done": "done",
}
MARKER = re.compile(rf"^{re.escape(MARKER_LINE)}(\d+)$", re.MULTILINE)
# The board's own tags (ADR-0012): `agent` exactly, and these prefixes.
RESERVED_PREFIXES = ("board:", "claim:", "migrated:")
# rusty_tick's limits (crates/apps/rusty_tick/src/service.rs).
MAX_TITLE_CHARS = 500
MAX_NOTES_CHARS = 100_000
MAX_TAGS = 20
MAX_TAG_CHARS = 64
MAX_DOC_BYTES = 64 * 1024
I64 = range(-(2**63), 2**63)

Send = Callable[[str, str, Optional[dict], dict], "tuple[int, Any]"]


class MigrationError(Exception):
    """A reason to stop before (or instead of) writing."""


@dataclass(frozen=True)
class Card:
    n: int
    title: str
    notes: str
    tags: tuple[str, ...]
    sort_order: int
    deps: tuple[int, ...]
    done: bool


def notes_of(doc: dict) -> str:
    sections = [
        f"## {name}\n\n{doc.get(key, '').strip()}"
        for name, key in (("Summary", "summary"), ("Prompt", "prompt"), ("Notes", "notes"))
        if doc.get(key, "").strip()
    ]
    return "\n\n".join(sections + [f"{MARKER_LINE}{doc['n']}"])


def is_reserved(tag: str) -> bool:
    return tag == "agent" or tag.startswith(RESERVED_PREFIXES)


def card_of(doc: dict) -> Card:
    """The task a card becomes, refused if rusty_tick would refuse any part of it."""
    status = doc.get("status")
    if status not in COLUMNS:
        raise MigrationError(f"card #{doc.get('n')}: unknown status {status!r}")
    own = [t.strip().lower() for t in doc.get("tags") or [] if t.strip()]
    own = [t for t in own if not is_reserved(t)]
    tags = ["agent", f"board:{COLUMNS[status]}", MARKER_TAG]
    tags += [f"kind:{doc['kind']}", f"track:{doc['track']}"] + own
    card = Card(
        n=int(doc["n"]),
        title=doc["title"].strip(),
        notes=notes_of(doc),
        tags=tuple(dict.fromkeys(tags)),
        sort_order=int(doc.get("order") or 0),
        deps=tuple(dict.fromkeys(int(d) for d in doc.get("deps") or [])),
        done=status == "done",
    )
    problem = limit_problem(card)
    if problem:
        raise MigrationError(f"card #{card.n}: {problem}")
    return card


def limit_problem(card: Card) -> Optional[str]:
    if not card.title or len(card.title) > MAX_TITLE_CHARS:
        return f"title must be 1 to {MAX_TITLE_CHARS} characters"
    if len(card.notes) > MAX_NOTES_CHARS:
        return f"notes are longer than {MAX_NOTES_CHARS} characters"
    if len(card.tags) > MAX_TAGS:
        return f"{len(card.tags)} tags, rusty_tick keeps {MAX_TAGS}"
    long_tags = [t for t in card.tags if len(t) > MAX_TAG_CHARS]
    if long_tags:
        return f"tags longer than {MAX_TAG_CHARS} characters: {long_tags}"
    if card.sort_order not in I64:
        return f"order {card.sort_order} is out of range"
    board_doc = json.dumps({"v": 1, "deps": ["0" * 36] * len(card.deps)})
    if len(board_doc.encode()) > MAX_DOC_BYTES:
        return f"{len(card.deps)} dependencies do not fit a {MAX_DOC_BYTES}-byte board document"
    return None


def load_cards(export: Path) -> list[Card]:
    docs = [json.loads(p.read_text(encoding="utf-8")) for p in sorted(export.glob("*.json"))]
    cards = sorted((card_of(d) for d in docs), key=lambda c: c.n)
    numbers = [c.n for c in cards]
    if len(set(numbers)) != len(numbers):
        raise MigrationError("two cards share a number")
    unknown = sorted({d for c in cards for d in c.deps} - set(numbers))
    if unknown:
        raise MigrationError(f"dependencies on cards not in the export: {unknown}")
    return cards


class Tick:
    """The few rusty_tick calls the migration makes. `send` is swappable for tests."""

    def __init__(self, send: Send):
        self.send = send

    def call(self, method: str, path: str, body: Optional[dict] = None, ok: int = 200, **headers: str) -> Any:
        status, reply = self.send(method, path, body, headers)
        if status != ok:
            raise MigrationError(f"rusty_tick answered {status} to {method} {path}: {reply}")
        return reply

    def migrated(self, tasks: Optional[list[dict]] = None) -> dict[int, dict]:
        """Card number to task, from the markers, trash included."""
        by_number: dict[int, dict] = {}
        for task in tasks if tasks is not None else self.snapshot_tasks():
            if MARKER_TAG not in task["tags"]:
                continue
            n = card_number(task)
            if n in by_number:
                raise MigrationError(f"two migrated tasks for card #{n}: stop and resolve by hand")
            by_number[n] = task
        return by_number

    def snapshot_tasks(self) -> list[dict]:
        """Every task, trash and completed included."""
        return self.call("GET", "/snapshot")["tasks"]

    def list_id(self) -> str:
        for row in self.call("GET", "/lists")["lists"]:
            if row["name"] == LIST_NAME:
                return row["id"]
        return self.call("POST", "/lists", {"name": LIST_NAME}, ok=201)["id"]

    def create(self, card: Card, list_id: str) -> dict:
        body = {
            "listId": list_id,
            "title": card.title,
            "notes": card.notes,
            "tags": list(card.tags),
            "sortOrder": card.sort_order,
        }
        return self.call("POST", "/tasks", body, ok=201)

    def finish(self, card: Card, task: dict, deps: list[str]) -> list[str]:
        """Write the board document and close a done card; return what changed."""
        changed = []
        status, doc = self.send("GET", f"/docs/board/{task['id']}", None, {})
        if status not in (200, 404):
            raise MigrationError(f"rusty_tick answered {status} reading card #{card.n}'s board document")
        body = {"v": 1, "deps": deps}
        current = doc.get("body") if status == 200 else None
        if current is None or current.get("deps") != deps:
            # Fenced on the task alone: every board-document write moves the task's
            # version, so it also names the document. The write moves it, so re-read.
            merged = {**(current or {}), **body}
            self.call("PUT", f"/docs/board/{task['id']}", merged, **{"If-Task-Match": task["etag"]})
            task = self.call("GET", f"/tasks/{task['id']}")
            changed.append("deps")
        if card.done and task["status"] == "open":
            self.call("PATCH", f"/tasks/{task['id']}", {"status": "done"}, **{"If-Match": task["etag"]})
            changed.append("closed")
        return changed


def card_number(task: dict) -> int:
    """The card a marked task came from: its one marker line, wherever the notes put it."""
    numbers = {int(n) for n in MARKER.findall(task.get("notes") or "")}
    if len(numbers) != 1:
        raise MigrationError(
            f"task {task['id']} is tagged {MARKER_TAG} but names {len(numbers)} cards in its notes: "
            "stop and resolve by hand"
        )
    return numbers.pop()


def http_send(base: str, token: str) -> Send:
    def send(method: str, path: str, body: Optional[dict], headers: dict) -> "tuple[int, Any]":
        request = urllib.request.Request(
            f"{base.rstrip('/')}/api/v1{path}",
            method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json", **headers},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as reply:
                status, raw = reply.status, reply.read()
        except urllib.error.HTTPError as e:
            status, raw = e.code, e.read()
        return status, json.loads(raw) if raw.strip() else None

    return send


def plan(cards: list[Card], migrated: dict[int, dict]) -> list[str]:
    lines = []
    for card in cards:
        task = migrated.get(card.n)
        state = "exists" if task else "create"
        deps = ", ".join(f"#{d}" for d in card.deps) or "none"
        lines.append(f"#{card.n:>3} {state:<6} {card.title} | {' '.join(card.tags)} | deps {deps}")
    return lines


def run(cards: list[Card], tick: Tick, out: Callable[[str], None]) -> None:
    migrated = tick.migrated()
    list_id = tick.list_id()
    for card in cards:
        if card.n not in migrated:
            migrated[card.n] = tick.create(card, list_id)
            out(f"#{card.n} created {migrated[card.n]['id']}")
    for card in cards:
        task = tick.call("GET", f"/tasks/{migrated[card.n]['id']}")
        if task.get("deletedMs") is not None:
            out(f"#{card.n} is in the trash; left as it is")
            continue
        deps = [migrated[d]["id"] for d in card.deps]
        changed = tick.finish(card, task, deps)
        if changed:
            out(f"#{card.n} {' and '.join(changed)}")


def foreign_subtasks(migrated: dict[int, dict], tasks: list[dict]) -> list[str]:
    """Subtasks of migrated tasks that the migration did not create."""
    ours = {t["id"] for t in migrated.values()}
    return sorted(
        f"{t['id']} ({t['title']!r})"
        for t in tasks
        if t.get("parentId") in ours and t["id"] not in ours
    )


def knows_leaf_deletes(tick: Tick) -> bool:
    """Whether rusty_tick knows `leaf`, asked without changing anything on any version.

    One that knows it refuses an unknown value (400) before looking at the task. One
    that does not answers 404, or, if a task has this id, 412: the `If-Match` names no
    version a task can have, so even then nothing is trashed.
    """
    status, _ = tick.send(
        "DELETE",
        "/tasks/00000000-0000-0000-0000-000000000000?leaf=probe",
        None,
        {"If-Match": "never-a-version"},
    )
    return status == 400


def rollback(tick: Tick, out: Callable[[str], None]) -> None:
    if not knows_leaf_deletes(tick):
        raise MigrationError("this rusty_tick cannot trash a task without its subtasks (DELETE ?leaf=true); update it first")
    tasks = tick.snapshot_tasks()
    migrated = tick.migrated(tasks)
    foreign = foreign_subtasks(migrated, tasks)
    if foreign:
        raise MigrationError(f"migrated tasks have subtasks it did not create, nothing trashed: {foreign}")
    for n, task in sorted(migrated.items()):
        if task.get("deletedMs") is not None:
            continue
        # Fenced, so a task changed since it was read is reported, not trashed; `leaf`
        # makes rusty_tick refuse one that has gained a subtask since the check above.
        tick.call("DELETE", f"/tasks/{task['id']}?leaf=true", ok=204, **{"If-Match": task["etag"]})
        out(f"#{n} trashed {task['id']}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("export", nargs="?", type=Path, help="directory of the artifact's card documents")
    parser.add_argument("--tick-url", help="rusty_tick root, e.g. http://127.0.0.1:8788")
    parser.add_argument("--run", action="store_true", help="write (default: dry run)")
    parser.add_argument("--rollback", action="store_true", help="move every migrated task to the trash")
    parser.add_argument("--yes", action="store_true", help="confirm --rollback")
    args = parser.parse_args(argv)

    tick = None
    if args.tick_url:
        token = os.environ.get("RUSTY_TICK_TOKEN", "").strip()
        if not token:
            parser.error("RUSTY_TICK_TOKEN is not set")
        tick = Tick(http_send(args.tick_url, token))
    try:
        if args.rollback:
            if tick is None or not args.yes:
                parser.error("--rollback needs --tick-url and --yes")
            rollback(tick, print)
            return 0
        if args.export is None:
            parser.error("EXPORT_DIR is required")
        cards = load_cards(args.export)
        if not args.run:
            migrated = tick.migrated() if tick else {}
            print("\n".join(plan(cards, migrated)))
            print(f"{len(cards)} cards, {sum(c.n not in migrated for c in cards)} to create. Dry run: nothing written.")
            return 0
        if tick is None:
            parser.error("--run needs --tick-url")
        run(cards, tick, print)
        return 0
    except MigrationError as e:
        print(f"stopped: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
