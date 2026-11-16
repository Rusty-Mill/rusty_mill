# Live sharing (C14): design note

Status: Proposed. Card #278 (arc 1). Built in the PR that adds this note. Wire changes are for Nano's
review on that PR (D281). Decision ids D280-D283 are in the rusty_term pack.

## What it is

A principal shares a session; other admitted principals attach and watch; they type only while they
hold an input lease the block's owner (or the session's creator, for an unowned block) grants. It
needs [shared mode](SHARED-MODE.md) and builds on C77. It adds no second lease system and no second
access list (D280, Nano 2026-11-16).

## The rule

A share is a **batch of watch-only C77 grants**. Attach stays decided by D238/D239's one test: the
client's principal owns the block, created its session, is the server's account, or holds a grant.
A watch-only grant satisfies the test and never the lease: `acquire_lease` refuses it
(`input_gated`) however the block's input mode reads. A full `GrantLease` replaces a watch grant,
which is how a watcher is promoted to type; a share never takes a full grant down to watch-only.

## Wire (D281)

| What | Number |
|---|---|
| Feature `SHARING` | bit 46 |
| `ShareSession{session, client, until}` | client type 0x25 |
| `UnshareSession{session, client}` | client type 0x26 |
| Event `ShareChanged{session, client, until?}` | kind 35, `None` = ended |
| Catalog methods | `share-session` (`--seconds` 1 to 86400), `unshare-session` |
| ctl / rules event | `share-changed` |

Both requests are refused without `SHARING`. Old peers never see them: the server offers the bit and
sends the event only to clients that negotiated it. `GrantChanged` still announces each block's
grant, so a client that does not know sharing sees grants and finds out they are watch-only only
when its lease is refused.

## Behaviour (D281, D282)

- **Who shares.** A client of the server's account or of the session's creator's principal
  (`authorize_manage`, the C33 rule). Only a shared server: elsewhere `NotShared`.
- **Target.** One connection (a `ClientId`), as grants are (D104). A reconnect needs a new share.
- **Reach.** Every block in the session now, and every block made in it later. Blocks whose lease
  the sharer may not grant (another client's owned block, D140) are left out, so an owner keeps the
  say over its own block.
- **End.** `until` (unix ms, as a grant: after now, at most 24 h), `unshare`, or the watcher's
  disconnect. Each ends the watch grants, takes the watcher off any block it had only the watch
  right to, and sends `ShareChanged{until: None}` and the per-block `GrantChanged`. Unshare is
  immediate. A watcher that also has a full grant, or is the creator, owner or account, stays.
- **Not persisted.** A share lives in memory and dies with the server, as grants do; the C45 store
  keeps no new principal field (D164 (1)).
- **Visibility.** `ShareChanged` goes to every client with `SHARING`, as `GrantChanged` goes to
  every client with `INPUT_GATE` (D283). `list` is unchanged: it does not say who watches.

## Not in this slice

- Per-principal or `*` shares (a share names a connection).
- A read-only grant flag on `GrantLease` itself; the watch right comes only from a share.
- Showing watchers in `list` and in the GUI (S3b shows "who else has input" from grants today).

## Tests

Core (`rusty_mux_core`): a watcher attaches and never leases; a full grant promotes it; unshare
detaches at once with the events; later blocks are watched; expiry; only creator and account share;
only on a shared server; disconnect ends a share; an owned block is skipped. Wire
(`server/tests/shared.rs`): share and unshare answered, `ShareChanged` delivered, request refused
without the bit. A second uid is needed for the end-to-end watcher; the model tests carry it, as
for shared mode.
