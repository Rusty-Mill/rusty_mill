//! Single-key map enums.
//!
//! Upstream writes variant choices as a map with one key — `{pathPrefix: /v1}`,
//! `{stdio: {...}}`, `{key: sk-...}`. That is what serde calls an externally
//! tagged enum, but `serde_yaml` 0.9 encodes those as YAML *tags* (`!PathPrefix
//! /v1`) and rejects the map form outright, so `#[derive(Deserialize)]` cannot
//! read a real agentgateway config. The derive only works where the enum sits
//! behind `#[serde(flatten)]`, which routes through a map deserializer instead.
//!
//! [`one_of_enum!`] generates the map representation directly, so it behaves
//! identically flattened or not, and in YAML or JSON.
//!
//! Two deliberate choices in the generated deserializer:
//!
//! - **Unknown keys are skipped, not rejected.** Flattening hands the enum
//!   every key its parent did not claim, so rejecting strangers would make an
//!   unrecognized sibling field fatal — exactly the forward-incompatibility
//!   this crate is trying to avoid.
//! - **Two known keys are rejected.** `{exact: /a, pathPrefix: /b}` has no
//!   sensible reading, and silently taking the first would make routing depend
//!   on key order in a YAML file.

macro_rules! one_of_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $key:literal => $variant:ident($ty:ty)
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $(
                $(#[$vmeta])*
                $variant($ty),
            )+
        }

        impl ::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                use ::serde::ser::SerializeMap as _;
                let mut map = serializer.serialize_map(Some(1))?;
                match self {
                    $( $name::$variant(value) => map.serialize_entry($key, value)?, )+
                }
                map.end()
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> ::core::result::Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                const KEYS: &[&str] = &[$($key),+];

                struct OneOfVisitor;

                impl<'de> ::serde::de::Visitor<'de> for OneOfVisitor {
                    type Value = $name;

                    fn expecting(
                        &self,
                        f: &mut ::core::fmt::Formatter<'_>,
                    ) -> ::core::fmt::Result {
                        write!(f, "a map with exactly one of {:?}", KEYS)
                    }

                    fn visit_map<A>(
                        self,
                        mut map: A,
                    ) -> ::core::result::Result<Self::Value, A::Error>
                    where
                        A: ::serde::de::MapAccess<'de>,
                    {
                        let mut found: ::core::option::Option<(&'static str, $name)> = None;

                        while let Some(key) = map.next_key::<String>()? {
                            let parsed = match key.as_str() {
                                $(
                                    $key => Some((
                                        $key,
                                        $name::$variant(map.next_value::<$ty>()?),
                                    )),
                                )+
                                _ => {
                                    map.next_value::<::serde::de::IgnoredAny>()?;
                                    None
                                }
                            };

                            if let Some((key, value)) = parsed {
                                if let Some((first, _)) = &found {
                                    return Err(::serde::de::Error::custom(format!(
                                        "`{}` and `{}` are mutually exclusive; specify exactly one of {:?}",
                                        first, key, KEYS
                                    )));
                                }
                                found = Some((key, value));
                            }
                        }

                        found.map(|(_, value)| value).ok_or_else(|| {
                            ::serde::de::Error::custom(format!(
                                "expected exactly one of {:?}",
                                KEYS
                            ))
                        })
                    }
                }

                deserializer.deserialize_map(OneOfVisitor)
            }
        }
    };
}

pub(crate) use one_of_enum;
