#!/usr/bin/env python3
"""Capture a sanitized ballchasing.com fixture for the canonical cross-check.

This is **manual data tooling**, gated on the ballchasing API key and ToS — it
is *never* run in CI and is not part of the Rust build (the comparator test runs
entirely against the committed fixtures, offline). Run it only to capture or
refresh the `*.ballchasing.json` fixtures that
`scoring/tests/ballchasing_contract.rs` compares our canonical decode against.

What it does
------------
1. Pings the API to verify the key.
2. Uploads a `.replay` as **visibility=private** (dedupes server-side: a 409 just
   returns the existing replay id — we never re-upload someone else's public one).
3. Polls the replay until ballchasing finishes its independent decode
   (``status == "ok"``), with backoff.
4. Distills + **sanitizes** the response into the small schema the Rust
   comparator reads (`replay_scoring::contract::BallchasingReplay`): the uploader
   identity is dropped entirely and every per-player platform id value is nulled,
   so nothing player/account-identifying lands in the committed fixture.

Usage
-----
    export BALLCHASING_API_KEY=<key>          # read from env ONLY; never logged
    python scripts/ballchasing_fetch.py assets/replays/42f2.replay \
        --out scoring/tests/fixtures/42f2.ballchasing.json

The key is read from `BALLCHASING_API_KEY` only and is never printed. Upload is
always private.
"""
import argparse
import json
import mimetypes
import os
import sys
import time
import urllib.error
import urllib.request
import uuid

API = "https://ballchasing.com"
PING = API + "/api/"
UPLOAD = API + "/api/v1/upload?visibility=private"
REPLAY = API + "/api/replays/"  # GET {id}; matches assets/corpus/refresh_ballchasing_stats.py

# blue is team 0, orange is team 1 (matches Engine.PlayerReplicationInfo:Team and
# the rest of the repo's COLOR_TEAM convention).
TEAM_CORE_FIELDS = ("goals", "shots", "saves", "assists")
PLAYER_CORE_FIELDS = ("goals", "assists", "saves", "shots", "score", "mvp")


def _key() -> str:
    key = os.environ.get("BALLCHASING_API_KEY")
    if not key:
        sys.exit("error: BALLCHASING_API_KEY not set (read from env only; never commit it)")
    return key


def _req(url, key, *, data=None, headers=None, method=None):
    h = {"Authorization": key}
    if headers:
        h.update(headers)
    return urllib.request.Request(url, data=data, headers=h, method=method)


def ping(key):
    """Verify the key; raises on non-200. Prints only non-identifying fields."""
    with urllib.request.urlopen(_req(PING, key), timeout=30) as resp:
        doc = json.load(resp)
    # `name`/`steam_id` identify the uploader account — do not print them.
    print(f"ping ok: type={doc.get('type')} (authenticated)", file=sys.stderr)


def _multipart(field, filename, blob):
    """Encode a single-file multipart/form-data body (stdlib-only, no requests)."""
    boundary = "----rls" + uuid.uuid4().hex
    ctype = mimetypes.guess_type(filename)[0] or "application/octet-stream"
    pre = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="{field}"; filename="{filename}"\r\n'
        f"Content-Type: {ctype}\r\n\r\n"
    ).encode()
    body = pre + blob + f"\r\n--{boundary}--\r\n".encode()
    return body, f"multipart/form-data; boundary={boundary}"


def upload(path, key):
    """Upload a replay privately; return its ballchasing id (201 new or 409 dup)."""
    with open(path, "rb") as fh:
        blob = fh.read()
    body, ctype = _multipart("file", os.path.basename(path), blob)
    req = _req(UPLOAD, key, data=body, headers={"Content-Type": ctype}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            doc = json.load(resp)
            print(f"uploaded (private): id={doc['id']}", file=sys.stderr)
            return doc["id"]
    except urllib.error.HTTPError as exc:
        # 409 = ballchasing already has this exact replay; the body carries its id.
        if exc.code == 409:
            doc = json.load(exc)
            print(f"already on ballchasing (dup): id={doc['id']}", file=sys.stderr)
            return doc["id"]
        raise


def poll(replay_id, key, *, tries=30):
    """GET the replay until its independent decode is done (status == ok)."""
    delay = 2.0
    for i in range(tries):
        with urllib.request.urlopen(_req(REPLAY + replay_id, key), timeout=30) as resp:
            doc = json.load(resp)
        status = doc.get("status")
        if status == "ok":
            print(f"decode ready after {i + 1} poll(s)", file=sys.stderr)
            return doc
        if status == "failed":
            sys.exit(f"error: ballchasing failed to parse replay {replay_id}")
        print(f"  status={status}; waiting {delay:.0f}s", file=sys.stderr)
        time.sleep(delay)
        delay = min(delay * 1.6, 20.0)
    sys.exit(f"error: replay {replay_id} not ready after {tries} polls")


def _team(side):
    """Distill one team side, keeping only the cross-check fields."""
    core = side.get("stats", {}).get("core", {})
    return {
        "stats": {"core": {k: core.get(k) for k in TEAM_CORE_FIELDS}},
        "players": [_player(p) for p in side.get("players", [])],
    }


def _player(p):
    """Distill + sanitize one player: keep name + core stats, null the id value."""
    core = p.get("stats", {}).get("core", {})
    pid = p.get("id") or {}
    return {
        "name": p.get("name"),
        # platform kept (non-identifying); the numeric/steam id is nulled out.
        "id": {"platform": pid.get("platform"), "id": None},
        "stats": {"core": {k: core.get(k) for k in PLAYER_CORE_FIELDS}},
    }


# The five stat groups carried verbatim in the --full-stats fixture (for the
# bc-validate exact-match validator). These are match facts, not identity.
FULL_STAT_GROUPS = ("core", "boost", "movement", "positioning", "demo")


def _player_full(p):
    """Keep a player's *whole* stat block (all five groups) for bc-validate.

    Still drops identity: only `name` + `stats` survive, and the platform id is
    nulled. Stat numbers pass through unchanged so they can be diffed field by
    field against the clone's output.
    """
    stats = p.get("stats", {})
    pid = p.get("id") or {}
    return {
        "name": p.get("name"),
        "id": {"platform": pid.get("platform"), "id": None},
        "stats": {g: stats.get(g, {}) for g in FULL_STAT_GROUPS},
    }


def _team_full(side):
    """A team side with every player's full stat block (and the team stats)."""
    return {
        "stats": {g: side.get("stats", {}).get(g, {}) for g in FULL_STAT_GROUPS},
        "players": [_player_full(p) for p in side.get("players", [])],
    }


def sanitize(doc, full=False):
    """Reduce the raw replay JSON to a committed-fixture schema.

    Drops the `uploader` block entirely (our account identity) and nulls every
    per-player platform id, so the fixture carries only match facts. With
    `full=True` it keeps the whole per-player stat block (for `bc-validate`);
    otherwise only the core cross-check fields (for the contract test).
    """
    team = _team_full if full else _team
    return {
        "id": doc.get("id"),
        "status": doc.get("status"),
        "map_code": doc.get("map_code"),
        "map_name": doc.get("map_name"),
        "team_size": doc.get("team_size"),
        "duration": doc.get("duration"),
        "blue": team(doc.get("blue", {})),
        "orange": team(doc.get("orange", {})),
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("replay", help="path to a .replay file")
    ap.add_argument("--out", required=True, help="where to write the sanitized fixture JSON")
    ap.add_argument("--id", help="skip upload; fetch an existing ballchasing replay id")
    ap.add_argument(
        "--full-stats",
        action="store_true",
        help="keep every per-player stat group (for bc-validate), not just core",
    )
    args = ap.parse_args()

    key = _key()
    ping(key)
    replay_id = args.id or upload(args.replay, key)
    doc = poll(replay_id, key)
    fixture = sanitize(doc, full=args.full_stats)

    with open(args.out, "w", encoding="utf-8") as fh:
        json.dump(fixture, fh, indent=2, sort_keys=True, ensure_ascii=False)
        fh.write("\n")
    n = len(fixture["blue"]["players"]) + len(fixture["orange"]["players"])
    mode = "full per-player stats" if args.full_stats else "core fields only"
    print(f"wrote {args.out}: {n} players, map={fixture['map_code']} "
          f"team_size={fixture['team_size']} ({mode}; uploader + player ids stripped)",
          file=sys.stderr)


if __name__ == "__main__":
    main()
