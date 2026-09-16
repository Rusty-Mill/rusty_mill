# The ACP specification, vendored

`acp-0.2.0-openapi.yaml` is the ACP v0.2.0 OpenAPI document, fetched verbatim from
[i-am-bee/acp](https://github.com/i-am-bee/acp/blob/main/docs/spec/openapi.yaml).

It is here so `tests/spec_coverage.rs` can check the README's central claim — *every endpoint
in the specification is implemented* — against the specification rather than against memory.
Before this, that claim and the table under it were maintained by hand, and nothing failed when
they drifted. Something checkable already had: the README claimed 163 tests when the crate had grown past 290.

## Why vendored rather than fetched

A test that reaches the network fails when the network does, which turns an unrelated outage into
a red build. It would also silently retarget the crate the day upstream edits the document — the
check would keep passing against a moving definition of "the spec", which is the opposite of what
it is for.

Pinning has the cost you would expect: the file goes stale, and nothing here notices. That is the
intended trade. This crate targets **v0.2.0** specifically, and moving to another version should
be a deliberate change with a diff to read, not something that happens to a test run on a Tuesday.

## Updating it

```sh
curl -sSL -o spec/acp-<version>-openapi.yaml \
  https://raw.githubusercontent.com/i-am-bee/acp/main/docs/spec/openapi.yaml
```

Name the file for the version it contains, update `SPEC` in `tests/spec_coverage.rs`, and expect
the test to fail until the crate actually implements whatever changed. `the_vendored_spec_is_the_version_this_crate_targets`
asserts the document's own `info.version`, so a file swapped underneath the test says so
immediately rather than quietly grading the crate against a different specification.
