# gap-analysis.md

Parity target for `rusty_url`: the [`url`](https://crates.io/crates/url) crate
(pinned `url v2.5.8`, `form_urlencoded v1.2.2` — the versions resolved by
`cargo` at the time this analysis was run, 2026-07-23). Reference surface
extracted with `cargo public-api` (nightly rustdoc JSON) against both crates.
Per user decision: reference is `url` crate + the WHATWG URL Living Standard
(used for behavioral correctness where the crate's API alone doesn't capture
it). No exclusions for this round — full public surface of both crates is
in scope. `rusty_url` currently has no `Cargo.toml` or `src/`, so every row
below is "wholly absent."

Not a gap row: **bootstrap** — crate skeleton (`Cargo.toml`, `src/lib.rs`,
CI) has to exist before any of this can land. Filed and implemented first,
outside the table below.

All rows are `Category: fn` unless noted, `Source: diff`, `Platforms: both`
(this is a pure-Rust parsing crate — no OS-specific surface the way a libc
binding would have one, except where noted), `Breaking?: no` (everything is
a first implementation, nothing pre-exists to break).

| # | Symbols (grouped) | Category | Reference | Breaking? | Est. size | Depends on | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `ParseError` (9 variants, `Display`/`Error`/`Clone`/`Eq`) | enum | `url::ParseError` | no | S | — | Self-contained error type; every other issue needs this to report failures. Do first. |
| 2 | `Host<S>` (`Domain`/`Ipv4`/`Ipv6`), `Host::parse`, `Host::parse_opaque`, `Display`/`Clone`/`Eq`/`Ord`/`Hash` | type + fn | `url::Host` | no | M | 1 | WHATWG "host parser": domain vs IPv4 vs IPv6 vs opaque-host, including IDNA/punycode for domains. |
| 3 | `form_urlencoded::{parse, Parse, ParseIntoOwned, byte_serialize, ByteSerialize, Serializer, Target, EncodingOverride}` | fn + type | `form_urlencoded` crate | no | M | — | Standalone `application/x-www-form-urlencoded` codec; no dependency on `Url`. Can land anytime, listed here in dependency order. |
| 4 | `Url` core: struct + WHATWG URL parser (scheme, special-scheme table, authority/userinfo, host via #2, port incl. `port_or_known_default`, path, query, fragment, percent-encoding per component) + `as_str`/`into_string`/`Display`/`Debug`/`AsRef<str>`/`From<Url> for String`/`FromStr`/`TryFrom<&str>`/`Clone`/`Eq`/`Ord`/`Hash` + accessors `scheme`/`is_special`/`cannot_be_a_base`/`has_authority`/`has_host`/`authority`/`username`/`password`/`host`/`host_str`/`domain`/`port`/`path`/`query`/`fragment` | type + fn | `url::Url` | no | L | 1, 2 | The foundational parser. Not further splittable — WHATWG URL parsing produces all fields from one state machine, so accessors can't be built before it. Largest single PR in this loop; implement against the URL Living Standard state machine, not just the crate's public signatures, so behavior (not just shape) matches. |
| 5 | `Origin`, `OpaqueOrigin`, `Url::origin`, `ascii_serialization`, `unicode_serialization`, `is_tuple`, `new_opaque` | type + fn | `url::Origin` | no | M | 4, 2 | HTML/URL "origin" concept — tuple origin for special schemes, opaque otherwise. |
| 6 | Setters: `set_scheme`, `set_username`, `set_password`, `set_host`, `set_ip_host`, `set_port`, `set_path`, `set_query`, `set_fragment` | fn | `url::Url` (setters) | no | M | 4 | Mutation must preserve parser invariants (e.g. can't set host on a cannot-be-a-base URL). |
| 7 | `Url::join`, `Url::make_relative` | fn | `url::Url` | no | M | 4 | Relative-reference resolution per spec section 4.4/4.6. |
| 8 | `Position` enum + `Index<Range<Position>>`/`RangeFrom`/`RangeTo`/`RangeFull` for `Url` | type + fn | `url::Position` | no | S | 4 | Slicing the serialized URL string by component boundary. |
| 9 | `Url::path_segments`, `Url::path_segments_mut`, `PathSegmentsMut` (`push`/`pop`/`pop_if_empty`/`extend`/`clear`) | fn + type | `url::PathSegmentsMut` | no | M | 4, 6 | Path segment iteration/mutation; mutator writes back through `set_path`-equivalent internals. |
| 10 | `Url::query_pairs`, `Url::query_pairs_mut`, `UrlQuery` | fn + type | `url::UrlQuery` | no | S | 4, 3 | Bridges `Url`'s query string to `form_urlencoded::Serializer`/`Parse`. |
| 11 | `ParseOptions`, `Url::options`, `Url::parse_with_params`, `SyntaxViolation` (11 variants + `description`) | type + fn | `url::ParseOptions` | no | M | 4, 3 | Configurable parsing: base URL, encoding override, syntax-violation callback. |
| 12 | `Url::from_file_path`, `Url::from_directory_path`, `Url::to_file_path` | fn | `url::Url` (file-path convs) | no | M | 4 | Genuinely platform-conditional: Windows drive-letter/UNC handling vs. Unix absolute paths. Needs correct behavior on both, not just one. |
| 13 | `Url::socket_addrs` | fn | `url::Url` | no | S | 4 | `std::net` resolution helper using host/port. |

13 gap issues + 1 bootstrap issue = 14 total, worked roughly in dependency
order (1 → 3 in parallel → 2 → 4 → 5/6/7/8 → 9/10 → 11 → 12/13).

**Explicitly out of scope for this round:** none — user opted for full
`url`-crate surface, no exclusions.

**Limitation carried over from the skill:** matching is by symbol name/shape
via `cargo public-api`, not by behavior. Row 4 in particular can look
"done" by API shape while still diverging from the WHATWG state machine on
edge cases (e.g. IDNA edge cases, special-scheme quirks) — worth spot-checks
against the spec and the reference crate's own test suite, not just type
signatures.
