//! Phase 1 exit criteria (docs/projects/mux/ROADMAP.md), against a real
//! server on a Unix socket with real PTYs running `/bin/sh`, or pseudo
//! consoles running `cmd.exe` on Windows.

#![cfg(any(target_os = "linux", windows))]

mod common;

use std::time::{Duration, Instant};

use common::{Client, block_for, run_and_wait, start_server};
use rusty_mux_proto::{Features, Message};

#[test]
fn two_clients_converge_on_one_pty() {
    let path = start_server();
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let mut b = Client::connect(&path);
    b.request(|req| Message::Attach { req, block });

    let marker = run_and_wait(&mut a, block, 21);
    b.wait_for(block, &marker);
    // Settle: one more round trip so both have seen the prompt that follows.
    let marker = run_and_wait(&mut a, block, 22);
    b.wait_for(block, &marker);
    let quiet = run_and_wait(&mut a, block, 23);
    b.wait_for(block, &quiet);

    assert_eq!(a.snapshot(block), b.snapshot(block));
}

#[test]
fn reattach_restores_the_grid_from_the_snapshot() {
    let path = start_server();
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let mut b = Client::connect(&path);
    b.request(|req| Message::Attach { req, block });
    run_and_wait(&mut a, block, 31);

    b.request(|req| Message::Detach { req });
    run_and_wait(&mut a, block, 32);
    let last = run_and_wait(&mut a, block, 33);

    b.request(|req| Message::Attach { req, block });
    // B's grid now comes only from the snapshot; A's from live output.
    assert!(
        !b.saw(block, &last),
        "output after reattach should be new only"
    );
    let after = run_and_wait(&mut a, block, 34);
    b.wait_for(block, &after);
    assert_eq!(a.snapshot(block), b.snapshot(block));
}

#[test]
fn input_round_trips_while_the_pty_floods() {
    let path = start_server();
    let mut a = Client::connect(&path);
    a.parse = false;
    let block = block_for(&mut a);
    run_and_wait(&mut a, block, 41);

    // A background job floods the PTY for the rest of the test; the shell in
    // the foreground must still read and answer a command.
    if cfg!(windows) {
        a.input(
            block,
            b"start /b cmd /q /c \"for /l %i in (0,1,999999) do @echo flood-%i\"\r",
        );
    } else {
        a.input(
            block,
            b"(i=0; while [ $i -lt 1000000 ]; do echo flood-$i; i=$((i+1)); done) &\r",
        );
    }
    a.wait_for(block, "flood-1000");
    let started = Instant::now();
    let marker = run_and_wait(&mut a, block, 43);
    let elapsed = started.elapsed();
    eprintln!("round trip under flood: {elapsed:?}");
    if cfg!(unix) {
        a.input(block, b"kill $!\r");
    }
    // The server outlives the test, so the flood must not.
    a.request(|req| Message::CloseBlock { req, block });
    // Measured at 5-70 ms in a debug build; the bound leaves room for a busy
    // CI runner. Without the pump's yield this took seconds.
    assert!(
        elapsed < Duration::from_secs(1),
        "{marker} took {elapsed:?} under flood"
    );
}

#[test]
fn a_v3_checkpoint_client_is_refused_attach() {
    // RTCP v3 carries no colour provenance, so a v3-only peer is neither
    // granted a checkpoint capability nor sent one: both sides fail closed.
    let path = start_server();
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let mut old = Client::connect_with(&path, Features::CHECKPOINT_V3, None);
    assert!(!old.features.contains(Features::CHECKPOINT_V3));
    assert!(!old.features.contains(Features::CHECKPOINT_V4));
    let answer = old.answer(|req| Message::Attach { req, block });
    assert!(
        matches!(answer.last(), Some(Message::Failed { reason, .. }) if reason.contains("requires feature bits")),
        "{answer:?}"
    );
    assert!(a.features.contains(Features::CHECKPOINT_V4));
}

#[test]
fn input_without_the_lease_is_refused() {
    let path = start_server();
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let mut b = Client::connect(&path);
    b.request(|req| Message::Attach { req, block });

    b.send(&Message::Input {
        req: 99,
        block,
        data: b"echo nope\r".to_vec(),
    });
    loop {
        if let Message::Failed {
            req: 99, reason, ..
        } = b.recv()
        {
            assert!(reason.contains("does not hold the input lease"), "{reason}");
            break;
        }
    }
}

/// Linux only: it relies on the exact output volume, which ConPTY's
/// re-rendering changes, and the budget it covers is the same on every OS.
#[cfg(target_os = "linux")]
#[test]
fn reattach_after_more_than_four_mib_of_real_pty_output() {
    let path = start_server();
    let mut a = Client::connect(&path);
    a.parse = false;
    let block = block_for(&mut a);
    // A verbose command must not consume an attach lifetime budget. The test
    // uses local /dev/zero only and waits for actual PTY output completion.
    a.input(
        block,
        b"head -c 5242880 /dev/zero | tr '\\000' x; printf '\\nBIG_%s\\n' DONE\r",
    );
    a.wait_for(block, "BIG_DONE");
    let mut b = Client::connect(&path);
    let mut c = Client::connect(&path);
    c.request(|req| Message::Attach { req, block });
    for n in 100..103 {
        b.request(|req| Message::Attach { req, block });
        let marker = run_and_wait(&mut a, block, n);
        b.wait_for(block, &marker);
        c.wait_for(block, &marker);
        assert_eq!(b.snapshot(block), c.snapshot(block));
        b.request(|req| Message::Detach { req });
    }
    a.request(|req| Message::CloseBlock { req, block });
}
