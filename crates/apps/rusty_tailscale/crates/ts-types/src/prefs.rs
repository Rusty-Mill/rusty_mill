//! Preferences types, mirroring Go's `ipn.Prefs` / `ipn.MaskedPrefs` at
//! v1.86. Only the fields the Phase-1 CLI reads are modeled; unknown fields
//! are ignored on decode.

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

use crate::{IpPrefix, null_default};

/// Response of `GET /localapi/v0/prefs` (subset of `ipn.Prefs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Prefs {
    #[serde(rename = "ControlURL")]
    pub control_url: String,
    pub route_all: bool,
    #[serde(rename = "ExitNodeID")]
    pub exit_node_id: String,
    #[serde(rename = "ExitNodeIP")]
    pub exit_node_ip: String,
    #[serde(rename = "CorpDNS")]
    pub corp_dns: bool,
    #[serde(rename = "RunSSH")]
    pub run_ssh: bool,
    pub want_running: bool,
    pub logged_out: bool,
    pub shields_up: bool,
    #[serde(deserialize_with = "null_default")]
    pub advertise_tags: Vec<String>,
    pub hostname: String,
    #[serde(deserialize_with = "null_default")]
    pub advertise_routes: Vec<IpPrefix>,
    #[serde(rename = "NoSNAT")]
    pub no_snat: bool,
    pub netfilter_mode: i64,
}

/// A partial prefs edit for `PATCH /localapi/v0/prefs` (`ipn.MaskedPrefs`).
///
/// Go's encoding pairs each field with a `<Field>Set` bool; only fields whose
/// mask flag is true are applied. Only the fields the CLI edits are modeled —
/// extend as commands grow.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaskedPrefs {
    pub want_running: Option<bool>,
}

impl Serialize for MaskedPrefs {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        if let Some(v) = self.want_running {
            map.serialize_entry("WantRunning", &v)?;
            map.serialize_entry("WantRunningSet", &true)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for MaskedPrefs {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// The wire form: each field paired with a `<Field>Set` mask flag; a
        /// field is applied only when its flag is true. Unknown fields ignored.
        #[derive(Default, Deserialize)]
        #[serde(default, rename_all = "PascalCase")]
        struct Wire {
            want_running: bool,
            want_running_set: bool,
        }
        let w = Wire::deserialize(d)?;
        Ok(MaskedPrefs {
            want_running: w.want_running_set.then_some(w.want_running),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::MaskedPrefs;

    #[test]
    fn masked_prefs_encoding() {
        let mp = MaskedPrefs {
            want_running: Some(false),
        };
        assert_eq!(
            serde_json::to_string(&mp).unwrap(),
            r#"{"WantRunning":false,"WantRunningSet":true}"#
        );
        assert_eq!(
            serde_json::to_string(&MaskedPrefs::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn masked_prefs_round_trip() {
        for mp in [
            MaskedPrefs {
                want_running: Some(true),
            },
            MaskedPrefs {
                want_running: Some(false),
            },
            MaskedPrefs::default(),
        ] {
            let json = serde_json::to_string(&mp).unwrap();
            let back: MaskedPrefs = serde_json::from_str(&json).unwrap();
            assert_eq!(mp, back, "round-trip via {json}");
        }
        // An unset field (mask flag absent) decodes to None.
        let only_value: MaskedPrefs = serde_json::from_str(r#"{"WantRunning":true}"#).unwrap();
        assert_eq!(only_value.want_running, None);
    }
}
