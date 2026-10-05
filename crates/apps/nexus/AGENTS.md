<!-- gitnexus:start -->
# Code impact analysis

Assess impact using available source, dependency, and call-site analysis plus
focused tests. GitNexus is optional assistance, not a prerequisite for editing
or committing; its absence must not block this work.

## Always Do

- **MUST assess impact before editing a symbol.** Inspect its direct callers,
  dependencies, and affected execution flows. Report the blast radius and risk
  level to the user. Use source searches and Cargo metadata when appropriate;
  GitNexus impact/query/context tools may supplement that analysis if available.
- **MUST review changed scope before committing.** Inspect the diff, verify that
  changed symbols and execution flows are expected, and run focused regression
  tests and applicable dependency checks. Optional `gitnexus_detect_changes()`
  can supplement this review.
- **MUST warn the user** about HIGH or CRITICAL risk before proceeding with edits.
- If relying on GitNexus, check index freshness. Refresh a stale index with
  `npx gitnexus analyze` when available, or use current source analysis instead.

## Never Do

- NEVER edit a function, class, or method without first assessing its impact.
- NEVER ignore HIGH or CRITICAL risk findings.
- NEVER rename symbols with blind find-and-replace. Use semantic tooling or
  verify every affected definition and reference with source review and tests.
- NEVER commit without reviewing changed scope and the applicable test results.

## Optional GitNexus resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/nexus/context` | Codebase overview, check index freshness |
| `gitnexus://repo/nexus/clusters` | All functional areas |
| `gitnexus://repo/nexus/processes` | All execution flows |
| `gitnexus://repo/nexus/process/{name}` | Step-by-step execution trace |

## Optional GitNexus guidance (when installed)

| Task | Skill file |
|------|------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
