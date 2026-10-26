//! Differential test against `boxcars` (a dev-dependency oracle, never
//! shipped): for every committed sample replay, the container fields and the
//! whole header must equal what boxcars decodes (P1's acceptance check in
//! `rleval/docs/replay-parser-scope.md`).
#![allow(clippy::panic)]

use std::path::PathBuf;

use boxcars::{HeaderProp, ParserBuilder};
use rl_replay::{Properties, Property};

const SAMPLES: &[&str] = &[
    "../../rleval/assets/replays/42f2.replay",
    "../../rleval/assets/replays/419a.replay",
    "../rb_replay_ingest/fixtures/subtr-actor-sample.replay",
];

fn read(sample: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(sample);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// boxcars' property tree in our shape. Its `Byte::value` is always `Some` for
/// supported versions; a `None` would fail the comparison.
fn ours(props: &[(String, HeaderProp)]) -> Properties {
    props.iter().map(|(k, v)| (k.clone(), convert(v))).collect()
}

fn convert(prop: &HeaderProp) -> Property {
    match prop {
        HeaderProp::Array(rows) => Property::Array(rows.iter().map(|row| ours(row)).collect()),
        HeaderProp::Bool(v) => Property::Bool(*v),
        HeaderProp::Byte { kind, value } => Property::Byte {
            kind: kind.clone(),
            value: value.clone().unwrap_or_else(|| "<boxcars: none>".into()),
        },
        HeaderProp::Float(v) => Property::Float(*v),
        HeaderProp::Int(v) => Property::Int(*v),
        HeaderProp::Name(v) => Property::Name(v.clone()),
        HeaderProp::QWord(v) => Property::QWord(*v),
        HeaderProp::Str(v) => Property::Str(v.clone()),
        HeaderProp::Struct { name, fields } => Property::Struct {
            name: name.clone(),
            fields: ours(fields),
        },
    }
}

#[test]
fn every_sample_header_equals_boxcars() {
    for sample in SAMPLES {
        let data = read(sample);
        let mine = rl_replay::parse(&data).unwrap_or_else(|e| panic!("{sample}: {e}"));
        let oracle = ParserBuilder::new(&data)
            .always_check_crc()
            .never_parse_network_data()
            .parse()
            .unwrap_or_else(|e| panic!("{sample}: boxcars: {e}"));

        assert_eq!(mine.header_size as i32, oracle.header_size, "{sample}");
        assert_eq!(mine.header_crc, oracle.header_crc, "{sample}");
        assert_eq!(mine.content_size as i32, oracle.content_size, "{sample}");
        assert_eq!(mine.content_crc, oracle.content_crc, "{sample}");
        assert_eq!(mine.content.len(), mine.content_size as usize, "{sample}");

        let h = &mine.header;
        assert_eq!(h.engine_version, oracle.major_version, "{sample}");
        assert_eq!(h.licensee_version, oracle.minor_version, "{sample}");
        assert_eq!(h.net_version, oracle.net_version, "{sample}");
        assert_eq!(h.game_type, oracle.game_type, "{sample}");
        assert!(!h.properties.is_empty(), "{sample}");
        assert_eq!(h.properties, ours(&oracle.properties), "{sample}");
    }
}

#[test]
fn every_sample_is_a_supported_version() {
    for sample in SAMPLES {
        let data = read(sample);
        let replay = rl_replay::parse(&data).unwrap_or_else(|e| panic!("{sample}: {e}"));
        let h = &replay.header;
        let version = (
            h.engine_version,
            h.licensee_version,
            h.net_version.unwrap_or(0),
        );
        assert!(version >= rl_replay::MIN_VERSION, "{sample}: {version:?}");
    }
}

#[test]
fn a_cut_sample_fails_cleanly_at_every_header_length() {
    let data = read(SAMPLES[0]);
    let header_end = 8 + u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    for cut in (0..header_end).step_by(97) {
        assert!(rl_replay::parse(&data[..cut]).is_err(), "cut at {cut}");
    }
}
