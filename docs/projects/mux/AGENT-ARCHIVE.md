# Archive dismissed agent blocks (card #1570)

Status: design; archive and secrets accepted (handoff D339, Nano 2026-11-18). Export to remind-me is its own slice behind the owner gate. Source: Nano, voice, 2026-11-18 (remind-me mem_2faddab4, a decision; card #1570).

## Lifecycle

Launch on card take, running (agent holds the lease), held on exit (C78: output and exit code), dismissed once the card has evidence (the orchestrator enforces it). **Dismissed means archived, not deleted.**

## Archive record

Transcript (C45/C70 screen and history format), exit code, start/end, agent identity, model, card id, session/project, who dismissed and why, and a label (good/bad, or the card outcome) plus a `sensitive` mark. The archive step scans the transcript and sets `sensitive` on any hit; nothing relies on someone remembering (D339).

Recall: by card, project or date range through `rusty-mux-ctl archive list|show|replay` and the switcher. Replay opens the record read-only in a held block in the same session (C78, REPLAY.md, RTCP v6). The [session overview](../term/SESSION-OVERVIEW.md) links to a card's archived runs.

## Why

1. The transcript is the card's evidence, re-openable in place.
2. Export to remind-me as a dialog plus summary, so facts and decisions distil from it.
3. The label makes archived runs training examples (learned router, future fine-tuning). It lives on the record and travels with the export.

## Constraints

- D75 deletes the spool when a block closes. Dismiss must copy history into the archive first.
- D74: the state dir holds screens unencrypted and owner-only. D74 covers the existing restore state only. D339 settles where the archive lives (below); transcript export stays its own gated slice.
- Location (D339): the mux state dir, not loaded at startup, covered by backups. No redaction (it gives false confidence).
- Export: a `sensitive` record still exports metadata and exit code so the card keeps its evidence; the transcript stays out of export and labels.
- Retention: keep all; a size cap with oldest-first eviction only when Nano sets one.

## Wire (D339)

A store record type, a recall call (ctl and wire) behind a Features bit, and a dismiss message carrying card id, evidence links and label. The remind-me export is a new dependency edge and an outside write: owner gate. The sensitive scan is security-relevant.

## Order

1. Archive on dismiss plus `ctl archive list|show` (cloud-buildable).
2. Replay in a held block (cloud-buildable; the GUI header is a GUI slice).
3. Export to remind-me (owner gate, after 1 and 2).

## Decided

- **Q98** (D339, Nano 2026-11-18): archive in the mux state dir, not loaded at startup, backup-covered; no redaction; scan at archive time and mark `sensitive` on any hit; still export metadata and exit code. Export to remind-me is its own slice.
