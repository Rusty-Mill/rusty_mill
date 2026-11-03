//! Racing first calls to `runtime::install_fallback` start one runtime. Its
//! own binary, so no other test has installed the fallback first.

use rusty_tokio::runtime::install_fallback;
use std::sync::{Arc, Barrier};

#[test]
fn racing_first_installs_share_one_runtime() {
    const CALLERS: usize = 8;
    let barrier = Arc::new(Barrier::new(CALLERS));

    let ids: Vec<_> = (0..CALLERS)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                install_fallback().expect("fallback").id()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|caller| caller.join().expect("caller"))
        .collect();

    assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
}
