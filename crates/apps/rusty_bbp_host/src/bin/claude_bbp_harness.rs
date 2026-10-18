//! MCP-only Claude reviewer bound to the moderator's current BBP turn.
fn main() {
    if let Err(error) = rusty_bbp_host::launchers::run(rusty_bbp_host::launchers::Provider::Claude)
    {
        eprintln!("claude-bbp-harness: {error}");
        std::process::exit(1);
    }
}
