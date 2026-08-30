#![cfg(windows)]
//! Sandbox parity check (RFC v2 R5+, D15, Phase 6 item 3): the Windows
//! backend has no confinement mechanism yet (see
//! `docs/design-discussion-sandbox.md`) and must say so honestly rather
//! than silently claiming enforcement — no re-exec trickery needed here
//! since `Unsupported` touches no OS state at all.

use std::path::Path;

use platform::security::{Sandbox, SandboxStatus};

#[test]
fn confine_filesystem_reports_unsupported() {
    let sandbox = platform_windows::WindowsSandbox;
    let root: &Path = Path::new(".");
    let status = sandbox.confine_filesystem(&[root], &[]).unwrap();
    assert_eq!(status, SandboxStatus::Unsupported);
}

#[test]
fn block_inet_sockets_reports_unsupported() {
    let sandbox = platform_windows::WindowsSandbox;
    let status = sandbox.block_inet_sockets().unwrap();
    assert_eq!(status, SandboxStatus::Unsupported);
}
