# C53 Kitty graphics through the mux (slice plan)

Capability C53 ([CAPABILITIES.md](CAPABILITIES.md), card #682), arc 2 wave 3. Go: Nano, 2026-11-17 14:05Z ("all"
to the go on waves 2 and 3). **Status: plan only.** No code, no wire change. Every wire shape below is
*proposed* and goes to Nano on a decision card before its slice is coded.

Ids: handoff D333-D338 and Q92-Q94 are reserved for this work; gui-design D97-D100 are reserved for the look
(cite them as "gui-design D97", the handoff pack has its own D97-D100). Wire: `Features::IMAGES` = bit 52.
D-rows are written by `rusty-term-expert` when a slice's decision lands, not in this PR.

## What exists

- `rusty_term_core` decodes Kitty (`kitty.rs`), Sixel and iTerm2 into the grid. The headless grid keeps the
  Kitty store (`kitty_images`, at most 8 images and 16 Mi pixels per grid), virtual placeholder cells, and
  half-block fallback cells for Sixel, iTerm2 and plain Kitty placements. RTCP v6 round-trips the store.
- **Placed pixel images (`Grid::images`, `GridImage`) are `gui`-only** and not in the checkpoint
  (`checkpoint.rs` "GUI-only pixel overlays"). The mux server's grid has no `gui`, so a placement's pixel
  rect, cell footprint and scroll anchor are lost there. The engine decodes already; what is missing is
  keeping *placements* headless and carrying them.
- The `rusty_term` window already draws images (gpu and cpu paths) from a local grid.
- No `blob_read` exists in the mux crates. A `BlockCall` is answered with `Text`.
- The wire is 1200-byte datagrams on UDP ([UDP-TRANSPORT.md](UDP-TRANSPORT.md)), so image bytes cannot ride the stream.
- The CAPABILITIES wording "needs the engine's image support" is stale: the engine has it.

## Slices

One PR each. S1 to S3 in order; S4 depends only on the store, so it is built with S2 or before it, so the
wire bit and Q92 are settled once.

| # | Slice | Crates | Gate |
|---|---|---|---|
| S1 | Headless placements (built, card #1558). A bounded list of `{image id, cell rect, scroll serial, stamp}` kept by the headless grid beside the store, dropped with eviction and when the anchor leaves scrollback; checkpoint carries it (RTCP version decided with the code). A per-store change stamp so a delta can skip unchanged images (DELTA.md open question). Half-block fallback cells stay in the grid, so a client without bit 52 sees what it sees today. Prefer a new headless type over moving `GridImage` out of `gui` (more reversible) | `rusty_term_core` | none: engine detail, logged `[default]` |
| S2 | Server snapshot and stream. `Features::IMAGES` (bit 52). Attach and delta carry *descriptors* only (id, w, h, stamp, placement); bytes are fetched, not pushed. Server-wide budget; over budget the client keeps the fallback cells | `rusty_mux_proto`, `rusty_mux_server`, `rusty_mux_client` | wire change, Nano (Q93, Q94) |
| S3 | Client draw. `rusty_mux_gui` fetches bytes for visible placements and draws them through the existing gpu and cpu image paths. TUI and web keep half-block cells. Look per the gui-design rows below | `rusty_mux_gui`, `rusty_term` (gui) | look reviewed by `rusty-term-design-expert` |
| S4 | `terminal.blob_read` block method: image bytes by id, with offset and length paging, on the reliable channel. This is the fetch path for S3 and the REX-APP C1 method | `rusty_mux_proto`, `rusty_mux_server`, `rusty_mux_ctl` | security, Nano (Q92) |

Tests per slice: S1 round-trip and eviction plus a hostile checkpoint (placement past the store, oversize rect);
S2 old-peer sees no image bytes and an unchanged grid; S3 draw through both paths in the test harness; S4 caps, paging
and denied callers. Watch-only shares (C14) must get descriptors and blobs exactly when they may see the grid.

## Other limits

- **Access (C77, C46, C14).** Image bytes are block output. Images and `blob_read` follow screen visibility: an owner-only block
  hides its images from whoever cannot see its output (Q92).
- **History and replay (C70).** The checkpoint carries the store; the spool is line-form and holds no images. A placement that leaves
  scrollback is dropped, and `terminal.history` stays text-only.
- **Memory.** The per-grid cap multiplies by block count (about 64 MiB at 4 bytes a pixel each), hence a server-wide cap (Q94).
- **Windows.** The ConPTY rollback loses images (D169); the default own host passes them.
- **No new crate or dependency.** Child cards are made from this table after the plan merges, each tagged `arc2`, `c53`.

## Look (for `rusty-term-design-expert`, rows proposed as gui-design D97-D100, all `[default]`)

The pack has no image rule and no frame; existing rules cover most of it (D66 unfocused veil, D62 reconnecting, D61 detached
chip, D23 zoom, D50 program output is not chrome). No new frame is needed unless Nano picks a placeholder (Q92 in gui-design terms below).

- **D97:** images draw only inside the block's grid area, clipped there, anchored in cell coordinates so they scroll with their cells; veils and zoom apply as to text; never over header, badges or border.
- **D98:** D50 extends from Kitty animations to GIF, Sixel and iTerm2 animation: they keep playing under reduce motion.
- **D99:** selection highlights image cells normally; copy yields text cells only.
- **D100:** an image that cannot be shown (too large, over budget, failed decode) draws nothing; cells show the terminal background; a stale block redraws its images after resync.

## Questions for Nano

Added to `handoff/OPEN_QUESTIONS.md`. Each has an interim choice, so no slice waits except S2 and S4's wire.

- **Q92 (security):** who may read image bytes, and does an owner-only block hide its images? Recommended: follow screen visibility.
- **Q93 (wire), answered:** Nano chose descriptors on the stream and bytes by `blob_read` (card tap, 2026-11-17T14:13:26Z).
- **Q94 (budgets):** keep the core caps per block (8 images, 16 Mi pixels), add a 256 MiB server-wide cap with oldest-block eviction, and cap a `blob_read` reply at 16 MiB with paging.
- **Look (gui-design, non-blocking):** when an image cannot be shown, show nothing (default D100), a small placeholder line, or a placeholder plus a header badge? Recommended: nothing.
