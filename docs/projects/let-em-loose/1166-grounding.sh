#!/bin/bash
# #1166 grounding bundle (Nano's standing rule, 2026-11-18): before Nano acts on an escalated claim, the read-back audits
# every vote on it for grounding. This script collects the evidence and the mechanical checks; the judgement (which exact
# brief sentence a note relies on, or ungrounded) is Claude's and is written into the read-back from this file.
#
#   1166-grounding.sh DIR TASK OUTFILE [BIN]
#
# For each claim in the task's inbox it writes the claim, the brief, every scoring item with its note and what it cites
# (a claim, the question, itself, the brief), mechanical flags, wording the note shares with the claim but the brief lacks,
# and the scores: all votes (what the board counted), the latest vote per author, and the first vote per author.
# It posts nothing and acts on nothing. R (default: sudo -H -u orch-runner) prefixes every bbp call; R= runs it as the
# current user (tests).
set -euo pipefail
unset CDPATH

D=${1:?usage: 1166-grounding.sh DIR TASK OUTFILE [BIN]}
TASK=${2:?task}
OUT=${3:?outfile}
BIN=${4:-/opt/lel/current/bin}
R=${R-sudo -H -u orch-runner}
cd /

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# One read-only `bbp mcp` call as principal seed (the roster gives it read access to every message).
mcp() {
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"grounding","version":"0"}}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
    | $R env BBP_DIR="$D" BBP_TASK="$TASK" BBP_PRINCIPAL=seed BBP_SESSION=yes "$BIN/bbp" mcp \
    | jq -r 'select(.id==1).result.content[0].text'
}

$R "$BIN/bbp" inbox --dir "$D" --task "$TASK" --json yes > "$TMP/inbox.json"
if [ "$(jq 'length' "$TMP/inbox.json")" = 0 ]; then
  printf '# Grounding audit: %s\n\nNo escalated claim in the inbox, so there is nothing to act on.\n' "$TASK" > "$OUT"
  cat "$OUT"; exit 0
fi

BRIEF_ID=$(mcp task_card '{}' | jq -r '.Card.brief')
mcp get_artifact "{\"id\":$BRIEF_ID}" | jq -r '.content' > "$TMP/brief.txt"
[ -s "$TMP/brief.txt" ] || { echo "STOP: could not read the brief (art:$BRIEF_ID)" >&2; exit 1; }

# All messages, page by page.
AFTER=0; echo '[]' > "$TMP/messages.json"
for _ in $(seq 1 200); do
  PAGE=$(mcp read "{\"after\":$AFTER}")
  jq -s '.[0] + .[1].Read.messages' "$TMP/messages.json" <(echo "$PAGE") > "$TMP/m2.json" && mv "$TMP/m2.json" "$TMP/messages.json"
  [ "$(jq -r '.Read.more' <<<"$PAGE")" = true ] || break
  AFTER=$(jq -r '.Read.next' <<<"$PAGE")
done

python3 -I - "$TMP" "$TASK" "$BRIEF_ID" > "$OUT" <<'PY'
import json, re, sys
tmp, task, brief_id = sys.argv[1], sys.argv[2], int(sys.argv[3])
inbox = json.load(open(f"{tmp}/inbox.json"))
msgs = {m["id"]: m for m in json.load(open(f"{tmp}/messages.json"))}
brief = open(f"{tmp}/brief.txt").read()

def words(t): return set(re.findall(r"[a-z0-9_]{4,}", t.lower()))
bw = words(brief)

def body(m):
    b = m["draft"]["body"]
    return next(iter(b.values())) if b else {}

def refs_of(m):
    # What the vote cites is its `evidence` list (what the board shows as the item's refs); `refs` is
    # context orch adds (the brief, the claim voted on) and is not a citation.
    return list(m["draft"].get("evidence", []))

def describe(ref, claim_id, answers):
    if "Art" in ref:
        n = ref["Art"]["id"]
        return f"art:{n}" + (" (the brief)" if n == brief_id else ""), ["cites-the-brief"] if n == brief_id else ["cites-an-artifact"]
    n = ref["Msg"]
    m = msgs.get(n)
    if m is None:
        return f"msg:{n} (not found)", ["cites-a-missing-message"]
    kind = m["draft"]["kind"]
    flags = []
    label = f"msg:{n} ({kind} by {m['from']})"
    if n == claim_id: flags.append("SELF-CITATION")
    if answers is not None and n == answers: flags.append("cites-the-question")
    if kind == "Claim" and n != claim_id and n != answers: flags.append("CITES-A-CLAIM-AS-EVIDENCE")
    if kind == "Claim" and n != claim_id: flags.append("a claim is not a run or a test report")
    return label, flags

print(f"# Grounding audit bundle: {task}\n")
print("Evidence for the audit, not the audit. Nothing here is a decision. For each vote Claude must quote the exact brief")
print("sentence the note relies on, or mark the vote ungrounded, and then give the grounded-only scores. Do not act on an")
print("escalated claim before that audit has been read.\n")
print("## The brief (art:%d), numbered\n" % brief_id)
for i, line in enumerate(brief.splitlines(), 1):
    print(f"{i:>3}  {line}")
print()
for e in inbox:
    c = e["claim"]; cid = c["id"]
    cm = msgs.get(cid)
    answers = body(cm).get("answers") if cm else None
    print(f"## Escalated claim msg:{cid} ({e['reason']})\n")
    print(f"- text: {c['text']}")
    print(f"- stakes {e['stakes']}, reversible {str(e['reversible']).lower()}, board score {c['score']} against accept line {c['accept_at']}, answers {answers}")
    ctext = words(c["text"])
    items = [i for i in c["evidence"] if i["scoring"]]
    print(f"- scoring votes: {len(items)}\n")
    print("| msg | author | dir | kind | str | delta | cites | flags | wording shared with the claim but absent from the brief |")
    print("|---|---|---|---|---|---|---|---|---|")
    for it in sorted(items, key=lambda i: i["msg"]):
        m = msgs.get(it["msg"])
        note = body(m).get("note", "") if m else ""
        cites, flags = [], []
        for r in (refs_of(m) if m else []):
            d, f = describe(r, cid, answers)
            cites.append(d); flags += f
        shared = sorted((words(note) & ctext) - bw)[:12]
        fl = ", ".join(dict.fromkeys(flags)) or "-"
        print(f"| {it['msg']} | {it['author']} | {it['direction']} | {it['kind']} | {it['strength']} | {it['delta']:+d} | {'; '.join(cites) or '-'} | {fl} | {', '.join(shared) or '-'} |")
    print()
    for it in sorted(items, key=lambda i: i["msg"]):
        m = msgs.get(it["msg"])
        note = body(m).get("note", "") if m else ""
        print(f"- msg:{it['msg']} ({it['author']}, {it['direction']}): \"{note}\"")
    print()
    total = sum(i["delta"] for i in items)
    def per_author(pick):
        by = {}
        for i in sorted(items, key=lambda i: i["msg"]):
            if pick == "latest" or i["author"] not in by:
                by[i["author"]] = i
        return by
    latest, first = per_author("latest"), per_author("first")
    sup = lambda xs: len({i["author"] for i in xs if i["direction"] == "Support"})
    print("### Scores counting every vote the board counted\n")
    print(f"- all votes, repeats per author (the board): {total:+d}  (board says {c['score']:+d})")
    lat = ", ".join("%s: msg:%d" % (a, i["msg"]) for a, i in latest.items())
    fst = ", ".join("%s: msg:%d" % (a, i["msg"]) for a, i in first.items())
    print("- latest vote per author: %+d  (%s)" % (sum(i["delta"] for i in latest.values()), lat))
    print("- first vote per author: %+d  (%s)" % (sum(i["delta"] for i in first.values()), fst))
    print(f"- distinct support authors: {sup(items)}\n")
    print("### Audit to complete (Claude), before Nano acts\n")
    print("For each vote above, quote the exact numbered brief sentence the note relies on, or write UNGROUNDED. A vote that")
    print("attributes to the brief wording it does not contain (see the shared-wording column) is ungrounded; a vote that cites a")
    print("claim as a run is ungrounded. Then give: the score of grounded votes only (repeats allowed, and one vote per author),")
    print("the distinct grounded support authors against min_authors 2, and any way the claim itself departs from the brief.\n")
PY
cat "$OUT"
