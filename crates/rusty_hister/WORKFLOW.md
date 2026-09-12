# Repository Development Workflow

## Authority

The RustyMill workspace's `main` is authoritative for this cluster, same as
every other crate in the monorepo.

## Source of truth

- Read `AGENTS.md` and `docs/PROJECT-STATUS.md` before starting work.
- Treat `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` as the
  spec of record for what must exist; treat `docs/decisions/` as the record
  of what has (and hasn't yet) been decided about *how*.
- Report conflicts between this cluster's docs and actual code state; do not
  rely on conversation memory over repository evidence.

## Standing process

1. Every change lands through a PR against the workspace's default branch.
2. Merge with a merge commit on green CI — never squash or rebase-merge
   (history stays intact, same convention as the rest of RustyMill).
3. CI is the root workspace's existing affected-crates-filter pipeline
   (`.github/workflows/ci.yml`'s `plan` job); no separate CI wiring is
   needed for crates already listed in the root `Cargo.toml`'s `members`.
4. Don't begin a competing increment on the same crate while a PR touching
   it is open.

## Safeguards

- Never merge failing, pending, missing, stale, or older-head CI.
- Don't silently expand or narrow scope — a capability-inventory row moves
  to out-of-scope only via an explicit ADR recording the user's sign-off.
- Ask before anything hard to reverse: the crate-cluster split once
  non-bootstrap code depends on it, the licensing approach for ported test
  fixtures (ADR-0001), the search-engine choice (ADR-0002) and the
  JS-rendering-crawler approach (ADR-0003) while either is still open, or
  any dependency/toolchain change.

## ADRs

Write one per delivery cycle while this cluster is in active bootstrap/major
development (its current regime) — see `docs/decisions/`, numbered from this
cluster's own `0001` per the root workspace's ADR-0001 remit (root
`docs/adr/` covers workspace-wide decisions; this cluster's own
`docs/decisions/` covers decisions internal to `rusty_hister`).

## Next steps after this bootstrap commit

`ADR-0002` (search/indexing engine) and `ADR-0003` (JS-rendering crawler
approach) are open, awaiting the user's sign-off. Implementation work on
`rusty-hister-indexer`, `rusty-hister-vectorstore`'s storage side, and
`rusty-hister-crawler`'s JS-rendering backends should not start until those
land. Work that isn't blocked on either (capability inventory review,
`rusty-hister-core`'s `Document`/error types, `rusty-hister-model`'s schema,
`rusty-hister-extractor`'s SDK and non-JS-dependent extractors,
`rusty-hister-server`'s route table, `rusty-hister-mcp`'s tool surface) can
proceed in parallel.
