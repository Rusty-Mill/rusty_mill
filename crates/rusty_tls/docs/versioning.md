# Versioning and releases — `rusty_tls`

This repo follows the ecosystem convention in
[`rustils`' `docs/versioning.md`](https://github.com/baileyrd/rustils/blob/main/docs/versioning.md).
That document already answers the general questions, and this one does not
restate them:

- **§2** — the numbering rule at `0.y.z`.
- **§3** — pinning sibling repos by `rev`, and when to cut a tag.
- **§4** — bump in the PR that changes the API; tag lazily.

What follows is only the part that is specific to `rusty_tls`, plus the one
hazard the ecosystem document does not cover because it does not apply to
`rustils`.

## The numbering rule, applied here

> At `0.y.z`: any change to the public API surface — additive or breaking —
> bumps `y`. `z` is reserved for changes that touch no public item's shape at
> all.

`rusty_tls` has a single version for the whole crate; there is no workspace and
no group question to answer.

One clarification worth writing down, because it has already come up: **items
behind a feature gate are still public API.** The `handrolled` module is
reachable only with `handrolled-engine` *and* `--cfg rusty_tls_handrolled`, and
it still bumps `y` when it grows, because "public" is a question about the item
and not about how many people can currently see it. A rule that exempted gated
surface would mean the version stopped tracking the thing it exists to track
the moment anything interesting was gated.

## The hazard this repo actually has: trait identity across `rusty_tokio` revs

This is why `rusty_tls#7` was filed, and it is not a general versioning problem
— it is specific to the shape of this crate's dependencies.

`AsyncTlsStream` (behind the `rusty-tokio` feature) is generic over
`rusty_tokio`'s own `AsyncRead`/`AsyncWrite` traits. So is
`rusty_http::async_tokio::AsyncTransport`. A consumer such as `rusty_request`
depends on both.

**Cargo treats two different `rev`s of the same git dependency as two unrelated
crates.** If `rusty_tls` is built against `rusty_tokio` rev A and `rusty_http`
against rev B, then `rusty_tokio-A::AsyncRead` and `rusty_tokio-B::AsyncRead`
are different traits that happen to share a name, and the consumer gets a
type-identity error that reads like a mistake in their own code.

Nothing in the type system, and nothing in a version number on its own,
prevents this. What prevents it is knowing which `rusty_tokio` rev a given
`rusty_tls` release was built and tested against — and until this document
existed, the only place that was written down was a prose comment in a
downstream consumer's `Cargo.toml`.

### So: every release records its sibling revs

Each version entry in `CHANGELOG.md` names the `rusty_tokio` and `rustils`
revs that version was built and tested against. That is not decoration and not
an implementation detail; for a consumer resolving the graph above, **it is
part of what the version means**.

The same information belongs in `RELEASE_NOTES.md`'s entry for the version, so
that whichever file a reader lands on answers the question.

## Cutting a tag

Per the ecosystem document's §3 and §4: **tag lazily, at the consumer-trigger
point** — when an external consumer actually needs a tag to pin against, not
speculatively on every merge. Most PRs bump the number and stop there.

The mechanics, in order, because getting them out of order produces a tag that
points at the wrong thing:

1. The version bump lands on `main` as part of an ordinary PR.
2. The tag is cut **on the merge commit**, not on the branch that produced it.
3. `git tag -a vX.Y.Z -m "…"` — annotated, so it carries a date and an author.
4. Push the tag explicitly: `git push origin vX.Y.Z`.

A tag is effectively immutable once anything has pinned to it. Cut it
deliberately.

## Consumers: pin by tag or by rev, never by branch

Both are equally valid and carry the same guarantee. A tag is more readable; a
bare SHA is fine. A **branch name is not**, because it silently resolves to
something different on every fresh `Cargo.lock`.

```toml
# Readable, once a tag exists:
rusty_tls = { git = "https://github.com/baileyrd/rusty_tls", tag = "v0.2.1" }

# Equivalent, and what this repo does for its own siblings:
rusty_tls = { git = "https://github.com/baileyrd/rusty_tls", rev = "76efe35…" }
```

A consumer using `AsyncTlsStream` should additionally pin `rusty_tokio` to the
rev this release names, for the reason above.

## Why the version field must never be stale

The ecosystem document's §4 says to bump in the same PR that changes the API,
rather than in a later release PR. This repo learned why the hard way: the
thirteen PRs that built the hand-rolled engine (rusty_tls#25) each added public
surface and none of them bumped the version, so `Cargo.toml` claimed `0.1.0`
across the entire span while the crate grew an alternative TLS implementation.

Nothing was broken by that — no consumer was pinning by version, because there
was no version to pin. But it is precisely the state `rusty_tls#7` describes:
a version field that "carries no signal about what's actually inside a given
checkout."

CI now fails a pull request that changes `src/` without changing
`Cargo.toml`'s version line. Under the §2 rule every source change bumps
something — `y` if a public item's shape moved, `z` if not — so "source changed
but the version did not" is always a mistake, and it is cheap to catch
mechanically rather than in review.
