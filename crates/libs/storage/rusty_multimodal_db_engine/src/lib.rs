//! The embedded, mmap-backed generic record store, extracted from
//! `rusty_multimodal_db` (its ADR-0124) so a crate outside that product can
//! embed it: the workspace forbids one apps crate depending on another's,
//! and `rusty_remind_me`'s hub embeds this (its ADR-0021).
//!
//! [`generic`] is the store: composable layers ([`generic::store`]) over a
//! durable core ([`generic::GenericMmapStore`]), wrapped for sharing by
//! [`generic::GenericProductionStore`]. [`durability`] holds the error type
//! and blob helpers it shares with `rusty_multimodal_db`'s own durability
//! variants, and [`codec`] the one bincode configuration every on-disk
//! format here uses. `rusty_multimodal_db` re-exports all three under the
//! paths they had before the move.

pub mod codec;
pub mod durability;
pub mod generic;

#[cfg(test)]
mod test_support;
