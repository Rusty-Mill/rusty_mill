# Architecture

How `rusty_inventrory` is put together, and why. The capability list it is
built against is in [`../CAPABILITIES.md`](../CAPABILITIES.md).

---

## Shape

```
inventory-tauri ──┐
                  ├──► inventory-core ──► one encrypted SQLite file
inventory-cli   ──┘
```

Every capability lives in `inventory-core`. The two frontends hold no logic
beyond presentation — which is the reason the same features are reachable from
a menu bar panel and from a terminal without either one being a second
implementation.

### `inventory-core` modules

| Module | Responsibility |
| --- | --- |
| `model` | Domain types: `SourceId`, `Conversation`, `Message`, `Role`, `Retention`, source status. |
| `paths` | Where each tool stores history, per platform. Overridable for tests. |
| `keychain` | Index key: create, fetch, fail closed. |
| `db` | Open/migrate the encrypted store; plaintext→encrypted conversion; entropy check. |
| `sources/*` | The six readers, plus the snapshot and VS Code-fork helpers. |
| `embed/*` | On-device embedding: hashing fallback, locally-trained LSA, and the linear algebra behind it. |
| `search` | BM25 + semantic retrieval, RRF fusion, meaning labelling. |
| `index` | The `Inventory` type: indexing, freeze/repair, retention, stats. |
| `capture` | Quick capture and the clipboard scratchpad. |
| `handoff` | Session resumption and hand-off primers. |
| `update` | Update-check *policy*. Deliberately contains no transport. |
| `format` | Human-readable dates, relative times, byte sizes. |

---

## Decisions worth explaining

### Reading each source

Two rules hold everywhere:

1. **Read-only, always.** Nothing writes to, moves, or locks a file a tool
   owns.
2. **Tolerant parsing.** These are undocumented formats that change without
   notice. A record a parser does not recognise is skipped; it never fails the
   file.

**Snapshots.** Editors hold their SQLite stores open in WAL mode. Attaching
directly risks lock contention with a running app, and a read-only open may
refuse to recover the WAL — which would silently hide the most recent
conversations, exactly the ones the user wants. So the database and its `-wal`
and `-shm` sidecars are copied to a temporary directory and the *copy* is
opened. `sources::snapshot` has a test that writes to a WAL without
checkpointing and proves the snapshot still sees the data.

**The VS Code forks** (Cursor, Kiro, Antigravity) share Code's storage layout
and rename their chat keys freely between releases. So `sources::vscdb` finds
conversations *structurally* — it walks the JSON looking for anything shaped
like a message list — rather than reaching for known key paths. A renamed key
therefore does not take the source down. There is a test for exactly this
(`finds_conversations_under_unfamiliar_keys`).

### Freeze rather than delete

When a source fails to parse, the indexer:

- records the failure against that source and marks it **frozen**,
- **keeps every conversation already indexed from it**, still searchable,
- preserves `last_ok_at` so the UI can say when it last read cleanly,
- does **not** persist that pass's seen-file list, so the next run re-reads,
- leaves the other five sources completely unaffected.

Success clears the freeze, which is the whole of the self-repair behaviour.

The subtle part is that a corrupt store must produce an **error**, not an empty
result. Returning "no conversations" from an unreadable file would look
identical to "you have no conversations" — the outcome the freeze exists to
prevent. `vscdb::scan_fork` reads `sqlite_master` immediately after opening for
precisely this reason.

### Hybrid search

Keyword and semantic retrieval run separately and are fused by **Reciprocal
Rank Fusion**:

```
score(d) = Σ  w_list / (60 + rank_list(d))
```

RRF fuses *ranks*, not scores. BM25 values and cosine similarities are not on
comparable scales and their distributions shift with the query, so any weighted
score blend needs calibration that will not hold across six tools' very
different transcripts. Fusing ranks sidesteps the problem.

Recency enters as a **third ranked list** (weight 0.35) over the candidates
already retrieved, rather than as a score multiplier — so it can order
near-ties without ever swamping relevance.

**Query safety.** Every token is quoted and prefix-matched before it reaches
FTS5, so `NEAR(`, `-`, `*` and `"` typed by a user are text, not syntax.
Precision first: all terms ANDed; if that finds nothing, it widens to OR rather
than showing an empty page.

**Labelling.** A hit retrieved only by the semantic arm is labelled, because
"a result with none of your words otherwise looks like a bug". The label is
only *"found by meaning"* when a model that actually models meaning ran;
otherwise it reads *"found by similarity"*. `SearchResponse::semantic_available`
carries that distinction to both frontends.

### The embedding model

This is the largest departure from the reviewed product, which ships a static
embedding model. Rather than download one at runtime — which would put the
product's single strongest claim, that nothing leaves the machine, in tension
with its own setup — the model is **trained locally from the user's own
conversations**:

- tf-idf term/document matrix over the indexed corpus,
- truncated SVD via a randomized range finder with power iterations, the small
  dense eigenproblem solved by cyclic Jacobi,
- term vectors scaled by √σ; a document or query is the idf-weighted mean of
  its term vectors, L2-normalised.

Terms that keep the same company end up with neighbouring vectors. That is what
makes "container" retrieve "pod stuck terminating" — learned from *this user's*
corpus rather than from a general-purpose model, which is arguably the better
fit for jargon, internal service names and project vocabulary that no
general-purpose model has seen.

Consequences, stated plainly:

- It needs a corpus. Below 32 conversations, a hashed-random-projection
  embedder stands in. It is honest about having no semantics
  (`is_semantic() == false`), and the UI degrades its label accordingly.
- The rank adapts down when the vocabulary is small, rather than refusing.
- The model is retrained when the corpus grows 1.5×, and **every vector is
  recomputed** when it is — a retrained model is a new vector space, so mixing
  old and new vectors would be meaningless.
- Out-of-vocabulary queries return a **zero vector**, so the semantic arm
  abstains. The tempting alternative — fall back to the hashing embedder — is
  wrong: its output lives in a different space from the stored document
  vectors, and comparing across the two produces arbitrary similarities that
  would then be shown under a "found by meaning" label.
- Randomness is seeded from a constant, so indexing the same corpus twice gives
  the same vectors.

`Embedder` is a trait, so dropping in a shipped static model later is a new
implementation, not a rewrite.

### Encryption and the key

SQLCipher with a per-machine 256-bit key held in the OS keychain and never
written into the file it unlocks.

The important behaviour is the distinction between **no key yet** (normal first
run — mint one) and **keychain unreadable** (fatal). Collapsing them would let
a transient keychain error look like a fresh install and rebuild an index that
was never actually lost. `Error::KeyUnavailable` exists to make that
non-recoverable by construction, and a wrong key produces `KeyMismatch` rather
than a confusing "file is not a database" much later.

Plaintext indexes are converted by exporting to a staging file, reopening and
verifying it, and only then renaming the original to `.plaintext.bak`. An
interruption at any point leaves a working index behind.

`db::shannon_entropy` exists so `inv doctor` can report the same number the
product's security page invites you to verify.

### No network in the core

`inventory-core` links no HTTP client. `update.rs` holds the *policy* — opt-out,
once-a-day interval, version comparison — and defines an `UpdateTransport`
trait the shell implements. This turns "the app makes no network calls" from a
promise into something checkable with `cargo tree`.

---

## Divergences from the reviewed product

Recorded rather than glossed over.

| Area | Reviewed product | Here | Why |
| --- | --- | --- | --- |
| **Platform** | macOS Apple Silicon only | macOS, Linux, Windows path tables | The core has no macOS-specific dependency; restricting it would be a choice, not a constraint. Only macOS is *tested* against real installs. |
| **Semantic model** | Shipped static embedding model | Trained locally from the user's corpus, hashed fallback until it can be | Avoids downloading a model, and adapts to project-specific vocabulary. Costs a cold-start period and cross-machine reproducibility. |
| **Linux keychain** | n/a | Kernel keyring (`keyutils`), not Secret Service | Secret Service pulls in `libdbus` headers at build time. The kernel keyring does not survive a reboot, so a Linux index may need its key reissued — acceptable for a platform the original does not support at all. |
| **Update transport** | Built in, auto-installing | Policy only; transport is a trait | Keeps the core provably network-free. Auto-install is a packaging concern. |
| **Clip source app** | Tagged with the app | Tagged on macOS via `osascript`; untagged elsewhere | No cross-platform way to get the frontmost app without extra permissions. Untagged beats guessed. |
| **Licensing** | Machine-bound, activated online once | Not implemented | Nothing to activate against. `inv palette` reports the licence line the product shows. |
| **Zed compressed rows** | Presumably decompressed | Skipped | Zed has stored the thread blob compressed in some versions; a row that will not decode is skipped and the rest of the table still indexes. Adding zstd is a small, isolated change. |

## Testing

58 tests, no network, no fixtures checked in as binaries.

- **Unit tests** cover each parser against realistic records, the timestamp and
  calendar round-trip, FTS query escaping, the linear algebra against known
  spectra, encryption round-trip and key mismatch, and version comparison.
- **`tests/end_to_end.rs`** builds a fixture machine with all six tools
  installed and drives the real pipeline: index all six, search across them,
  confirm re-indexing is idempotent, break a source and prove it freezes
  *without losing history* and then repairs itself, and exercise capture,
  scratchpad, resume, primer and retention.
- The semantic capability has a dedicated test
  (`lsa_relates_words_that_share_context`) proving two words that never
  co-occur end up close when they share context.

`docs/make_fixture.py` builds a larger fixture for manual exercise; see the
README.
