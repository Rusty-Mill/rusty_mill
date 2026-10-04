//! A minimal, real server binary for the `Fair Play` domain
//! (`FPL-FR-007`, ADR-0137): the three tables — `card` (primary),
//! `person`, `card_default` — of one data directory, served on one
//! listener by `serve_tables` (ADR-0050). The multi-table shape of
//! `memory_server.rs`, reduced to its basic path: a durable data
//! directory, authentication, TLS, the exposure check and a graceful
//! drain; no change log, backup root, journal, MVCC or metrics
//! listener (each is a `memory_server` precedent to copy when a
//! deployment asks for it).
//!
//! # `SERVER_DATA_DIR` is required
//!
//! Unlike `memory_server`, there is no scratch dataset: the deck is
//! loaded once by `examples/fair_play_seed.rs` into a directory this
//! binary then opens — `people.mmap`, `cards.mmap`, `card_defaults.mmap`
//! (`PERSON_FILE`/`CARD_FILE`/`CARD_DEFAULT_FILE`), each opened when
//! present and created empty when not (`DDR-FR-001`). One process per
//! directory (`DataDirLock`, ADR-0092).
//!
//! # Usage
//!
//! `SERVER_DATA_DIR=<dir> fair_play_server [host:port]` — the address
//! defaults to `127.0.0.1:0`, and the banner on stderr names the port
//! the kernel bound (`RGT-FR-004`). `SERVER_AUTH_READ_ONLY_TOKEN`/
//! `SERVER_AUTH_READ_WRITE_TOKEN` (ADR-0012), `SERVER_TLS_CERT_CHAIN_PATH`/
//! `SERVER_TLS_PRIVATE_KEY_PATH`/`SERVER_TLS_CLIENT_CA_PATH` (ADR-0014/
//! ADR-0023) and `SERVER_ALLOW_INSECURE` (ADR-0094) are read exactly as
//! `memory_server` reads them; `SERVER_DRAIN_TIMEOUT_SECS` (ADR-0127)
//! bounds the drain SIGTERM/SIGINT start on Linux. This is a local
//! development tool: do not expose it beyond a trusted network unless
//! auth and TLS are both configured.

use rusty_multimodal_db::generic::fair_play::{
    open_or_create_card_default_production_stack, open_or_create_card_production_stack,
    open_or_create_person_production_stack, CARD_DEFAULT_FILE, CARD_FILE, PERSON_FILE,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::data_lock::DataDirLock;
use rusty_multimodal_db::server::exposure::{allow_insecure_from_env, check_exposure};
use rusty_multimodal_db::server::fair_play::{
    CardConnectionStore, CardDefaultConnectionStore, PersonConnectionStore,
};
use rusty_multimodal_db::server::{serve_tables, ConnectionStore, ServeOptions, TlsConfig};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

/// `DRN-FR-005` (ADR-0127): SIGTERM and SIGINT ask the server to drain —
/// `memory_server`'s own handler, unchanged. Linux only; elsewhere the
/// server runs until it is killed.
#[cfg(target_os = "linux")]
mod signals {
    use rusty_libc::signal::{signal, SIGINT, SIGTERM};
    use rusty_multimodal_db::server::Shutdown;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    static SIGNALLED: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_signal(_: i32) {
        SIGNALLED.store(true, Ordering::SeqCst);
    }

    /// Route SIGTERM and SIGINT to `shutdown`. `false` if either handler
    /// could not be installed, in which case the server is left as it was.
    pub fn drain_on_signal(shutdown: Shutdown) -> bool {
        let handler = on_signal as extern "C" fn(i32) as usize;
        // SAFETY: `on_signal` is an `extern "C" fn(i32)` that lives for the
        // whole process and performs one atomic store, which is
        // async-signal-safe.
        let installed =
            unsafe { signal(SIGTERM, handler).is_ok() && signal(SIGINT, handler).is_ok() };
        if installed {
            std::thread::spawn(move || {
                while !SIGNALLED.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                shutdown.request();
            });
        }
        installed
    }
}

fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:0".to_string());

    let dir = std::env::var_os("SERVER_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| {
        panic!("SERVER_DATA_DIR is required: the directory examples/fair_play_seed.rs loaded (ADR-0137)")
    });
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("SERVER_DATA_DIR {dir:?}: {e}"));
    let _data_dir_lock = DataDirLock::acquire(&dir).unwrap_or_else(|e| {
        panic!("SERVER_DATA_DIR {dir:?}: {e} — one fair_play_server per data directory")
    });
    let people = open_or_create_person_production_stack(&dir.join(PERSON_FILE))
        .unwrap_or_else(|e| panic!("opening {PERSON_FILE} in {dir:?}: {e}"));
    let cards = open_or_create_card_production_stack(&dir.join(CARD_FILE))
        .unwrap_or_else(|e| panic!("opening {CARD_FILE} in {dir:?}: {e}"));
    let defaults = open_or_create_card_default_production_stack(&dir.join(CARD_DEFAULT_FILE))
        .unwrap_or_else(|e| panic!("opening {CARD_DEFAULT_FILE} in {dir:?}: {e}"));
    let card: Arc<dyn ConnectionStore> =
        Arc::new(CardConnectionStore::new(GenericProductionStore::new(cards)));
    let person: Arc<dyn ConnectionStore> = Arc::new(PersonConnectionStore::new(
        GenericProductionStore::new(people),
    ));
    let card_default: Arc<dyn ConnectionStore> = Arc::new(CardDefaultConnectionStore::new(
        GenericProductionStore::new(defaults),
    ));

    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| panic!("binding {addr}: {e}"));
    // `RGT-FR-004` (ADR-0123): the banner names the address the kernel
    // bound, so `:0` is a usable request.
    let addr = listener
        .local_addr()
        .map(|bound| bound.to_string())
        .unwrap_or(addr);

    // `SERVER_AUTH_*_TOKEN` (ADR-0012) and TLS (ADR-0014/ADR-0023), read
    // as `memory_server` reads them; a configured-but-invalid TLS pair is
    // a startup error (`SRV-FR-003`).
    let auth = ServeOptions::from_env();
    let options = match TlsConfig::from_env() {
        None => auth,
        Some(Ok(tls)) => auth.with_tls(tls),
        Some(Err(e)) => panic!(
            "SERVER_TLS_CERT_CHAIN_PATH/SERVER_TLS_PRIVATE_KEY_PATH/SERVER_TLS_CLIENT_CA_PATH configured but invalid: {e}"
        ),
    };
    // `EXP-FR-002`/`003` (ADR-0094): a non-loopback bind without both
    // authentication and TLS is refused at startup; `SERVER_ALLOW_INSECURE=1`
    // turns the refusal into a warning for the operator who means it.
    if let Err(exposure) = check_exposure(&addr, &options) {
        if allow_insecure_from_env() {
            eprintln!(
                "WARNING: listening on {addr} although {exposure} (SERVER_ALLOW_INSECURE=1 is set)"
            );
        } else {
            panic!(
                "refusing to listen on {addr}: {exposure}; bind a loopback address, configure \
                 what is missing, or set SERVER_ALLOW_INSECURE=1 to serve anyway (ADR-0094)"
            );
        }
    }
    // `DRN-FR-005` (ADR-0127): on Linux, SIGTERM/SIGINT drain the server;
    // if the handlers cannot be installed it runs until killed.
    #[cfg(target_os = "linux")]
    let options = {
        let shutdown = rusty_multimodal_db::server::Shutdown::new();
        if signals::drain_on_signal(shutdown.clone()) {
            let secs = std::env::var("SERVER_DRAIN_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(30);
            options
                .with_shutdown(shutdown)
                .with_drain_timeout(std::time::Duration::from_secs(secs))
        } else {
            options
        }
    };

    eprintln!(
        "fair_play_server listening on {addr} (data: {}, auth: {}, TLS: {} — see ADR-0012/ADR-0014/ADR-0050/ADR-0094/ADR-0137; do not expose beyond a trusted network unless auth and TLS are both configured)",
        dir.display(),
        if options.is_configured() { "configured" } else { "NOT configured" },
        match options.tls() {
            None => "NOT configured",
            Some(tls) if tls.requires_client_certificate() => "configured, client certificate required",
            Some(_) => "configured",
        },
    );

    // `TBL-FR-001` (ADR-0050): three tables on one listener, `card` primary.
    let outcome = serve_tables(
        listener,
        vec![
            ("card".to_string(), card),
            ("person".to_string(), person),
            ("card_default".to_string(), card_default),
        ],
        0,
        options,
    );
    eprintln!("fair_play_server: stopped with {outcome:?}");
}
