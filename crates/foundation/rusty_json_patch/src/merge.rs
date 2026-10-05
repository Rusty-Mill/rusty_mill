//! RFC 7386 JSON Merge Patch.

use rusty_json::Value;

/// Applies `patch` to `target` per RFC 7386 §2: an object patch merges
/// member by member, `null` members delete, and any non-object patch
/// replaces the target outright. Always succeeds.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(changes) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::object();
    }
    let Some(map) = target.as_object_mut() else {
        return;
    };
    for (key, change) in changes.iter() {
        if change.is_null() {
            map.remove(key.as_str());
            continue;
        }
        let slot = map.entry(key.clone()).or_insert(Value::Null);
        merge_patch(slot, change);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_json::json;

    /// RFC 7386 Appendix A test cases.
    #[test]
    fn rfc_7386_appendix_a() {
        let cases: &[(Value, Value, Value)] = &[
            (json!({"a": "b"}), json!({"a": "c"}), json!({"a": "c"})),
            (
                json!({"a": "b"}),
                json!({"b": "c"}),
                json!({"a": "b", "b": "c"}),
            ),
            (json!({"a": "b"}), json!({"a": null}), json!({})),
            (
                json!({"a": "b", "b": "c"}),
                json!({"a": null}),
                json!({"b": "c"}),
            ),
            (json!({"a": ["b"]}), json!({"a": "c"}), json!({"a": "c"})),
            (json!({"a": "c"}), json!({"a": ["b"]}), json!({"a": ["b"]})),
            (
                json!({"a": {"b": "c"}}),
                json!({"a": {"b": "d", "c": null}}),
                json!({"a": {"b": "d"}}),
            ),
            (
                json!({"a": [{"b": "c"}]}),
                json!({"a": [1]}),
                json!({"a": [1]}),
            ),
            (json!(["a", "b"]), json!(["c", "d"]), json!(["c", "d"])),
            (json!({"a": "b"}), json!(["c"]), json!(["c"])),
            (json!({"a": "foo"}), json!(null), json!(null)),
            (json!({"a": "foo"}), json!("bar"), json!("bar")),
            (
                json!({"e": null}),
                json!({"a": 1}),
                json!({"e": null, "a": 1}),
            ),
            (
                json!([1, 2]),
                json!({"a": "b", "c": null}),
                json!({"a": "b"}),
            ),
            (
                json!({}),
                json!({"a": {"bb": {"ccc": null}}}),
                json!({"a": {"bb": {}}}),
            ),
        ];
        for (i, (target, patch, expected)) in cases.iter().enumerate() {
            let mut doc = target.clone();
            merge_patch(&mut doc, patch);
            assert_eq!(doc, *expected, "case {i}");
        }
    }
}
