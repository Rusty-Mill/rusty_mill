//! Identifier newtypes mirroring `tailcfg.UserID` and `tailcfg.StableNodeID`.

use std::fmt;

/// A user ID (`tailcfg.UserID`, an int64 in Go).
///
/// Go marshals it as a JSON number in struct fields but as a *string* when it
/// is a map key (`Status.User`), so deserialization accepts both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserID(pub i64);

impl fmt::Display for UserID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl serde::Serialize for UserID {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for UserID {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = UserID;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an integer user ID (number or string)")
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<UserID, E> {
                Ok(UserID(v))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<UserID, E> {
                i64::try_from(v)
                    .map(UserID)
                    .map_err(|_| E::custom("user ID out of i64 range"))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<UserID, E> {
                v.parse().map(UserID).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

/// A node's stable string ID (`tailcfg.StableNodeID`), e.g. `"1"` under
/// Headscale or `"nTKzp4…"` under the hosted control plane.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct StableNodeID(pub String);

impl fmt::Display for StableNodeID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_id_number_and_string() {
        assert_eq!(serde_json::from_str::<UserID>("1").unwrap(), UserID(1));
        assert_eq!(serde_json::from_str::<UserID>("\"1\"").unwrap(), UserID(1));
        assert!(serde_json::from_str::<UserID>("\"x\"").is_err());
    }

    #[test]
    fn user_id_as_map_key() {
        use std::collections::BTreeMap;
        let m: BTreeMap<UserID, String> = serde_json::from_str(r#"{"7": "a"}"#).unwrap();
        assert_eq!(m.get(&UserID(7)), Some(&"a".to_string()));
        // serde_json stringifies integer-like keys on serialize.
        assert_eq!(serde_json::to_string(&m).unwrap(), r#"{"7":"a"}"#);
    }
}
