---
category: Added
changelog: CI runs the rleval Python service's tests and builds its image; the image builds from the monorepo root again.
---
## 2026-10-23 - rleval service: CI, and an image that builds after the move

- **Added:** two CI jobs, both in `required-gate`. `rleval-service` runs the service's pytest suite with its `dev` extra (queue, S3 via moto, migrations, crypto) when anything under `rleval/service/` changes. `rleval-image` builds the image and checks that it bundles the scorer and imports the API when the Dockerfile or its ignore file changes. Until now the service's Python code had no CI at all.
- **Fixed:** the image had not built since the move into the monorepo, because its stage 1 ran `cargo build` in a context that held no Cargo workspace. It now builds from the monorepo root (`docker build -f crates/apps/rocket_league/rleval/Dockerfile .`, which compose passes), with `--locked -p replay-scoring` and cache mounts, and `Dockerfile.dockerignore` replaces the unused `rleval/.dockerignore`.
- **Fixed:** `test_recent_feed_is_newest_first` set naive datetimes on a timezone-aware column, which current sqlmodel rejects. It now uses UTC, as the app does.
- **Known limitations:** the service's dependencies are unpinned, so a new upstream release can turn the job red without a change here. The image job does not run for compose-file or `replay-scoring` changes; `cargo test` covers the crate.
