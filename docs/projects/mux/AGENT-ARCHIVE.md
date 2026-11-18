# Archive dismissed agent blocks (card #1570)

Status: design, proposed; not authorized to build. Source: Nano, voice, 2026-11-18 (remind-me mem_2faddab4, a decision; card #1570). Nano gates it. Pack ids to reserve: handoff D339, Q98.

## Lifecycle

Launch on card take, running (agent holds the lease), held on exit (C78: output and exit code), dismissed once the card has evidence (the orchestrator enforces it). **Dismissed means archived, not deleted.**

## Archive record

Transcript (C45/C70 screen and history format), exit code, start/end, agent identity, model, card id, session/project, who dismissed and why, and a label (good/bad, or the card outcome) plus a `sensitive` mark.

Recall: by card, project or date range through `rusty-mux-ctl archive list|show|replay` and the switcher. Replay opens the record read-only in a held block in the same session (C78, REPLAY.md, RTCP v6). The [session overview](../term/SESSION-OVERVIEW.md) links to a card's archived runs.

## Why

1. The transcript is the card's evidence, re-openable in place.
2. Export to remind-me as a dialog plus summary, so facts and decisions distil from it.
3. The label makes archived runs training examples (learned router, future fine-tuning). It lives on the record and travels with the export.

## Constraints

- D75 deletes the spool when a block closes. Dismiss must copy history into the archive first.
- D74: the state dir holds screens unencrypted and owner-only. D74 covers the existing restore state only. It does **not** authorize indefinite archive retention or transcript export; both need Nano's recorded answer (Q98).
- Retention: keep all; a size cap with oldest-first eviction only when Nano sets one.

## Wire (not allocated here)

A store record type, a recall call (ctl and wire) behind a Features bit, and a dismiss message carrying card id, evidence links and label. The remind-me export is a new dependency edge and an outside write: owner gate. Redaction is security-relevant.

## Order

1. Archive on dismiss plus `ctl archive list|show` (cloud-buildable).
2. Replay in a held block (cloud-buildable; the GUI header is a GUI slice).
3. Export to remind-me (owner gate, after 1 and 2).

## Open questions (proposed only; implementation gated until Nano's answer is recorded)

Every item below is a recommendation, not an authorized default. Nano gates this design (card tag `needs-nano`). Wire, security, privacy, retention and export behaviour must not be built until his answer is recorded in the pack by the expert.

- **Q98a** Where does the archive live? a) mux state dir with an optional cap, exported to remind-me on dismiss; b) orch owns it, mux streams; c) remind-me only. **Recommend a**: the mux holds the grid, history and exit code, and D74 already gates the dir.
- **Q98b** Secrets: a) mark `sensitive`, exclude from export and labels, no redaction; b) pattern redaction plus the mark; c) never export transcripts. **Recommend a**: pattern redaction misses things.
