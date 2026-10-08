//! A loader for Project Wycheproof `testvectors_v1` JSON files.
//!
//! Every test is flattened into a [`Case`] that sees both its own fields and
//! its group's (a group holds the key, a test holds the message and result).
//! Accessors return `Result<_, String>` naming the missing field, so a schema
//! surprise fails with a message instead of a panic deep in a helper.

use std::rc::Rc;

use rusty_json::Value;

/// What the vector's author says the implementation must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Must be accepted / produce the expected output.
    Valid,
    /// Must be rejected.
    Invalid,
    /// Either outcome is defensible; the file's flags say why.
    Acceptable,
}

/// One test vector with its group's fields in scope.
#[derive(Debug, Clone)]
pub struct Case {
    /// The `tcId`.
    pub id: u64,
    /// The vector's `comment`.
    pub comment: String,
    /// The `result` field.
    pub verdict: Verdict,
    /// The `flags` list.
    pub flags: Vec<String>,
    group: Rc<Value>,
    test: Value,
}

impl Case {
    fn field(&self, key: &str) -> Result<&Value, String> {
        self.test
            .get(key)
            .or_else(|| self.group.get(key))
            .ok_or_else(|| format!("tcId {}: no field {key:?}", self.id))
    }

    /// A string field from the test, else its group.
    pub fn str(&self, key: &str) -> Result<&str, String> {
        self.field(key)?
            .as_str()
            .ok_or_else(|| format!("tcId {}: {key:?} is not a string", self.id))
    }

    /// A hex-string field decoded to bytes.
    pub fn hex(&self, key: &str) -> Result<Vec<u8>, String> {
        rusty_hex::decode(self.str(key)?)
            .map_err(|e| format!("tcId {}: {key:?} is not hex: {e:?}", self.id))
    }

    /// A non-negative integer field.
    pub fn uint(&self, key: &str) -> Result<u64, String> {
        self.field(key)?
            .as_u64()
            .ok_or_else(|| format!("tcId {}: {key:?} is not an unsigned integer", self.id))
    }

    /// Whether the vector carries `flag`.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

/// Parses a Wycheproof file into its cases, in file order.
pub fn cases(json: &str) -> Result<Vec<Case>, String> {
    let doc = Value::parse(json).map_err(|e| e.to_string())?;
    let groups = doc
        .get("testGroups")
        .and_then(Value::as_array)
        .ok_or("no testGroups array")?;
    let mut out = Vec::new();
    for group in groups {
        let shared = Rc::new(group.clone());
        let tests = group
            .get("tests")
            .and_then(Value::as_array)
            .ok_or("group without tests")?;
        for test in tests {
            out.push(parse_case(&shared, test)?);
        }
    }
    Ok(out)
}

fn parse_case(group: &Rc<Value>, test: &Value) -> Result<Case, String> {
    let id = test
        .get("tcId")
        .and_then(Value::as_u64)
        .ok_or("test without tcId")?;
    let verdict = match test.get("result").and_then(Value::as_str) {
        Some("valid") => Verdict::Valid,
        Some("invalid") => Verdict::Invalid,
        Some("acceptable") => Verdict::Acceptable,
        other => return Err(format!("tcId {id}: unknown result {other:?}")),
    };
    let flags = test
        .get("flags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| f.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(Case {
        id,
        comment: test
            .get("comment")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        verdict,
        flags,
        group: Rc::clone(group),
        test: test.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"{"testGroups":[{"keySize":128,"tests":[
        {"tcId":1,"comment":"c","flags":["F"],"msg":"0aff","result":"valid"},
        {"tcId":2,"msg":"zz","result":"invalid"}]}]}"#;

    #[test]
    fn flattens_group_fields_into_cases() {
        let cs = cases(DOC).unwrap();
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].uint("keySize").unwrap(), 128);
        assert_eq!(cs[0].hex("msg").unwrap(), vec![0x0a, 0xff]);
        assert_eq!(cs[0].verdict, Verdict::Valid);
        assert!(cs[0].has_flag("F") && !cs[1].has_flag("F"));
    }

    #[test]
    fn reports_missing_and_malformed_fields() {
        let cs = cases(DOC).unwrap();
        assert!(cs[1].hex("msg").unwrap_err().contains("not hex"));
        assert!(cs[0].str("nope").unwrap_err().contains("no field"));
    }

    #[test]
    fn rejects_unknown_result() {
        let doc = r#"{"testGroups":[{"tests":[{"tcId":9,"result":"maybe"}]}]}"#;
        assert!(cases(doc).unwrap_err().contains("unknown result"));
    }

    #[test]
    fn rejects_missing_groups() {
        assert!(cases("{}").is_err());
    }
}
