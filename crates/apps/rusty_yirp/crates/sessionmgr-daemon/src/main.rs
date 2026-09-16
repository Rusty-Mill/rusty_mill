//! The `sessionmgr` binary. All three roles share this entrypoint; see
//! `lib.rs` for the dispatch table.

#[rusty_tokio::main]
async fn main() {
    // Must run before this process spawns anything at all. Every role --
    // including the hidden `__worker-main` entrypoint, which goes on to
    // spawn a child of its own -- shares this `main`, so doing it here,
    // first, covers every spawn in the process. See the function's own
    // documentation for the hang this prevents.
    sessionmgr_proc::harden_inherited_stdio();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match sessionmgr_daemon::run(&args).await {
        Ok(()) => std::process::exit(0),
        Err(err) => {
            eprintln!("sessionmgr: {err}");
            std::process::exit(err.exit_code());
        }
    }
}
