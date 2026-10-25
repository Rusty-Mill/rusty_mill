//! The header section: the version triple, the replay class and a property
//! map (`replay-file-format.md` §2, §3).

use crate::error::Error;
use crate::reader::Reader;

/// Oldest `(engine, licensee, net)` version this parser accepts: the oldest
/// committed sample (`419a.replay`) and the newest encoding change boxcars
/// gates below it. Older replays fail rather than decode by guesswork.
pub const MIN_VERSION: (i32, i32, i32) = (868, 23, 8);

/// Maximum number of nested array/struct properties on any header path.
///
/// Both kinds consume the same budget; the root property map has depth zero.
/// This bounds parser recursion and the depth of the returned property tree.
pub const MAX_PROPERTY_DEPTH: usize = 64;

/// Most entries an array property may hold (as boxcars).
const MAX_ARRAY_LEN: usize = 25_000;

/// Properties in file order. A list, not a map: nothing in the format rules
/// out a repeated key.
pub type Properties = Vec<(String, Property)>;

/// The decoded header section.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Engine (major) version, e.g. 868.
    pub engine_version: i32,
    /// Licensee (minor) version, e.g. 27.
    pub licensee_version: i32,
    /// Net (patch) version, e.g. 10; present when engine > 865 and licensee > 17.
    pub net_version: Option<i32>,
    /// Replay class, e.g. `TAGame.Replay_Soccar_TA`.
    pub game_type: String,
    /// The property map: scoreboard, goals, map name, frame count and so on.
    pub properties: Properties,
}

impl Header {
    /// The first property named `key`.
    pub fn get(&self, key: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

/// One header property value.
#[derive(Debug, Clone, PartialEq)]
pub enum Property {
    /// `ArrayProperty`: rows, each a property map (e.g. `PlayerStats`).
    Array(Vec<Properties>),
    /// `BoolProperty`.
    Bool(bool),
    /// `ByteProperty`: an enum type and value, e.g. `OnlinePlatform`.
    Byte {
        /// Enum type name.
        kind: String,
        /// Enum value name.
        value: String,
    },
    /// `FloatProperty`.
    Float(f32),
    /// `IntProperty`.
    Int(i32),
    /// `NameProperty`.
    Name(String),
    /// `QWordProperty`.
    QWord(u64),
    /// `StrProperty`.
    Str(String),
    /// `StructProperty`: a struct name and its fields.
    Struct {
        /// Struct type name.
        name: String,
        /// Its fields.
        fields: Properties,
    },
}

impl Property {
    /// The value of an `Int` property.
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Property::Int(v) => Some(*v),
            _ => None,
        }
    }
}

/// Parse a whole header section. Fails on an unsupported version, an unknown
/// property kind, or bytes left after the property map.
pub(crate) fn parse(r: &mut Reader<'_>) -> Result<Header, Error> {
    let engine_version = r.i32("engine version")?;
    let licensee_version = r.i32("licensee version")?;
    let net_version = if engine_version > 865 && licensee_version > 17 {
        Some(r.i32("net version")?)
    } else {
        None
    };
    if (engine_version, licensee_version, net_version.unwrap_or(0)) < MIN_VERSION {
        return Err(Error::UnsupportedVersion {
            engine: engine_version,
            licensee: licensee_version,
            net: net_version,
        });
    }
    let game_type = r.text("game type")?;
    let properties = properties(r, 0)?;
    if r.remaining() > 0 {
        return Err(Error::TrailingHeaderData {
            bytes: r.remaining(),
        });
    }
    Ok(Header {
        engine_version,
        licensee_version,
        net_version,
        game_type,
        properties,
    })
}

/// A property map, up to and including its `None` key.
fn properties(r: &mut Reader<'_>, depth: usize) -> Result<Properties, Error> {
    let mut out = Vec::new();
    loop {
        let key = r.name("property key")?;
        if key == "None" {
            return Ok(out);
        }
        if let Some(value) = property(r, depth)? {
            out.push((key, value));
        }
    }
}

/// One value after its key. `None` for a `ByteProperty` whose enum type is
/// `None`: it carries one padding byte and no value, and is dropped (as
/// boxcars does).
fn property(r: &mut Reader<'_>, depth: usize) -> Result<Option<Property>, Error> {
    let kind_offset = r.offset();
    let kind = r.name("property kind")?;
    // The value's byte size. Not trusted (boxcars ignores it too): every kind
    // below has a self-delimiting encoding.
    r.u64("property size")?;
    if matches!(kind.as_str(), "ArrayProperty" | "StructProperty") && depth >= MAX_PROPERTY_DEPTH {
        return Err(Error::NestingLimit {
            offset: kind_offset,
            limit: MAX_PROPERTY_DEPTH,
        });
    }
    let value = match kind.as_str() {
        "ArrayProperty" => {
            let offset = r.offset();
            let n = r.len("array length")?;
            if n > MAX_ARRAY_LEN {
                return Err(Error::BadLength {
                    what: "array length",
                    offset,
                    value: n as i64,
                });
            }
            Property::Array(
                (0..n)
                    .map(|_| properties(r, depth + 1))
                    .collect::<Result<_, _>>()?,
            )
        }
        "BoolProperty" => Property::Bool(r.u8("bool value")? == 1),
        "ByteProperty" => {
            let kind = r.name("byte enum type")?;
            if kind == "None" {
                r.take(1, "byte padding")?;
                return Ok(None);
            }
            let value = r.name("byte enum value")?;
            Property::Byte { kind, value }
        }
        "FloatProperty" => Property::Float(r.f32("float value")?),
        "IntProperty" => Property::Int(r.i32("int value")?),
        "NameProperty" => Property::Name(r.text("name value")?),
        "QWordProperty" => Property::QWord(r.u64("qword value")?),
        "StrProperty" => Property::Str(r.text("str value")?),
        "StructProperty" => {
            let name = r.name("struct name")?;
            let fields = properties(r, depth + 1)?;
            Property::Struct { name, fields }
        }
        _ => {
            return Err(Error::UnknownProperty {
                offset: kind_offset,
                kind,
            })
        }
    };
    Ok(Some(value))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn name(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    fn header_start(out: &mut Vec<u8>, engine: i32, licensee: i32, net: Option<i32>) {
        out.extend_from_slice(&engine.to_le_bytes());
        out.extend_from_slice(&licensee.to_le_bytes());
        if let Some(net) = net {
            out.extend_from_slice(&net.to_le_bytes());
        }
        name(out, "TAGame.Replay_Soccar_TA");
    }

    fn int_prop(out: &mut Vec<u8>, key: &str, v: i32) {
        name(out, key);
        name(out, "IntProperty");
        out.extend_from_slice(&4u64.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn parses_ints_and_the_terminator() {
        let mut data = Vec::new();
        header_start(&mut data, 868, 27, Some(10));
        int_prop(&mut data, "TeamSize", 2);
        name(&mut data, "None");
        let header = parse(&mut Reader::new(&data, 0)).unwrap();
        assert_eq!(header.net_version, Some(10));
        assert_eq!(header.game_type, "TAGame.Replay_Soccar_TA");
        assert_eq!(header.get("TeamSize"), Some(&Property::Int(2)));
    }

    #[test]
    fn a_byte_property_without_an_enum_is_dropped() {
        let mut data = Vec::new();
        header_start(&mut data, 868, 27, Some(10));
        name(&mut data, "Platform");
        name(&mut data, "ByteProperty");
        data.extend_from_slice(&0u64.to_le_bytes());
        name(&mut data, "None");
        data.push(0);
        int_prop(&mut data, "TeamSize", 3);
        name(&mut data, "None");
        let header = parse(&mut Reader::new(&data, 0)).unwrap();
        assert_eq!(
            header.properties,
            vec![("TeamSize".into(), Property::Int(3))]
        );
    }

    #[test]
    fn old_versions_are_refused() {
        let mut data = Vec::new();
        header_start(&mut data, 868, 17, None);
        assert!(matches!(
            parse(&mut Reader::new(&data, 0)),
            Err(Error::UnsupportedVersion { net: None, .. })
        ));
        let mut data = Vec::new();
        header_start(&mut data, 868, 22, Some(9));
        assert!(matches!(
            parse(&mut Reader::new(&data, 0)),
            Err(Error::UnsupportedVersion { net: Some(9), .. })
        ));
        let mut data = Vec::new();
        header_start(&mut data, 868, 23, Some(7));
        assert!(matches!(
            parse(&mut Reader::new(&data, 0)),
            Err(Error::UnsupportedVersion { net: Some(7), .. })
        ));
        let mut data = Vec::new();
        header_start(&mut data, 868, 23, Some(8));
        name(&mut data, "None");
        assert!(parse(&mut Reader::new(&data, 0)).is_ok());
    }

    #[test]
    fn an_unknown_kind_names_itself_and_its_offset() {
        let mut data = Vec::new();
        header_start(&mut data, 868, 27, Some(10));
        name(&mut data, "Mystery");
        let at = data.len();
        name(&mut data, "MapProperty");
        data.extend_from_slice(&0u64.to_le_bytes());
        let err = parse(&mut Reader::new(&data, 8)).unwrap_err();
        assert_eq!(
            err,
            Error::UnknownProperty {
                offset: at + 8,
                kind: "MapProperty".into()
            }
        );
    }

    #[test]
    fn a_missing_terminator_is_truncation() {
        let mut data = Vec::new();
        header_start(&mut data, 868, 27, Some(10));
        int_prop(&mut data, "TeamSize", 2);
        assert!(matches!(
            parse(&mut Reader::new(&data, 0)),
            Err(Error::Truncated {
                what: "property key",
                ..
            })
        ));
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut data = Vec::new();
        header_start(&mut data, 868, 27, Some(10));
        name(&mut data, "None");
        data.push(0);
        assert_eq!(
            parse(&mut Reader::new(&data, 0)),
            Err(Error::TrailingHeaderData { bytes: 1 })
        );
    }
}
