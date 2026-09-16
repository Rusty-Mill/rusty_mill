//! Regression coverage for the round-7 finding: `json::de`/`ron::de`
//! (`SeqWalker`/`MapWalker::next_element`/`next_key`, plus RON's
//! `Some(...)`/tagged-newtype-variant wrappers and `skip_seq_like`/
//! `skip_map_like`) and the generic `Value` re-drive machinery in
//! `value.rs` all recursed straight back into `Deserialize` with no depth
//! tracking, so sufficiently deep untrusted input overflowed the native
//! stack instead of producing an error.

use rusty_serde::json::{self, Value};
use rusty_serde::{ron, Deserialize, Serialize};

/// Comfortably past both formats' `MAX_NESTING_DEPTH`/`MAX_VALUE_DEPTH`
/// (128) but far short of anything that would itself overflow the
/// *test's own* stack while building the input (these are built
/// iteratively, never recursively).
const DEEP: usize = 50_000;

/// Depth used for pre-built (rather than parsed) `Value` trees below.
/// `Value` has no custom `Drop`, so its ordinary derived recursive drop
/// overflows the stack on its own past roughly 2,000-5,000 levels,
/// independently of anything under test here; this stays far past
/// `MAX_VALUE_DEPTH` (128, so the guard is genuinely exercised) while
/// staying well clear of that unrelated threshold, so the fixture itself
/// - and the still-nested remainder abandoned when the guard errors out
///   partway through - can be safely dropped at the end of the test.
const VALUE_TEST_DEPTH: usize = 1_000;

#[test]
fn json_deeply_nested_array_is_rejected_instead_of_overflowing_stack() {
    let mut text = "[".repeat(DEEP);
    text.push('1');
    text.push_str(&"]".repeat(DEEP));

    let err = json::from_str::<Value>(&text).unwrap_err();
    assert!(
        err.to_string().contains("nesting depth"),
        "expected a nesting-depth error, got: {err}"
    );
}

#[test]
fn ron_deeply_nested_array_is_rejected_instead_of_overflowing_stack() {
    let mut text = "[".repeat(DEEP);
    text.push('1');
    text.push_str(&"]".repeat(DEEP));

    let err = ron::from_str::<Value>(&text).unwrap_err();
    assert!(
        err.to_string().contains("nesting depth"),
        "expected a nesting-depth error, got: {err}"
    );
}

#[test]
fn ron_deeply_nested_some_is_rejected_instead_of_overflowing_stack() {
    // `Some(...)` is RON's own wrapper syntax, independent of any
    // array/object bracket nesting - a distinct recursion vector from the
    // bracket-nesting case above.
    let mut text = "Some(".repeat(DEEP);
    text.push('1');
    text.push_str(&")".repeat(DEEP));

    let err = ron::from_str::<Value>(&text).unwrap_err();
    assert!(
        err.to_string().contains("nesting depth"),
        "expected a nesting-depth error, got: {err}"
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Tree {
    Leaf,
    Node(Box<Tree>),
}

#[test]
fn ron_deeply_nested_newtype_variant_is_rejected_instead_of_overflowing_stack() {
    // A recursive enum's newtype variant (`Node(Box<Tree>)`) is parsed via
    // `VariantDataAccess::newtype_variant`, which recurses through a bare
    // `(...)` wrapper rather than `parse_seq_bracket`.
    let mut text = "Node(".repeat(DEEP);
    text.push_str("Leaf");
    text.push_str(&")".repeat(DEEP));

    let err = ron::from_str::<Tree>(&text).unwrap_err();
    assert!(
        err.to_string().contains("nesting depth"),
        "expected a nesting-depth error, got: {err}"
    );
}

#[test]
fn value_deeply_nested_in_memory_tree_is_rejected_instead_of_overflowing_stack() {
    // Built iteratively (not recursively) so *constructing* the fixture
    // doesn't itself overflow the test's stack. Stands in for a `Value`
    // tree reaching `ValueDeserializer`/`from_value` from anywhere other
    // than these two text parsers, both of which already cap nesting
    // while parsing - `ValueSeqAccess`/`ValueMapAccess` otherwise recurse
    // independently of either format's own guard.
    let mut deep = Value::Seq(Vec::new());
    for _ in 0..VALUE_TEST_DEPTH {
        deep = Value::Seq(vec![deep]);
    }

    let err = json::from_value::<Value>(deep).unwrap_err();
    assert!(
        err.to_string().contains("nesting depth"),
        "expected a nesting-depth error, got: {err}"
    );
}
