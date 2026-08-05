# rusty_inventrory

A Rust implementation of [Inventory](https://www.myinventory.site) — one
private index for every AI coding conversation on your machine.

It reads the local stores **Claude Code, Codex, Cursor, Zed, Kiro** and
**Antigravity** already write to disk, merges them into a single encrypted
SQLite index, and searches that index by keyword *and* by meaning at once.

No account, no server, no sync. The core library links no HTTP client at all.

![The search panel](docs/panel.png)

The capability review this was built from is in
[`CAPABILITIES.md`](CAPABILITIES.md); the design decisions and the places this
deliberately differs from the reviewed product are in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## What it is, and is not

It is **search**. It does not inject context into any AI tool, does not make
your tools "remember", and is not a memory layer — that is a different
category of product.

## Layout

| Crate | What it is |
| --- | --- |
| `inventory-core` | Everything: source parsers, encrypted store, hybrid search, capture, scratchpad, resume, primers. No network code. |
| `inventory-cli` | `inv` — the full capability set on the terminal. |
| `inventory-tauri` | The menu bar app: tray, global shortcuts, search panel. |

## Build

```bash
cargo build --release          # core + CLI
cargo test  --workspace        # 58 tests
cargo build --release -p inventory-tauri
```

The desktop crate is outside the workspace's `default-members`, so a plain
`cargo build` and `cargo test` need no GUI toolkit. Building it on Linux needs
`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev` and
`librsvg2-dev`; macOS and Windows need nothing extra.

## Use

```bash
inv index                        # read every installed tool
inv search "container stuck"     # keyword + meaning, across all six
inv sources                      # per-source status, including anything frozen
inv show 42                      # print a whole conversation
inv resume 42                    # reopen it in Claude Code or Codex
inv primer 42                    # condense it to paste into any other tool
inv capture "check the auth fix" # save a thought, see what you already solved
inv scratch on                   # opt in to the clipboard scratchpad
inv retention                    # every window, with its on-disk cost
inv palette                      # what ⌘K shows: version, license, shortcuts
inv doctor                       # verify encryption, embeddings, source health
inv stats
```

Search takes `--source`, `--limit`, `--days`, `--json`, and `--no-meaning`
(the ⌘M toggle, off).

### Desktop shortcuts

| Action | Shortcut |
| --- | --- |
| Search | `⌘⇧Space` |
| Quick capture | `⌘⇧N` |
| Clipboard scratchpad | `⌘⇧V` |
| Command palette | `⌘K` |
| Toggle meaning search | `⌘M` |
| Close | `Esc` |

Run `inventory-tauri --show` to open the panel immediately, which is how you
confirm an install works without guessing at the shortcut.

## Capabilities

- **Six sources**, read-only, snapshotted before reading so indexing can never
  interfere with a running editor. History from before install is indexed on
  the first pass.
- **Hybrid search** — SQLite FTS5 BM25 blended with on-device embeddings,
  fused by Reciprocal Rank Fusion, with recency as a third ranked list.
  Matched words are highlighted; hits found only by meaning are labelled.
- **Speaker attribution** across all six formats.
- **Quick capture** that immediately matches a new thought against everything
  already indexed.
- **Clipboard scratchpad**, off by default, tagged with the app each clip came
  from, with export/clear prompts.
- **Session resumption** for Claude Code and Codex, falling back gracefully
  when the project folder has moved.
- **Hand-off primers** — a condensed thread to paste into any other tool.
- **Retention windows** (7/30/90/365/all) that show their on-disk cost before
  you choose.
- **Freeze on parse failure** — a tool changing its storage format cannot
  delete what was already indexed from it, and the source repairs itself once
  it is readable again.
- **Encrypted at rest** with SQLCipher, key in the OS keychain, fail-closed if
  the key cannot be read.

### The encryption boundary

Encryption protects the index once it is **away from this unlocked machine** —
a copied backup, a second account, the drive read elsewhere. It does **not**
protect against a process already running as you while the keychain is
unlocked. That is the same limit every password manager has, and it is part of
the claim rather than a footnote to it.

## Privacy

The index lives in one file you can delete at any time:

- macOS `~/Library/Application Support/site.myinventory.app/inventory.sqlite3`
- Linux `~/.local/share/site.myinventory.app/inventory.sqlite3`
- Windows `%APPDATA%\site.myinventory.app\inventory.sqlite3`

`inventory-core` has no HTTP dependency, so "makes no network calls" is
checkable with `cargo tree`. Update checking is a trait the shell implements,
and it can be turned off.

## Trying it without your own history

`docs/make_fixture.py` builds a fixture machine with all six tools installed
and enough conversations for the semantic model to train on:

```bash
python3 docs/make_fixture.py /tmp/demo-home
export INVENTORY_HOME=/tmp/demo-home INVENTORY_DATA_DIR=/tmp/demo-data
export INVENTORY_INDEX_KEY=$(printf 'a%.0s' {1..64})
cargo run --release -p inventory-cli -- index
cargo run --release -p inventory-cli -- search container
```

The fixture is built so that Codex conversations say "container" and Claude
Code ones say "pod" — searching either surfaces both, and the ones with none
of your words are labelled *found by meaning*.

`INVENTORY_INDEX_KEY` is a testing affordance that keeps the key out of your
real keychain. Do not use it for a real index; it puts the key in the process
environment.

## Status

This implements the reviewed capability list, is cross-platform where the
original is macOS-only, and is not the shipping product. The two significant
departures — a locally-trained embedding model instead of a shipped static
one, and the Linux keychain backend — are documented in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md#divergences-from-the-reviewed-product).
