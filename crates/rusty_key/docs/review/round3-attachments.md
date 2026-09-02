# Round 3 — attachments & source provenance

> Companion to the round-3 persona audits (`round3-*.md`) and their synthesis
> (`round3-consolidated.md`). Records what was reviewed, what the attachments
> actually are, and the resolution of the long-standing "PDF not renderable"
> caveat in [`../research/references.md`](../research/references.md).

---

## 1. What was attached

The review was triggered by two **visual summaries** of the foundational paper:

1. **`Engineering the Agent Harness` — a 13-slide NotebookLM deck** (PDF, 17 MB).
   Each page is a single full-bleed 1376×768 illustration (isometric "structural
   exoskeleton" infographics) with **no text layer** — image-only, so it is not
   text-extractable. Slide 1 title: *"Engineering the Agent Harness — Building the
   Structural Exoskeleton for Autonomous AI in Production."* Footer credits NotebookLM.
   (The harness reported it as "167 pages"; it is in fact 13 image slides.)
2. **A mind-map PNG** summarising the paper's structure (transcribed in §3).

Both are **derivative visual glosses of the foundational paper**, not new external
sources. They re-group the paper's canonical *11 responsibilities + 5 design
principles* into a friendlier "5 pillars" framing (see §3). The round-3 audit
therefore treats them as **framing only** and audits Rusty Keys against the
**paper text**, not the slides.

## 2. Authoritative source — now readable (caveat resolved)

The foundational paper is **Zhong, H. & Zhu, S., *AI Harness Engineering*, arXiv
`2605.13357v1` (CC-BY 4.0)** — already in the repo at
[`../research/2605.13357v1.pdf`](../research/2605.13357v1.pdf).

`references.md` carried a caveat: the build environment lacked PDF tooling
(no poppler/pdftotext/pypdf), so the original design was built from a **degraded
zlib `FlateDecode` recovery** that stripped inter-word spaces and ligatures, and
**three details were left unconfirmed**, blocking the freeze of several *Proposed*
ADRs.

**That blocker is now resolved.** `PyMuPDF` (`fitz`) was installed in this session
and extracts the paper cleanly (16 pages, ~8,400 words). The clean text is saved
to [`../research/2605.13357v1.txt`](../research/2605.13357v1.txt) and is the
shared input for every round-3 audit. The PDF is born-digital with a real text
layer — the original problem was missing *tooling*, not an image-only PDF (unlike
the attached slide deck, which **is** image-only).

### 2.1 The three previously-unconfirmed details — now confirmed verbatim

| # | Detail | Paper text (2605.13357v1) | Matches prior assumption? |
|---|---|---|---|
| 1 | Entropy categories + severity | "categories of agent-introduced maintenance burden—**code, documentation, dependency, test, file residue, architecture, workflow**—together with a **0–3 severity**" | ✓ exactly (7 categories, 0–3) |
| 2 | M-HIR denominator | "**M-HIR = missing-harness interventions / total episodes**" | ✓ ("total episodes") |
| 3 | Intervention-log fields | "the intervention log records **human assistance, its avoidability, its burden level, and the harness gap** it corresponds to" | ✓ (avoidability / burden / harness-gap) |

The `ai-harness-engineer` audit (`round3-ai-harness-engineer.md`) carries these into
a per-ADR **freeze / fix-then-freeze / keep-Proposed** recommendation.

### 2.2 Other paper invariants confirmed against the clean text

- **Definition:** the harness is a *runtime substrate* mediating how a model agent
  **observes** a project, **acts** on it, receives **feedback**, and **establishes
  completion**. "Agent = Model + Harness."
- **Eleven component responsibilities:** task specification, context selection, tool
  access, project memory, task state, observability, failure attribution,
  verification, permissions, entropy auditing, intervention recording.
- **H0–H3 ladder:** a controlled-visibility ablation exposing progressively more
  runtime support; runs adjudicated by **verification autonomy, not task success**.
- **Episode package = eight evidence classes:** action, tool, context, verification,
  failure attribution, intervention, entropy, outcome.
- **Outcome taxonomy = five labels** (see the harness-engineer audit for verbatim).
- **Failure taxonomy = eight types:** Fcontext, Ftool, Ffeedback, Fverify, Frecovery,
  Fentropy, Fmodel, Funknown.

## 3. Mind-map transcription (framing)

Root: **AI Harness Engineering**

1. **Definition & Core Premise** — Agent = Model + Harness · runtime substrate
   mediating agent and environment · external scaffolding for reliability and
   autonomy · shift from "Dragon" to "Dragon Trainer".
2. **Core Components (Harness Pillars)** — Context & Memory Management · Tool
   Orchestration & Action Surface · Verification & Feedback Loops · Guardrails &
   Governance · Observability & Tracing.
3. **Evaluation Framework** — H0–H3 harness ladder · metrics · episode package
   (auditable trace-based evidence).
4. **Design Principles** — explicit runtime resources · traceable mediation ·
   attribution before recovery · maintenance and entropy awareness.
5. **Implementation & Tools** — OpenHarness (HKUDS) · Claude Code (Anthropic) ·
   Deep Agents (LangChain) · Codex (OpenAI) · NxCode & MindStudio.

> Note: the "5 pillars" in branch 2 are a presentation re-grouping of the paper's
> 11 responsibilities; the audits use the canonical 11.

## 4. Scope of round 3

A **faithfulness + completeness audit** of Rusty Keys against the now-readable
paper — *not* an external-source applicability review (that was round 2). Five
persona audits, each additive (one `round3-<persona>.md`), then a consolidated
synthesis. Recommended changes (freezing ADRs, correcting any drift, removing the
`references.md` caveat) are deferred to a follow-up apply phase, per the round-2
two-phase pattern.
