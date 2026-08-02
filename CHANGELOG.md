# Changelog

All notable changes to `rusty_tokio` are recorded here, starting from the
first tagged release. Versions follow [Semantic Versioning](https://semver.org/):
a bump to the second number (`0.X.0`) means a breaking change -- including a
trait-identity break, the sharpest edge for crates generic over this one's
`AsyncRead`/`AsyncWrite` (see [#107](https://github.com/baileyrd/rusty_tokio/issues/107))
-- while a bump to the third (`0.1.X`) is purely additive.

No changelog was kept before this point; `git log` is the record for
anything prior to v0.2.0.

## [0.2.0] - 2026-08-02

### Breaking

- `sync::Semaphore::acquire`/`acquire_many`/`acquire_owned`/`acquire_many_owned`
  now return `Result<SemaphorePermit, AcquireError>` (previously infallible).
  `try_acquire`/`try_acquire_many`/`try_acquire_owned`/`try_acquire_many_owned`
  now return `Result<SemaphorePermit, TryAcquireError>` (previously `Option`).
  This widening is what makes the new `close`/`is_closed` below possible at
  all. ([#122](https://github.com/baileyrd/rusty_tokio/issues/122))

### Added

- `sync::Semaphore::close`/`is_closed`: closing a semaphore wakes every
  queued waiter with `AcquireError` and fails every subsequent
  `acquire`/`try_acquire` call the same way, without disturbing permits
  already held. ([#122](https://github.com/baileyrd/rusty_tokio/issues/122))

### Fixed

- `Cargo.toml`'s `rusty_std` dependency is now a pinned `git` dependency
  instead of a `path` dependency, so this crate can actually be built as a
  git dependency from outside this repo's own multi-repo dev checkout.
  Requires the matching fix in `rusty_std` itself (its own `rusty_libc`/
  `rusty_win32` path deps converted the same way).
  ([#254](https://github.com/baileyrd/rusty_tokio/issues/254))
