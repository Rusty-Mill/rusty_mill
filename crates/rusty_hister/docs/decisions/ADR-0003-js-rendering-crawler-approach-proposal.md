# ADR-0003: JS-rendering crawler approach

Status: Accepted
Date: 2026-09-12 (decided; superseding the same-day decision-request below)

## Decided, not just requested

This ADR was opened as a decision-request and decided later the same day on
the user's (baileyrd/Nano's) explicit instruction — "Decide ADR-0002 and
ADR-0003 now." The Context and Options sections below are unchanged from
the request; the Decision section after them records what was chosen and
why.

## This was a decision-request, not a decision (as originally opened)

Per the kickoff brief: headless-browser/CDP control for JS-rendered
crawling has no coverage anywhere in this workspace today (confirmed by
grepping the whole workspace for "chromedp"/"CDP"/"devtools protocol"/
WebSocket-client usage — nothing exists). This ADR lays out the options for
the user to choose between; it does not choose one.

## Context

Hister's Go source (`server/crawler/`, capability inventory
`docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §8) has **three**
crawler backends behind one `fetcher` interface, and the JS-rendering half
splits into two genuinely different problems:

1. **`http` backend** — plain HTTP fetch, no JS rendering. Already covered:
   `rusty_http`/`rusty_request` handle this (see ADR-0001's sovereignty
   audit). Not blocked by this ADR.
2. **`chromedp` backend** (`crawler/chromedp.go`) — wraps the mature Go
   `chromedp` library, which drives headless Chrome over the Chrome
   DevTools Protocol (CDP). The Rust ecosystem has reasonably mature
   equivalents (`chromiumoxide`, `fantoccini` for WebDriver-classic). This
   is a "pick and validate a crate" problem, not a from-scratch one.
3. **`bidi` backend** (`crawler/bidi.go`, 434 lines) — Hister
   **hand-implements the W3C WebDriver BiDi protocol directly over a raw
   WebSocket connection**, deliberately with no external driver binary or
   CDP library dependency (its own doc comment: *"talks directly to the
   browser over a WebSocket — no external driver binary or library
   needed."*). The Rust WebDriver/BiDi crate ecosystem is thin and immature
   compared to Python's or JavaScript's BiDi support as of this writing —
   there is no drop-in equivalent.

Separately, `cmd/companion/qutebrowser/cdp.go` implements a **fourth**,
independent raw-CDP client for the qutebrowser companion daemon (a
DevTools-protocol daemon that attaches to an already-running qutebrowser
instance rather than launching/controlling headless Chrome from a start
URL) — but the companion is deferred to a later phase per ADR-0001's v1
scope decision, so it isn't this ADR's immediate concern, though whatever
CDP client this ADR's decision produces would likely be reused there later.

The **Notion extractor hard-depends** on JS rendering existing at all
(capability inventory §4.5.16: it only works when the crawler ran with
`chromedp` or `bidi`) — so "descope all JS rendering" and "descope only
BiDi" are differently-sized decisions, not one lever.

## Options

### Option A: Hand-roll a minimal CDP client on `rusty_tokio` +
`rusty_http`/`rusty_tls` + `rusty_json`

Build a first-party CDP WebSocket client from scratch, matching this
workspace's general sovereignty preference for hand-rolled infrastructure
over external dependencies where feasible.

- **Pro**: no new external dependency; consistent with this workspace's own
  stated direction; the resulting crate is a genuine, reusable
  `rusty_*`-shaped platform crate (a first "CDP client" building block,
  similar in spirit to `rusty_http` or `rusty_json`), and could also serve
  the qutebrowser-companion phase later and any other future workspace need
  for browser automation.
- **Con**: real engineering effort — CDP's protocol surface (Target,
  Page, Runtime, Network domains at minimum) is larger than the BiDi
  subset Hister hand-rolled (434 lines gets Hister only BiDi, a narrower
  protocol than full CDP); this is the more expensive of the two possible
  hand-roll targets (CDP vs. BiDi) since Hister's own `chromedp` backend
  wraps an existing mature Go library rather than hand-rolling CDP itself,
  meaning there's no equivalently-scoped Go reference implementation to
  study for the CDP path the way there is for BiDi.

### Option B: External dependency — `chromiumoxide` (or `fantoccini`),
ADR-justified per this workspace's Tier A (adapter) dependency policy

Take on a documented external dependency for the `chromedp`-equivalent
path, per the same reasoning this workspace already applies to `rusty_tls`
(rustls) and SQLx-based DB adapters (root ADR-0002's dependency-sovereignty
policy: external integration is allowed behind a first-party boundary for
things "not expected to ever reach zero dependencies... it must not be
marketed as dependency-free").

- **Pro**: dramatically less engineering effort than Option A; `chromiumoxide`
  is a maintained, reasonably complete CDP client; matches how this
  workspace already treats other adapter-tier needs (TLS, SQL databases)
  rather than insisting on zero dependencies everywhere.
- **Con**: a new external dependency, which per root ADR-0002 needs its own
  tier classification (Tier A) and rationale recorded either here or in a
  crate-level note; still doesn't solve the BiDi half (see below) since
  `chromiumoxide`/`fantoccini` target CDP/WebDriver-classic, not BiDi.

### For the `bidi` backend specifically (orthogonal to A/B above)

1. **Re-implement BiDi from scratch**, following the shape of Hister's own
   434-line implementation (a JSON-RPC-like command/response correlation
   layer over a raw WebSocket) — achievable in isolation, and per the
   capability inventory's own assessment, "only" 434 lines of fairly
   mechanical protocol code in the Go original.
2. **Explicit descope**, keeping only the `chromedp`-equivalent JS-rendering
   path. This is a real, sign-off-worthy scope reduction (not just an
   implementation detail) — BiDi's selling point over CDP is being a
   standards-track protocol usable without a vendor-specific driver
   binary, and dropping it changes an operational property of the
   crawler, not just which code implements it.

## Recommended framing, as originally written

Treat the CDP path (Option A vs. B) and the BiDi sub-decision as two
separate calls, since their risk profiles and available off-the-shelf
answers are completely different — bundling them into one "headless browser
support: yes/no" decision would hide that the BiDi half is the genuinely
open-ended one. (This framing held: the two calls below are decided
separately, for different reasons.)

## Decision

**CDP path: Option B — `chromiumoxide`** (or a substantively equivalent,
actively-maintained Rust CDP client; the exact crate pick is left to
`rusty-hister-crawler`'s implementation phase to re-confirm against current
maintenance status, not re-litigated here) **as a documented Tier A adapter
dependency**, per root ADR-0002's dependency-sovereignty policy.

Rationale: Hister's own `chromedp` backend already wraps an external Go
library rather than hand-rolling CDP itself — matching that with a
hand-rolled *Rust* CDP client (Option A) would mean this port doing *more*
from-scratch work than the reference implementation it's porting, for a
protocol surface (Target, Page, Runtime, Network domains at minimum) that's
large and moves with each Chrome release. That maintenance burden is closer
in kind to TLS or a SQL wire protocol — both already accepted as Tier A
adapters in this workspace (root ADR-0002) — than to something worth this
port owning end-to-end. JS-rendering is a means to an end for Hister (get
the DOM after scripts run), not its differentiating feature, so the lower-
engineering-cost path is the right default here.

**BiDi path: explicit descope for v1.** Not re-implemented.

Rationale: BiDi's entire selling point in the Go original is avoiding a
CDP-library/driver-binary dependency — a benefit that evaporates the moment
this port already accepts `chromiumoxide` (a CDP library) for the primary
path above. Paying to hand-roll a second, immature-in-Rust protocol for a
benefit already given up elsewhere isn't a good trade. This is recorded
here as the explicit, sign-off-worthy scope reduction the capability
inventory flagged it as being (capability inventory §8.1's `bidi` backend
row moves from REQUIRED to **OUT-OF-SCOPE, decided 2026-09-12** by this
ADR) — not silently dropped. The **Notion extractor's** JS-rendering
requirement (capability inventory §4.5.16) is unaffected: it's satisfied by
the CDP path above, which is not descoped. BiDi support can be revisited as
a later-phase addition if a concrete driver-free-deployment need shows up;
nothing here forecloses that.

**qutebrowser-companion's own CDP client**: decided to **reuse whatever CDP
client this ADR produces** (`chromiumoxide`, or its replacement if one is
adopted later) when that later phase starts, rather than leaving it as a
separate future decision — no reason to re-litigate the same CDP-client
choice twice for two consumers within the same cluster.

This decision does not itself add `chromiumoxide` (or any CDP crate) to any
`Cargo.toml` — it lands when `rusty-hister-crawler`'s JS-rendering backend
actually starts consuming it (roadmap Phase 4), not in this decision
record.
