#!/bin/bash
# #1166 grounding bundle (Nano's standing rule D155, extended by his Q59 answer on 2026-11-18): the read-back audits every
# vote on every claim that is ESCALATED (before Nano acts on it) and on every claim the board DROPPED (because a drop on
# ungrounded votes decides whether a question has a survivor). This script collects the evidence and the mechanical checks;
# the judgement (which exact brief sentence a note relies on, or ungrounded) is Claude's and is written into the read-back.
#
#   1166-grounding.sh DIR TASK OUTFILE [BIN]
#
# For each such claim it writes the claim, the brief (numbered), every scoring vote with its note and what it cites (a claim,
# another vote, the question, itself, the brief), mechanical flags, wording the note shares with the claim but the brief lacks,
# the scores (all votes, latest vote per author, first vote per author), a mechanical hint (the score of the votes that carry no
# flag), and the board's rule that fired. It posts nothing and acts on nothing. R (default: sudo -H -u orch-runner) prefixes
# every bbp call; R= runs it as the current user (tests).
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

# The board's own history of every claim.
mkdir -p "$TMP/why"
for id in $(jq -r '.[] | select(.draft.kind=="Claim") | .id' "$TMP/messages.json"); do
  $R "$BIN/bbp" why --dir "$D" --task "$TASK" --claim "$id" > "$TMP/why/$id.json"
done

python3 -I - "$TMP" "$TASK" "$BRIEF_ID" > "$OUT" <<'PY'
import json, re, sys, glob, os
tmp, task, brief_id = sys.argv[1], sys.argv[2], int(sys.argv[3])
inbox = {e["claim"]["id"]: e for e in json.load(open(f"{tmp}/inbox.json"))}
msgs = {m["id"]: m for m in json.load(open(f"{tmp}/messages.json"))}
why = {int(os.path.basename(p)[:-5]): json.load(open(p)) for p in glob.glob(f"{tmp}/why/*.json")}
brief = open(f"{tmp}/brief.txt").read()

def words(t): return set(re.findall(r"[a-z0-9_]{4,}", t.lower()))
bw = words(brief)

def body(m):
    b = m["draft"]["body"]
    return next(iter(b.values())) if b else {}

def describe(ref, claim_id, answers, author):
    if "Art" in ref:
        n = ref["Art"]["id"]
        return (f"art:{n}" + (" (the brief)" if n == brief_id else ""), ["cites-the-brief"] if n == brief_id else ["cites-an-artifact"])
    n = ref["Msg"]
    m = msgs.get(n)
    if m is None:
        return f"msg:{n} (not found)", ["CITES-A-MISSING-MESSAGE"]
    kind = m["draft"]["kind"]
    flags = []
    label = f"msg:{n} ({kind} by {m['from']})"
    if n == claim_id: flags.append("SELF-CITATION")
    elif answers is not None and n == answers: flags.append("cites-the-question")
    elif kind == "Claim": flags += ["CITES-A-CLAIM-AS-EVIDENCE", "a claim is not a run or a test report"]
    elif kind == "Evidence": flags.append("CITES-ANOTHER-VOTE")
    return label, flags

HARD = {"SELF-CITATION", "CITES-A-CLAIM-AS-EVIDENCE", "CITES-ANOTHER-VOTE", "CITES-A-MISSING-MESSAGE"}

audited = []
for cid in sorted(why):
    steps = why[cid]["steps"]
    status = steps[-1]["status"]
    if status in ("Escalated", "Dropped"):
        audited.append((cid, status))

print(f"# Grounding audit bundle: {task}\n")
print("Evidence for the audit, not the audit. Nothing here is a decision. For each vote Claude must quote the exact brief")
print("sentence the note relies on, or mark the vote ungrounded, and then give the grounded-only scores. Do not act on an")
print("escalated claim before that audit has been read. A DROPPED claim is audited too: a drop on ungrounded votes decides")
print("whether the question has a survivor.\n")
print("## The brief (art:%d), numbered\n" % brief_id)
for i, line in enumerate(brief.splitlines(), 1):
    print(f"{i:>3}  {line}")
print()
if not audited:
    print("## No escalated or dropped claim\n\nNothing to audit and nothing to act on.")
    sys.exit(0)

summary = []
sections = []
for cid, status in audited:
    w = why[cid]
    cm = msgs.get(cid)
    answers = body(cm).get("answers") if cm else None
    steps = w["steps"]
    rule = None
    for s in steps:
        ch = s["change"]
        if isinstance(ch, dict) and "Status" in ch:
            rule = ch["Status"]["rule"]
    # The board scores only the votes of the claim's CURRENT epoch (a `reopen` starts a new one). A vote counts there if it
    # scored when posted in that epoch, or if the reopen that began the epoch woke it: the reopen's Status step lists those
    # votes in `woke` (the reopening evidence, or every dormant vote a human reopen woke) while the vote's own step keeps
    # its old epoch and `scoring` as appended. A vote that scored in an earlier epoch is listed apart and is in no score.
    epoch = steps[-1]["epoch"]
    woken = set()
    for s in steps:
        ch = s["change"]
        if isinstance(ch, dict) and "Status" in ch and s["epoch"] == epoch:
            woken |= set(ch["Status"].get("woke", []))
    items, earlier = [], []
    dormant = 0
    for s in steps:
        ch = s["change"]
        if isinstance(ch, dict) and "Evidence" in ch:
            ev = dict(ch["Evidence"], epoch=s["epoch"])
            if ev["msg"] in woken:
                ev["woke"] = True
                items.append(ev)
            elif not ev["scoring"]:
                dormant += 1
            elif s["epoch"] == epoch:
                items.append(ev)
            else:
                earlier.append(ev)
    flagged_rows, hint_items = [], []
    out = []
    author = w["author"]
    kindword = "ESCALATED" if status == "Escalated" else "DROPPED"
    why_line = ""
    if status == "Escalated" and cid in inbox:
        why_line = f", escalated for {inbox[cid]['reason']}, accept line {inbox[cid]['claim']['accept_at']}"
    out.append(f"## {kindword} claim msg:{cid}{why_line}\n")
    out.append(f"- text: {w['body']['text']}")
    out.append(f"- author {author}, answers {answers}, stakes {w['body']['stakes']}, reversible {str(w['body']['reversible']).lower()}")
    out.append(f"- the board's rule that fired: {json.dumps(rule)}")
    nwoke = sum(1 for i in items if i.get("woke"))
    out.append(f"- current epoch {epoch}; scoring votes in it: {len(items)} (of which woken by the reopen: {nwoke}); votes from earlier epochs (not scored now): {len(earlier)}; dormant votes still not scored: {dormant}\n")
    out.append("| msg | author | dir | kind | str | delta | cites | flags | wording shared with the claim but absent from the brief |")
    out.append("|---|---|---|---|---|---|---|---|---|")
    ctext = words(w["body"]["text"])
    notes = []
    for it in sorted(items, key=lambda i: i["msg"]):
        m = msgs.get(it["msg"])
        b = body(m) if m else {}
        note = b.get("note", "")
        cites, flags = [], []
        for r in (m["draft"].get("evidence", []) if m else []):
            d, f = describe(r, cid, answers, it["author"])
            cites.append(d); flags += f
        shared = sorted((words(note) & ctext) - bw)[:12]
        if it["author"].startswith("seed"): flags.append("FIXTURE")
        if it.get("woke"): flags.append("woke-by-reopen")
        if it["kind"] == "Opinion": flags.append("opinion")
        fl = list(dict.fromkeys(flags))
        hard = bool(set(fl) & HARD) or (bool(shared) and "cites-the-brief" in fl and it["direction"] == "Support")
        if not hard: hint_items.append(it)
        out.append(f"| {it['msg']} | {it['author']} | {it['direction']} | {it['kind']} | {b.get('strength', '?')} | {it['delta']:+d} | {'; '.join(cites) or '-'} | {', '.join(fl) or '-'} | {', '.join(shared) or '-'} |")
        notes.append(f"- msg:{it['msg']} ({it['author']}, {it['direction']}): \"{note}\"")
    out.append("")
    out += notes
    out.append("")
    if earlier:
        out.append("Votes from earlier epochs (they scored then; the board no longer counts them):\n")
        for it in sorted(earlier, key=lambda i: i["msg"]):
            m = msgs.get(it["msg"])
            b = body(m) if m else {}
            out.append(f"- msg:{it['msg']} (epoch {it['epoch']}, {it['author']}, {it['direction']}, {it['delta']:+d}): \"{b.get('note', '')}\"")
        out.append("")
    total = sum(i["delta"] for i in items)
    def per_author(pick):
        by = {}
        for i in sorted(items, key=lambda i: i["msg"]):
            if pick == "latest" or i["author"] not in by:
                by[i["author"]] = i
        return by
    latest, first = per_author("latest"), per_author("first")
    lat = ", ".join("%s: msg:%d" % (a, i["msg"]) for a, i in latest.items())
    fst = ", ".join("%s: msg:%d" % (a, i["msg"]) for a, i in first.items())
    hint = sum(i["delta"] for i in hint_items)
    hint_counter_authors = len({i["author"] for i in hint_items if i["direction"] == "Counter"})
    hint_support_authors = len({i["author"] for i in hint_items if i["direction"] == "Support"})
    out.append("### Scores\n")
    out.append(f"- all votes, repeats per author (the board): {total:+d}  (board says {steps[-1]['score']:+d})")
    out.append("- latest vote per author: %+d  (%s)" % (sum(i["delta"] for i in latest.values()), lat))
    out.append("- first vote per author: %+d  (%s)" % (sum(i["delta"] for i in first.values()), fst))
    out.append(f"- mechanical hint, votes with no flag only: {hint:+d}  (counter authors {hint_counter_authors}, support authors {hint_support_authors}); a hint, not the audit: a vote can pass these checks and still invent a fact, or fail them and be fine")
    out.append(f"- distinct support authors on all votes: {len({i['author'] for i in items if i['direction']=='Support'})}\n")
    if status == "Dropped":
        out.append("This claim was DROPPED by the board. If the audit finds it dropped on ungrounded votes, say so and give the grounded-only score against the drop line; do not act (reopen, choose) without Nano.\n")
    sections.append("\n".join(out))
    summary.append((cid, kindword, answers, steps[-1]["score"], hint))

print("## Claims audited\n")
print("| claim | status | answers | board score | mechanical hint |")
print("|---|---|---|---|---|")
for cid, k, a, sc, h in summary:
    print(f"| {cid} | {k} | {a} | {sc:+d} | {h:+d} |")
print()
print("\n".join(sections))
print("### Audit to complete (Claude), before Nano acts\n")
print("For each vote above, quote the exact numbered brief sentence the note relies on, or write UNGROUNDED. A vote that")
print("attributes to the brief wording it does not contain (see the shared-wording column) is ungrounded; a vote that cites a")
print("claim as a run, cites itself or cites another vote is ungrounded. Then, for each claim: the score of grounded votes only")
print("(repeats allowed, and one vote per author), the distinct grounded authors against the rule that fired, and any way the")
print("claim itself departs from the brief.")
PY
cat "$OUT"
