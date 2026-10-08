#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Events and responses round-trip through rusty_serde's JSON, since the log
//! and the wire both depend on it.
mod common;
use common::*;
use rusty_bbp::*;

#[test]
fn every_logged_event_round_trips_through_json() {
    let mut fx = Fx::new();
    fx.reach_approved();
    let log = fx.d.store.events(&fx.st().task, 0).expect("log");
    assert!(log.len() > 40);
    for e in &log {
        let json = rusty_serde::json::to_string(e).expect("serialize");
        let back: Event = rusty_serde::json::from_str(&json).expect("deserialize");
        assert_eq!(&back, e, "{json}");
    }
    let card = fx.st().card();
    let json = rusty_serde::json::to_string(&card).expect("card");
    let back: Card = rusty_serde::json::from_str(&json).expect("card back");
    assert_eq!(back, card);
}

#[test]
fn sha256_is_hex_on_the_wire() {
    let d = Sha256::of(b"abc");
    let json = rusty_serde::json::to_string(&d).expect("json");
    assert_eq!(json, format!("\"{}\"", d.hex()));
    assert_eq!(
        d.hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let back: Sha256 = rusty_serde::json::from_str(&json).expect("back");
    assert_eq!(back, d);
}
