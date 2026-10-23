# Native Ubuntu package download cache

The shared `setup-build-env` action caches **downloaded `.deb` archives only**.
It never caches `/usr`, `/var/lib/dpkg`, APT indexes, locks, credentials or an
installed system. `native-packages.txt` contains the same 18 explicit packages
previously installed inline. Normal APT dependency/recommendation resolution is
unchanged. Direct mingw, valgrind and other job-local APT steps in `ci.yml` are
outside this initial cache's scope. Swatinem's existing Rust cache remains separate.

## Installation and integrity

1. On GitHub-hosted Ubuntu only, prepare an unprivileged staging directory under
   `RUNNER_TEMP` and use the official, SHA-pinned `actions/cache/restore@v5`.
2. Always run `apt-get update`, including on exact cache hits. Use the system
   `/usr/bin/python3` and `python3-apt` bindings to read package records. Each
   record must belong to its own trusted (signed) APT index and provide SHA-256
   and size for the exact package name, version and architecture. dpkg status and
   unsigned origins cannot authenticate a restored archive.
3. Copy only regular `.deb` files with matching bytes through private temporary
   files into a fresh, root-owned `/var/tmp/rusty-mill-apt-*` directory. The root
   installation directory is **never** restored from a cache. Cache provenance or
   a key match alone is not trusted: branch-local/poisoned cache data must pass the
   same current APT metadata validation. Symlinks, malformed, corrupt and no-longer
   indexed archives are ignored. Hash the copied bytes before making them visible
   to APT, avoiding a source-file race between checking and copying.
4. Always perform normal authenticated `apt-get install -y` for the package list.
   Do not pass local archive paths to `dpkg` or skip installation on a hit. New
   versions and missing dependencies download normally, even when an old version
   remains valid in the indexes.
5. Retain downloads using both APT/apt-get keep-download options and the isolated
   archive directory. This avoids the image's default `/var/cache/apt` cleanup
   hooks without disabling them or changing host configuration. Compare the
   install's `--print-uris` download plan with files surviving successful install;
   that plan is used only to check retention, never as authenticity evidence.
6. Revalidate retained archives, export only `.deb` files as an unprivileged step,
   and save immediately after successful native installation. Saving does not wait
   for Rust tests. Failed installs propagate normally and cannot produce save
   outputs. Missing retention/verification disables saving with a warning.

Cache service/key-preparation/export/save failures are optional. A failed or
partial restore supplies at most individually verified files; other downloads use
APT normally. Missing system `python3-apt` means no restored bytes are accepted and
no cache is published; it does not install a new verifier dependency. The helper
does not change repositories, mirror priorities, suites or signature settings.
The independent mirror work in #926 remains compatible and runs before APT update.
Self-hosted/Linux developers do not use the download cache; the normal install
path is unchanged. No setup/install command is run locally by the fixture tests.

## Keys, writers and refresh

Key: `apt-debs-v1-main-ubuntu-<release>-<dpkg-architecture>-<SHA256(package-list)>-<UTC YYYYMMDD>`.
Restore fallback removes **only the date**, retaining release, architecture and
package-list separation. Daily immutable generations allow the next existing
main sweep to publish refreshed archives. Within a day an immutable hit can miss
new downloads, but can never freeze installed security versions: APT updates and
resolves current versions on every install. Old archives cannot substitute for
different versions or architectures. No cache is deleted by this implementation.

Only successful `schedule` or `workflow_dispatch` runs with repository
`Rusty-Mill/rusty_mill` and ref `refs/heads/main` may save. PRs (including forks),
merge groups, other branches and other events never publish this cache.
`pull_request_target` is excluded from consumption too. The existing daily full
main sweep already invokes this action, so it warms the cache without another job,
workflow or full build. Concurrent writers may race for one immutable key;
`actions/cache/save` conflicts are nonfatal. An exact hit is not resaved.

Both cache actions use commit `caa296126883cff596d87d8935842f9db880ef25` (v5,
Node 24, hosted runner requirement 2.327.1+). The existing allowlist permits
GitHub-owned actions; no allowlist, token permissions or security settings change.
Restore segments time out after one minute. The enclosing setup's **15-minute
cap** and all tests, failure gates and merge-queue requirements remain intact.

## Validation and bounded performance check

Run `python3 .github/scripts/test_apt_download_cache.py` and the existing workflow,
platform, lint and change-fragment tests. Fixtures cover cold/warm/missing caches,
corruption, origin trust, updates, install failure, event/ref boundaries and retained
downloads. These tests do not measure package download speed.

Real cold/warm performance is pending trusted execution **after this PR is merged**.
Do not enable writes from the PR to obtain a benchmark. Use the next two normal main
daily sweeps (or an already-needed manual main full sweep), not a duplicate warming
build. Compare the existing setup step durations, restore hit/matched-key logs and
the helper's `verified-reused`, `planned-downloads`, `retention`, `install-seconds`
and retained-export count. A first miss establishes cold behavior; a subsequent
restore (including the prior-date fallback) establishes warm behavior. Record image
version, Ubuntu release, architecture and package-list hash so differences in
preinstalled packages are visible. Require archive retention and at least one
verified reused package, successful unchanged-package installation, and reaching
compilation within the existing cap before claiming a benefit. If no archives are
needed or restoration misses, there is no warm measurement. No sources or system
dependencies on Beast are changed to run this check.

References: [official cache restore/save documentation](https://github.com/actions/cache/tree/caa296126883cff596d87d8935842f9db880ef25),
[APT trusted index and package record API](https://apt-team.pages.debian.net/python-apt/library/apt_pkg.html).
