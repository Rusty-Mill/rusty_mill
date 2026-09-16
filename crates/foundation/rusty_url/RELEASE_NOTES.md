# Release Notes

## Unreleased

- Bootstrap crate skeleton: `Cargo.toml`, `src/lib.rs`, CI (build/test/clippy/fmt),
  license files, `.gitignore`. (#1)
- Add `ParseError`: the 9-variant, `#[non_exhaustive]` URL-parsing error enum,
  with `Display`/`std::error::Error`/`Clone`/`Copy`/`PartialEq`/`Eq`. (#2)
- Add `Host<S>` and the WHATWG host parser (domain via `idna`, IPv4, IPv6,
  opaque host), plus `Display`/`Clone`/`Eq`/`Ord`/`Hash`. Adds a dependency
  on the `idna` crate for IDNA/punycode domain normalization (by design —
  see #3 discussion). (#3)
- Add the `form_urlencoded` module: `parse`/`Parse`/`ParseIntoOwned`,
  `byte_serialize`/`ByteSerialize`, and `Serializer`/`Target`/
  `EncodingOverride` for building `application/x-www-form-urlencoded`
  strings. Standalone — no dependency on `Url`. (#4)
- Add `Url`: the WHATWG basic URL parser (special schemes incl. `file:`
  with Windows drive-letter handling, non-special/opaque-path schemes,
  dot-segment path normalization, percent-encoding per component) plus
  `Url::parse`, `as_str`/`into_string`, `Display`/`Debug`/`Clone`/`Eq`/
  `Ord`/`Hash`/`AsRef<str>`/`From<Url> for String`/`FromStr`/
  `TryFrom<&str>`, and accessors `scheme`/`is_special`/`cannot_be_a_base`/
  `has_authority`/`authority`/`username`/`password`/`has_host`/`host`/
  `host_str`/`domain`/`port`/`port_or_known_default`/`path`/`query`/
  `fragment`. Scoped to `Url::parse` without a base URL — `join` and
  `ParseOptions::base_url` are later issues. Differentially tested against
  55 cases from the reference `url` crate (byte-for-byte identical output).
  (#5)
- Add `Origin`/`OpaqueOrigin` and `Url::origin()`: `(scheme, host, port)`
  tuple origins for `ftp`/`http`/`https`/`ws`/`wss`, recursive origin
  resolution for `blob:` URLs, and opaque (equal-only-to-itself) origins
  for everything else, plus `ascii_serialization`/`unicode_serialization`.
  (#6)
- Add `Url` setters: `set_scheme`/`set_username`/`set_password`/
  `set_host`/`set_ip_host`/`set_port`/`set_path`/`set_query`/
  `set_fragment`. Introduces a parser `Context` (URL-parser vs. setter) so
  re-parsing an isolated component (e.g. `set_path`) doesn't treat an
  embedded `?`/`#` as starting a new component. Matches the reference
  crate's documented quirks exactly, including `set_host(None)` leaving
  `has_host()` true with an empty-string domain rather than clearing to
  no host — verified against the reference `url` crate across 31
  setter-call cases (byte-for-byte identical output). (#7)
- Add `Url::join` and `Url::make_relative`. `join` required adding the
  base-relative branches to the parser that were deliberately deferred in
  #5 (`parse_relative`, `fragment_only`, and `file:`-scheme base
  inheritance, including the Windows-drive-letter case); `make_relative`
  is pure string logic over two already-parsed URLs, no parser changes
  needed. Verified against the reference `url` crate across 29 `join`
  cases and 12 `make_relative` cases (byte-for-byte identical output),
  including scheme-relative/absolute/dot-segment resolution, `file:`
  drive-letter inheritance, and cannot-be-a-base rejection. (#8)
- Add `Position` and `Index<Range/RangeFrom/RangeTo/RangeFull<Position>>`
  for `Url`, letting callers slice the serialized URL string by component
  boundary (e.g. `&url[Position::BeforeUsername..Position::AfterPort]`
  for the authority). Verified against the reference `url` crate across 7
  URLs spanning special/non-special/`file:`/cannot-be-a-base schemes and
  missing-component (empty-range) cases — byte-for-byte identical output.
  (#9)
- Add `Url::path_segments`/`path_segments_mut` and `PathSegmentsMut`
  (`push`/`pop`/`pop_if_empty`/`extend`/`clear`). Introduces a third
  parser `Context` (`PathSegmentSetter`) so `extend()` treats each segment
  string as one opaque unit — `/` and `%` within it get percent-encoded
  rather than read as a separator or existing escape. Verified against
  the reference `url` crate's own documented examples plus additional
  cases (non-special scheme, multi-segment `extend`) — byte-for-byte
  identical output. (#10)
- Add `Url::query_pairs`/`query_pairs_mut` and `UrlQuery` (the
  `form_urlencoded::Target` bridging `query_pairs_mut`'s `Serializer` back
  into the URL, preserving the fragment across the edit and restoring it
  when the serializer is finished or dropped). Verified against the
  reference `url` crate — decoded-pairs iteration, append/clear/extend
  mutation, fragment preservation, and adding a query to a URL that had
  none — byte-for-byte identical output. (#11)
- Add `ParseOptions`/`Url::options`/`SyntaxViolation`/`Url::parse_with_params`.
  `ParseOptions` is a fluent builder (`base_url`/`encoding_override`/
  `syntax_violation_callback`) over the parser's full configuration —
  `Url::parse` and `Url::join` are now thin wrappers around it. Adds
  `check_url_code_point`/`is_url_code_point` and threads non-fatal
  `SyntaxViolation` reporting (backslash-as-separator, embedded
  credentials, expected-`//`, NULL-in-fragment, bad `%XX`, non-URL code
  points, etc.) through every parser state, plus a scheme-filtered
  query-string `encoding_override` for legacy non-UTF-8 form encodings.
  Purely additive — no parsed output changes when the callback/override
  aren't set. Verified against the reference `url` crate across 11 cases
  (base-URL resolution, `parse_with_params`, six violation-triggering
  inputs, encoding-override applied vs. scheme-filtered-out, and a
  combined base+violation case) — byte-for-byte identical output. (#12)
- Add `Url::from_file_path`/`Url::from_directory_path`/`Url::to_file_path`
  for converting between `file:` URLs and `std::path::Path`. Platform-
  conditional, matching the reference crate's split: Unix-like systems
  treat paths as raw bytes under a single root, while Windows paths carry
  a drive letter or UNC-share prefix that becomes the URL's host (ported
  faithfully but only compile-checked, not run, since CI is Linux-only —
  the string-manipulation half, `file_url_segments_to_pathbuf_windows`, is
  additionally unit-tested directly since it doesn't depend on Windows-
  specific `Path` parsing). Verified the Unix path against the reference
  `url` crate across 13 cases (`from_file_path`/`from_directory_path`
  success and error cases, joining off a directory vs. file base,
  `to_file_path` including `localhost`/non-local-host/non-`file:`-scheme
  rejection, and round-tripping) — byte-for-byte identical output. (#13)
- Add `Url::socket_addrs`, resolving a URL's host and port to one or more
  `std::net::SocketAddr` (via the standard library's DNS support for a
  domain host, or directly for an IP-literal host), with a caller-supplied
  `default_port_number` fallback for schemes this crate doesn't know a
  default port for. This closes the last tracked gap against the
  reference `url` crate's public API surface. Verified against the
  reference `url` crate across 6 deterministic cases (IPv4/IPv6-literal
  hosts, explicit vs. known-default vs. callback-supplied port, and the
  two error paths — no host, no resolvable port) — byte-for-byte
  identical output. Domain-name resolution isn't covered by a
  differential/unit test, since real DNS lookups would make the test
  suite non-deterministic and network-dependent; the domain branch is a
  direct, untransformed call into `(domain, port).to_socket_addrs()`. (#14)
