//! RFC 3339 timestamp (de)serialization matching the A2A spec's data type
//! conventions (Section 5.6.1): `google.protobuf.Timestamp` fields are
//! represented in JSON as millisecond-precision, UTC ("Z"-suffixed) ISO 8601
//! strings.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serializer};

/// For `Option<DateTime<Utc>>` fields (the common case: all A2A timestamp
/// fields are optional in the proto).
pub mod option {
    use super::*;

    pub fn serialize<S>(value: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(dt) => serializer.serialize_some(&dt.to_rfc3339_opts(SecondsFormat::Millis, true)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: Option<String> = Option::deserialize(deserializer)?;
        match raw {
            Some(s) => DateTime::parse_from_rfc3339(&s)
                .map(|dt| Some(dt.with_timezone(&Utc)))
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}
