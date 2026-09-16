# Capability Review — myinventory.site

A capability inventory of **Inventory**, taken from a deep read of
<https://www.myinventory.site> on 2026-08-05. This is the specification that
`rusty_inventrory` is built against.

Pages reviewed: `/`, `/docs`, `/security`, `/changelog`, `/compare`,
`/vs/built-in-history`, `/vs/cloud-ai-memory`, `/about`, `/llms.txt`,
`/sitemap.xml`.

---

## 1. What the product is

> "Inventory is a local-first macOS menu bar app that indexes the conversation
> history your AI coding tools already write to disk, and makes it searchable
> by keyword and by meaning, across tools, in one place. No account, no
> server, no data leaves the machine." — `/llms.txt`

Positioning line from the homepage: *"One private index for every AI
conversation on your machine. Nothing leaves your Mac, and what stays is
encrypted."*

### What it explicitly is **not**

The site is unusually precise about the negative space, and `/llms.txt`
contains a "what not to claim on this product's behalf" section. These are
scope boundaries for the rebuild, not marketing trivia:

| Not | Why it matters |
| --- | --- |
| Not a memory layer | It never injects context into a tool automatically. "It is search, not autocomplete for context." |
| Not context injection | It does not feed anything back into a model. |
| Not cloud-based | No account, no server, no cross-machine sync. |
| Not cross-platform | macOS Apple Silicon only; "Windows source exists but has never been compiled or run." |

---

## 2. Sources indexed

Six, exactly: **Claude Code, Codex, Cursor, Zed, Kiro, Antigravity.**

- Reads the local stores those tools *already* write — no cooperation from
  the tool required.
- **Read-only, always.** "It never writes to them, and it reads live
  databases through a private snapshot so indexing can never interfere."
- Reads whichever are installed and ignores the rest.
- Indexes history from **before install** — "Everything already on disk,
  however far back it goes. There is nothing to enable and no waiting period."

---

## 3. Search

The core capability, and the one with the most specified internals.

- **Hybrid ranking.** "SQLite FTS5 keyword search blended with on-device
  semantic search (a static embedding model, no network call, no server-side
  model), fused by **Reciprocal Rank Fusion**." — `/llms.txt`
- **Semantic recall.** Every conversation is embedded on-device so
  *"container stuck"* finds *"pod stuck terminating"*.
- **Meaning hits are labelled.** "Results found only by meaning are labelled
  as such in the UI" — *"because a result with none of your words otherwise
  looks like a bug."*
- **Ranked by relevance and recency**, with matched words highlighted in
  context.
- **Cross-tool by construction** — one search box, one result list, filterable
  per source (`All / Cursor / Claude Code / Zed / Codex / Kiro / Antigravity`).
- **Knows who said what** — speaker attribution is a compare-matrix row.
- Reachable "a keystroke from anywhere, app closed."

Homepage stat strip: `0ms search latency · 6 tools indexed · 100% on-device ·
∞ history kept`.

---

## 4. Feature list

| # | Capability | Detail as specified |
| --- | --- | --- |
| 1 | **Global search** | `⌘⇧Space` from any app. Full-text + semantic across every tool. |
| 2 | **Search by meaning** | On-device embeddings blended with keyword results; meaning-only hits labelled. Toggle with `⌘M`. |
| 3 | **Quick capture** | `⌘⇧N` opens a one-field capture; on save it "immediately matches it against everything you've already worked out" and surfaces related conversations. |
| 4 | **Clipboard scratchpad** | `⌘⇧V` shows a running list of everything copied, **tagged with the app it came from**. **Off by default**, opt-in, with periodic export/clear prompts. |
| 5 | **Session resumption** | Claude Code and Codex sessions "reopen in the terminal with the full transcript attached, so the agent picks up with real context, **even if the project folder has moved**." |
| 6 | **Hand-off primers** | Copy a condensed primer of any thread — "what was being solved, the last exchanges and the code" — to paste as an opening message in any other tool. |
| 7 | **Retention windows** | Index the last 7, 30, 90, 365 days, or everything. The command palette "shows the on-disk size of each choice, so the trade is visible before you make it." |
| 8 | **Command palette** | `⌘K`. Also shows running version, license state, and update availability. |
| 9 | **Background indexing** | "First pass takes seconds. After that it stays live as you work, reading each file once." |
| 10 | **Per-source status** | Shows last successful read date per source when a source becomes unreadable. |
| 11 | **Freeze-on-parse-failure** | A source that stops parsing "freezes at its last known-good state rather than losing already-indexed history, and is retried automatically on the next launch." A tool changing its format "can no longer delete what Inventory had already indexed from it." |
| 12 | **Self-repair** | "A source that breaks repairs itself once it can be read again" — no manual reindex. |
| 13 | **Encryption at rest** | SQLCipher, key per machine in the macOS Keychain. |
| 14 | **Auto-update** | Updates install themselves; notification before install. The only network call the app makes. |

### Keyboard shortcuts (complete)

| Action | Shortcut |
| --- | --- |
| Global search | `⌘⇧Space` |
| Quick capture | `⌘⇧N` |
| Clipboard scratchpad | `⌘⇧V` |
| Command palette | `⌘K` |
| Toggle meaning search | `⌘M` |
| Close window | `Esc` |

---

## 5. Security and privacy

- **One file:** `~/Library/Application Support/site.myinventory.app/inventory.sqlite3`
- **Encrypted at rest with SQLCipher** — "the same library as Signal Desktop
  and 1Password's mobile apps."
- **Key handling:** "The key is generated on your Mac, stored in the macOS
  Keychain, and never written into the file it unlocks."
- **Verifiable:** "the encrypted file scores 8.0000 bits per byte of Shannon
  entropy — the theoretical maximum."
- **Decryption is in-memory** — "Search is unaffected; decryption happens in
  memory as you use it."
- **Fail-closed:** "If the key ever cannot be read, Inventory stops rather
  than starting over." Protects against wiping the index over a transient
  Keychain error.
- **Safe migration:** existing plaintext indexes are "converted
  automatically, without touching the original until the new one is proven,"
  with verification and backup retention.
- **Network:** the *only* request is an update check. Fully functional
  offline.

### The stated encryption boundary (must be repeated, not dropped)

`/llms.txt` is explicit that this is part of the claim rather than a caveat:
encryption protects the file **once separated from the unlocked Mac** — a
copied backup, a second account, a drive read elsewhere. It does **not**
protect against a process already running as that user with the Keychain
unlocked. Same limit as every macOS password manager.

---

## 6. Commercial and platform

- **$19.99 once.** No subscription, no account, no seat pricing, no feature
  tiers. Free updates for life.
- Machine-bound license, **activated once online, verified offline afterward**.
- Payments via **Polar**.
- **macOS** (Apple Silicon) today; **Windows "coming soon"** with a notify
  form.

---

## 7. Release history

| Version | Contents |
| --- | --- |
| Private beta | Multi-tool search across Cursor, Claude Code, Zed, Codex, Kiro; semantic + keyword search; quick capture; clipboard scratchpad; session resumption; conversation primers; retention windows; keyboard shortcuts. |
| 1.0.0 | Antigravity added as a supported source. |
| 1.0.1 | Freeze-on-format-change; per-source status with last successful read; self-repair. |
| 1.0.2 | SQLCipher encryption at rest; automatic verified index conversion; auto-updates; version/license in command palette; fail-closed Keychain error handling. |

All dated August 2026.

---

## 8. Competitive framing

Compare matrix rows (`/compare`): searches across every tool at once · knows
who said what · finds it when you can't recall the words · reads history from
before you installed it · nothing leaves your machine · works with no network
· survives a tool changing its storage format · cost.

- **vs built-in per-tool history:** built-in is free and good *within one
  tool*. "Inventory earns its price at the second tool, not the first."
- **vs cloud AI memory:** a genuinely different category — memory layers make
  your AI arrive briefed; Inventory helps you find what was already said.
  "They do not overlap much." Cloud memory requires an account, stores on the
  provider's servers, needs network, and "usually starts from the day you
  connect it."

---

## 9. What this repo builds

`rusty_inventrory` is a Rust implementation of the capability list above,
restructured as a portable core rather than a macOS-only app:

- `inventory-core` — sources, encrypted store, hybrid search, all features.
- `inventory-cli` — `inv`, the full capability set on the terminal.
- `inventory-tauri` — tray app, global shortcuts, search palette.

Deliberate divergences from the reviewed product are recorded in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md#divergences-from-the-reviewed-product);
the most significant are cross-platform paths and keychain backends, and a
locally-trained embedding model in place of a shipped static one.
