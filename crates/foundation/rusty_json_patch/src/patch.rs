//! RFC 6902 JSON Patch: the six operations, their wire form, and atomic
//! application.

use crate::pointer::Slot;
use crate::{Error, Pointer, Result};
use alloc::string::ToString;
use alloc::vec::Vec;
use rusty_json::Value;

/// One RFC 6902 operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Insert into an object (replacing any existing member) or an array
    /// (shifting later elements; `-` appends).
    Add {
        /// Target location.
        path: Pointer,
        /// Value to insert.
        value: Value,
    },
    /// Remove the value at `path`, which must exist.
    Remove {
        /// Target location.
        path: Pointer,
    },
    /// Replace the value at `path`, which must exist.
    Replace {
        /// Target location.
        path: Pointer,
        /// New value.
        value: Value,
    },
    /// Remove at `from`, then add at `path`.
    Move {
        /// Source location.
        from: Pointer,
        /// Target location.
        path: Pointer,
    },
    /// Add at `path` a copy of the value at `from`.
    Copy {
        /// Source location.
        from: Pointer,
        /// Target location.
        path: Pointer,
    },
    /// Fail unless the value at `path` equals `value`.
    Test {
        /// Location to check.
        path: Pointer,
        /// Expected value.
        value: Value,
    },
}

/// An ordered list of operations: an RFC 6902 patch document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch(pub Vec<Op>);

impl Patch {
    /// Parses a patch document (a JSON array of operation objects).
    pub fn from_value(value: &Value) -> Result<Self> {
        let Some(items) = value.as_array() else {
            return Err(Error::InvalidPatch("patch is not an array".into()));
        };
        items
            .iter()
            .map(op_from_value)
            .collect::<Result<Vec<_>>>()
            .map(Patch)
    }

    /// The wire form, ready for `Value::to_json_string`.
    pub fn to_value(&self) -> Value {
        Value::Array(self.0.iter().map(op_to_value).collect())
    }

    /// True when there is nothing to apply.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Applies every operation in order. Atomic: on any error `doc` is
    /// left exactly as it was.
    pub fn apply(&self, doc: &mut Value) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let mut work = doc.clone();
        for op in &self.0 {
            apply_op(&mut work, op)?;
        }
        *doc = work;
        Ok(())
    }
}

fn apply_op(doc: &mut Value, op: &Op) -> Result<()> {
    match op {
        Op::Add { path, value } => add(doc, path, value.clone()),
        Op::Remove { path } => remove(doc, path).map(|_| ()),
        Op::Replace { path, value } => {
            let target = path
                .resolve_mut(doc)
                .ok_or_else(|| Error::PathNotFound(path.to_string()))?;
            *target = value.clone();
            Ok(())
        }
        Op::Move { from, path } => {
            if from.is_proper_prefix_of(path) {
                return Err(Error::MoveIntoSelf(path.to_string()));
            }
            if from == path {
                return Ok(());
            }
            let value = remove(doc, from)?;
            add(doc, path, value)
        }
        Op::Copy { from, path } => {
            let value = from
                .resolve(doc)
                .cloned()
                .ok_or_else(|| Error::PathNotFound(from.to_string()))?;
            add(doc, path, value)
        }
        Op::Test { path, value } => {
            let actual = path
                .resolve(doc)
                .ok_or_else(|| Error::PathNotFound(path.to_string()))?;
            if actual == value {
                Ok(())
            } else {
                Err(Error::TestFailed(path.to_string()))
            }
        }
    }
}

fn add(doc: &mut Value, path: &Pointer, value: Value) -> Result<()> {
    if path.is_root() {
        *doc = value;
        return Ok(());
    }
    match path.slot(doc, true)? {
        Slot::Key(map, key) => {
            map.insert(key, value);
        }
        Slot::Index(items, index) => items.insert(index, value),
    }
    Ok(())
}

fn remove(doc: &mut Value, path: &Pointer) -> Result<Value> {
    if path.is_root() {
        return Ok(core::mem::replace(doc, Value::Null));
    }
    match path.slot(doc, false)? {
        Slot::Key(map, key) => map
            .remove(key.as_str())
            .ok_or_else(|| Error::PathNotFound(path.to_string())),
        Slot::Index(items, index) => Ok(items.remove(index)),
    }
}

fn op_from_value(value: &Value) -> Result<Op> {
    let field = |name: &str| -> Result<&Value> {
        value
            .get(name)
            .ok_or_else(|| Error::InvalidPatch(alloc::format!("operation lacks {name:?}")))
    };
    let pointer = |name: &str| -> Result<Pointer> {
        let text = field(name)?
            .as_str()
            .ok_or_else(|| Error::InvalidPatch(alloc::format!("{name:?} is not a string")))?;
        Pointer::parse(text)
    };
    let kind = field("op")?
        .as_str()
        .ok_or_else(|| Error::InvalidPatch("\"op\" is not a string".into()))?;
    let path = pointer("path")?;
    Ok(match kind {
        "add" => Op::Add {
            path,
            value: field("value")?.clone(),
        },
        "remove" => Op::Remove { path },
        "replace" => Op::Replace {
            path,
            value: field("value")?.clone(),
        },
        "move" => Op::Move {
            from: pointer("from")?,
            path,
        },
        "copy" => Op::Copy {
            from: pointer("from")?,
            path,
        },
        "test" => Op::Test {
            path,
            value: field("value")?.clone(),
        },
        other => return Err(Error::InvalidPatch(alloc::format!("unknown op {other:?}"))),
    })
}

fn op_to_value(op: &Op) -> Value {
    let mut out = Value::object();
    let (kind, path): (&str, &Pointer) = match op {
        Op::Add { path, .. } => ("add", path),
        Op::Remove { path } => ("remove", path),
        Op::Replace { path, .. } => ("replace", path),
        Op::Move { path, .. } => ("move", path),
        Op::Copy { path, .. } => ("copy", path),
        Op::Test { path, .. } => ("test", path),
    };
    out.insert("op", kind);
    out.insert("path", path.to_string());
    match op {
        Op::Add { value, .. } | Op::Replace { value, .. } | Op::Test { value, .. } => {
            out.insert("value", value.clone());
        }
        Op::Move { from, .. } | Op::Copy { from, .. } => {
            out.insert("from", from.to_string());
        }
        Op::Remove { .. } => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_json::json;

    fn apply(doc: Value, patch: Value) -> Result<Value> {
        let mut doc = doc;
        Patch::from_value(&patch)?.apply(&mut doc)?;
        Ok(doc)
    }

    /// RFC 6902 Appendix A, in order: A.1 through A.16.
    #[test]
    fn rfc_6902_appendix_a() {
        let ok: &[(Value, Value, Value)] = &[
            (
                json!({"foo": "bar"}),
                json!([{"op": "add", "path": "/baz", "value": "qux"}]),
                json!({"baz": "qux", "foo": "bar"}),
            ),
            (
                json!({"foo": ["bar", "baz"]}),
                json!([{"op": "add", "path": "/foo/1", "value": "qux"}]),
                json!({"foo": ["bar", "qux", "baz"]}),
            ),
            (
                json!({"baz": "qux", "foo": "bar"}),
                json!([{"op": "remove", "path": "/baz"}]),
                json!({"foo": "bar"}),
            ),
            (
                json!({"foo": ["bar", "qux", "baz"]}),
                json!([{"op": "remove", "path": "/foo/1"}]),
                json!({"foo": ["bar", "baz"]}),
            ),
            (
                json!({"baz": "qux", "foo": "bar"}),
                json!([{"op": "replace", "path": "/baz", "value": "boo"}]),
                json!({"baz": "boo", "foo": "bar"}),
            ),
            (
                json!({"foo": {"bar": "baz", "waldo": "fred"}, "qux": {"corge": "grault"}}),
                json!([{"op": "move", "from": "/foo/waldo", "path": "/qux/thud"}]),
                json!({"foo": {"bar": "baz"}, "qux": {"corge": "grault", "thud": "fred"}}),
            ),
            (
                json!({"foo": ["all", "grass", "cows", "eat"]}),
                json!([{"op": "move", "from": "/foo/1", "path": "/foo/3"}]),
                json!({"foo": ["all", "cows", "eat", "grass"]}),
            ),
            (
                json!({"baz": "qux", "foo": ["a", 2, "c"]}),
                json!([
                    {"op": "test", "path": "/baz", "value": "qux"},
                    {"op": "test", "path": "/foo/1", "value": 2}
                ]),
                json!({"baz": "qux", "foo": ["a", 2, "c"]}),
            ),
            (
                json!({"foo": "bar"}),
                json!([{"op": "add", "path": "/child", "value": {"grandchild": {}}}]),
                json!({"foo": "bar", "child": {"grandchild": {}}}),
            ),
            (
                json!({"foo": "bar"}),
                json!([{"op": "add", "path": "/baz", "value": "qux", "xyz": 123}]),
                json!({"foo": "bar", "baz": "qux"}),
            ),
            (
                json!({"foo": ["bar"]}),
                json!([{"op": "add", "path": "/foo/-", "value": ["abc", "def"]}]),
                json!({"foo": ["bar", ["abc", "def"]]}),
            ),
            (
                json!({"/": 9, "~1": 10}),
                json!([{"op": "test", "path": "/~01", "value": 10}]),
                json!({"/": 9, "~1": 10}),
            ),
        ];
        for (i, (doc, patch, expected)) in ok.iter().enumerate() {
            assert_eq!(
                apply(doc.clone(), patch.clone()).unwrap(),
                *expected,
                "case {i}"
            );
        }

        // A.9: a failed test aborts the patch.
        assert!(matches!(
            apply(
                json!({"baz": "qux"}),
                json!([{"op": "test", "path": "/baz", "value": "bar"}])
            ),
            Err(Error::TestFailed(_))
        ));
        // A.12: adding to a non-existent target.
        assert!(matches!(
            apply(
                json!({"foo": "bar"}),
                json!([{"op": "add", "path": "/baz/bat", "value": "qux"}])
            ),
            Err(Error::PathNotFound(_))
        ));
        // A.13: an invalid patch document.
        assert!(matches!(
            apply(json!({}), json!([{"path": "/x", "value": 1}])),
            Err(Error::InvalidPatch(_))
        ));
        // A.15: `~01` is not `/`, so the test fails on value.
        assert!(matches!(
            apply(
                json!({"/": 9, "~1": 10}),
                json!([{"op": "test", "path": "/~01", "value": 9}])
            ),
            Err(Error::TestFailed(_))
        ));
        // A.16: an array index past the end.
        assert!(matches!(
            apply(
                json!({"foo": ["bar", "baz"]}),
                json!([{"op": "add", "path": "/foo/5", "value": 1}])
            ),
            Err(Error::InvalidIndex(_))
        ));
    }

    #[test]
    fn apply_is_atomic() {
        let original = json!({"a": 1, "b": [1, 2]});
        let mut doc = original.clone();
        let patch = Patch::from_value(&json!([
            {"op": "replace", "path": "/a", "value": 2},
            {"op": "remove", "path": "/b/0"},
            {"op": "remove", "path": "/missing"}
        ]))
        .unwrap();
        assert!(patch.apply(&mut doc).is_err());
        assert_eq!(
            doc, original,
            "a failing patch leaves the document untouched"
        );
    }

    #[test]
    fn root_and_edge_operations() {
        let mut doc = json!({"a": 1});
        Patch::from_value(&json!([{"op": "replace", "path": "", "value": [1]}]))
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(doc, json!([1]));

        Patch::from_value(&json!([{"op": "add", "path": "/1", "value": 2}]))
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(doc, json!([1, 2]), "index == len appends");

        assert!(matches!(
            Patch::from_value(&json!([{"op": "remove", "path": "/2"}]))
                .unwrap()
                .apply(&mut doc),
            Err(Error::InvalidIndex(_))
        ));
        assert!(matches!(
            Patch::from_value(&json!([{"op": "remove", "path": "/-"}]))
                .unwrap()
                .apply(&mut doc),
            Err(Error::InvalidIndex(_))
        ));
        assert!(matches!(
            Patch::from_value(&json!([{"op": "move", "from": "/0", "path": "/0/x"}]))
                .unwrap()
                .apply(&mut json!([{"x": 1}])),
            Err(Error::MoveIntoSelf(_))
        ));

        let mut nested = json!({"a": {"b": 1}});
        Patch::from_value(&json!([{"op": "copy", "from": "/a", "path": "/c"}]))
            .unwrap()
            .apply(&mut nested)
            .unwrap();
        assert_eq!(nested, json!({"a": {"b": 1}, "c": {"b": 1}}));
    }

    #[test]
    fn wire_round_trip() {
        let wire = json!([
            {"op": "add", "path": "/a", "value": 1},
            {"op": "remove", "path": "/b"},
            {"op": "replace", "path": "/c", "value": null},
            {"op": "move", "from": "/d", "path": "/e"},
            {"op": "copy", "from": "/f", "path": "/g"},
            {"op": "test", "path": "/h~0i~1j", "value": [true]}
        ]);
        let patch = Patch::from_value(&wire).unwrap();
        assert_eq!(patch.to_value(), wire);
    }
}
