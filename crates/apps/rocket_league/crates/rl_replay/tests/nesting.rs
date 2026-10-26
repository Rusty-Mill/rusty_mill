//! CRC-valid files exercise the public parser's shared array/struct budget.
#![allow(clippy::unwrap_used, clippy::panic)]

use rl_replay::{crc, parse, Error, Properties, Property, MAX_PROPERTY_DEPTH};

fn name(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as i32 + 1).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

#[derive(Clone, Copy)]
enum Shape {
    Arrays,
    Structs,
    Mixed,
}

fn is_array(shape: Shape, level: usize) -> bool {
    match shape {
        Shape::Arrays => true,
        Shape::Structs => false,
        Shape::Mixed => level % 2 == 0,
    }
}

// Construct iteratively so even a very deep adversarial input is safe to build.
fn chain(out: &mut Vec<u8>, depth: usize, shape: Shape) -> Vec<usize> {
    let mut kind_offsets = Vec::new();
    for level in 0..depth {
        name(out, "child");
        kind_offsets.push(8 + out.len()); // include the section size and CRC
        name(
            out,
            if is_array(shape, level) {
                "ArrayProperty"
            } else {
                "StructProperty"
            },
        );
        out.extend_from_slice(&0u64.to_le_bytes()); // ignored size, as boxcars
        if is_array(shape, level) {
            out.extend_from_slice(&1i32.to_le_bytes());
        } else {
            name(out, "Record");
        }
    }
    name(out, "leaf");
    name(out, "IntProperty");
    out.extend_from_slice(&4u64.to_le_bytes());
    out.extend_from_slice(&42i32.to_le_bytes());
    for _ in 0..depth {
        name(out, "None");
    }
    kind_offsets
}

fn header() -> Vec<u8> {
    let mut out = Vec::new();
    for version in [868i32, 27, 10] {
        out.extend_from_slice(&version.to_le_bytes());
    }
    name(&mut out, "TAGame.Replay_Soccar_TA");
    out
}

fn file(header: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for section in [header, b"content"] {
        out.extend_from_slice(&(section.len() as u32).to_le_bytes());
        out.extend_from_slice(&crc(section).to_le_bytes());
        out.extend_from_slice(section);
    }
    out
}

fn check_chain(mut props: &Properties, depth: usize, shape: Shape) {
    for level in 0..depth {
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "child");
        props = match &props[0].1 {
            Property::Array(rows) if is_array(shape, level) => {
                assert_eq!(rows.len(), 1);
                &rows[0]
            }
            Property::Struct { name, fields } if !is_array(shape, level) => {
                assert_eq!(name, "Record");
                fields
            }
            other => panic!("unexpected level {level}: {other:?}"),
        };
    }
    assert_eq!(props, &vec![("leaf".into(), Property::Int(42))]);
}

#[test]
fn valid_nested_data_survives_at_the_limit_and_siblings_reuse_the_budget() {
    for shape in [Shape::Arrays, Shape::Structs, Shape::Mixed] {
        for depth in [0, 1, MAX_PROPERTY_DEPTH] {
            let mut h = header();
            chain(&mut h, depth, shape);
            chain(&mut h, depth, shape);
            name(&mut h, "None");
            let data = file(&h);
            let replay = parse(&data).unwrap();
            assert_eq!(replay.content, b"content");
            assert_eq!(replay.header.properties.len(), 2);
            for prop in &replay.header.properties {
                check_chain(&vec![prop.clone()], depth, shape);
            }
        }
    }
}

#[test]
fn excessive_nesting_returns_a_structured_error_at_the_rejected_kind() {
    for shape in [Shape::Arrays, Shape::Structs, Shape::Mixed] {
        for depth in [MAX_PROPERTY_DEPTH + 1, 10_000] {
            let mut h = header();
            let offsets = chain(&mut h, depth, shape);
            name(&mut h, "None");
            let err = parse(&file(&h)).unwrap_err();
            assert_eq!(
                err,
                Error::NestingLimit {
                    offset: offsets[MAX_PROPERTY_DEPTH],
                    limit: MAX_PROPERTY_DEPTH,
                }
            );
            assert!(err
                .to_string()
                .contains(&format!("byte {}", offsets[MAX_PROPERTY_DEPTH])));
        }
    }
}

#[test]
fn array_rows_each_receive_the_remaining_budget() {
    let mut h = header();
    name(&mut h, "rows");
    name(&mut h, "ArrayProperty");
    h.extend_from_slice(&0u64.to_le_bytes());
    h.extend_from_slice(&2i32.to_le_bytes());
    for _ in 0..2 {
        chain(&mut h, MAX_PROPERTY_DEPTH - 1, Shape::Mixed);
        name(&mut h, "None");
    }
    name(&mut h, "None");
    let data = file(&h);
    let replay = parse(&data).unwrap();
    let Property::Array(rows) = &replay.header.properties[0].1 else {
        panic!("expected array");
    };
    assert_eq!(rows.len(), 2);
    for row in rows {
        check_chain(row, MAX_PROPERTY_DEPTH - 1, Shape::Mixed);
    }
}
