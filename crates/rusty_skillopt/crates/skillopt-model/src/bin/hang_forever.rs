//! Test-only stand-in for a wedged subprocess. Ignores every argument and
//! every byte of stdin, then sleeps far longer than any test's own timeout
//! budget -- used by `tests/claude_cli_timeout.rs` and
//! `tests/aisf_stage_timeout.rs` to prove `ClaudeCliBackend::chat` and
//! `AisfStageBackend::chat` recover (rather than hang forever) when the
//! real `claude`/AISF child process never exits.
fn main() {
    std::thread::sleep(std::time::Duration::from_secs(3600));
}
