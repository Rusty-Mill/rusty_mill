## What
<!-- Dependency bump, CI tweak, refactor with no behavior change, etc. -->

## Why
<!-- What prompted this — a security advisory, a broken workflow, cleanup after a
     prior PR, etc. -->

## Checklist
- [ ] No intended behavior change (or: behavior change is called out explicitly above)
- [ ] `cargo test --all-features` passes locally
- [ ] `cargo clippy --all-features --all-targets -- -D warnings` clean
