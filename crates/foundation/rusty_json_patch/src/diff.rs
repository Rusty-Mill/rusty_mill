//! The patch that turns one document into another.

use crate::{Op, Patch, Pointer};
use alloc::string::ToString;
use alloc::vec::Vec;
use rusty_json::Value;

/// Computes a patch such that `diff(a, b).apply(a) == b`.
///
/// Objects are compared member by member; arrays element by element with
/// the tail added or removed. Scalars and type changes become a single
/// `replace`. The patch is minimal for objects, and for arrays whose
/// elements changed in place; an insertion in the middle of an array
/// produces replaces from that index on (a longest-common-subsequence
/// diff is not worth its size for the state deltas this is built for).
pub fn diff(from: &Value, to: &Value) -> Patch {
    let mut ops = Vec::new();
    diff_into(from, to, &Pointer::default(), &mut ops);
    Patch(ops)
}

fn diff_into(from: &Value, to: &Value, path: &Pointer, ops: &mut Vec<Op>) {
    match (from, to) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, old) in a.iter() {
                match b.get(key.as_str()) {
                    Some(new) => diff_into(old, new, &path.child(key.clone()), ops),
                    None => ops.push(Op::Remove {
                        path: path.child(key.clone()),
                    }),
                }
            }
            for (key, new) in b.iter() {
                if !a.contains_key(key.as_str()) {
                    ops.push(Op::Add {
                        path: path.child(key.clone()),
                        value: new.clone(),
                    });
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            let common = a.len().min(b.len());
            for i in 0..common {
                diff_into(&a[i], &b[i], &path.child(i.to_string()), ops);
            }
            // Remove from the end so earlier indices stay valid.
            for i in (common..a.len()).rev() {
                ops.push(Op::Remove {
                    path: path.child(i.to_string()),
                });
            }
            for item in &b[common..] {
                ops.push(Op::Add {
                    path: path.child("-"),
                    value: item.clone(),
                });
            }
        }
        _ if from == to => {}
        _ => ops.push(Op::Replace {
            path: path.clone(),
            value: to.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_json::json;

    fn round_trip(from: Value, to: Value) -> Patch {
        let patch = diff(&from, &to);
        let mut doc = from;
        patch.apply(&mut doc).unwrap();
        assert_eq!(doc, to);
        patch
    }

    #[test]
    fn identical_documents_yield_no_ops() {
        assert!(diff(&json!({"a": [1, {"b": 2}]}), &json!({"a": [1, {"b": 2}]})).is_empty());
    }

    #[test]
    fn objects_add_remove_and_recurse() {
        let patch = round_trip(
            json!({"keep": 1, "drop": 2, "nest": {"x": 1}}),
            json!({"keep": 1, "new": 3, "nest": {"x": 2}}),
        );
        assert_eq!(patch.0.len(), 3);
    }

    #[test]
    fn arrays_grow_shrink_and_change_in_place() {
        round_trip(json!([1, 2, 3]), json!([1, 5]));
        round_trip(json!([1]), json!([1, 2, 3]));
        round_trip(json!([]), json!([{"a": 1}]));
        round_trip(json!([[1, 2], [3]]), json!([[1], [3, 4]]));
    }

    #[test]
    fn type_changes_replace_whole_values() {
        let patch = round_trip(json!({"a": [1]}), json!({"a": {"b": 1}}));
        assert_eq!(patch.0.len(), 1);
        round_trip(json!(1), json!("one"));
        round_trip(json!(null), json!({"a": 1}));
    }

    #[test]
    fn keys_needing_escapes_survive() {
        round_trip(json!({"a/b": 1, "~": 2}), json!({"a/b": 3}));
    }
}
