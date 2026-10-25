use std::collections::BTreeSet;

use rusty_mux_core::{BlockId, Size};
use rusty_uuid::Uuid;

use super::*;
use crate::{decode, encode};

fn id(n: u8) -> Uuid {
    Uuid::from_bytes([n; 16])
}

/// One well-typed value per parameter.
fn sample(ty: Ty) -> Value {
    match ty {
        Ty::Id(_) => Value::Id(id(9)),
        Ty::Text => Value::Text("main".into()),
        Ty::Bytes => Value::Bytes(b"ls\r".to_vec()),
        Ty::Cells => Value::Cells(80),
    }
}

fn samples(method: &Method) -> Vec<(&'static str, Value)> {
    method
        .params
        .iter()
        .map(|p| (p.name, sample(p.ty)))
        .collect()
}

fn is_kebab(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

#[test]
fn names_are_unique_kebab_case() {
    let names: BTreeSet<_> = CATALOG.iter().map(|m| m.name).collect();
    assert_eq!(names.len(), CATALOG.len());
    for method in CATALOG {
        assert!(is_kebab(method.name), "{}", method.name);
        let params: BTreeSet<_> = method.params.iter().map(|p| p.name).collect();
        assert_eq!(params.len(), method.params.len(), "{}", method.name);
        assert!(params.iter().all(|p| is_kebab(p)), "{}", method.name);
    }
}

#[test]
fn every_method_builds_a_message_that_round_trips() {
    for method in CATALOG {
        let message = method.request(3, &samples(method)).expect(method.name);
        let frame = encode(&message).expect("encode");
        assert_eq!(
            decode(&frame).expect("decode"),
            Some((message, frame.len()))
        );
    }
}

#[test]
fn catalog_covers_each_request_type_once() {
    let types: Vec<u8> = CATALOG
        .iter()
        .map(|m| encode(&m.request(1, &samples(m)).expect(m.name)).expect("encode")[4])
        .collect();
    let unique: BTreeSet<u8> = types.iter().copied().collect();
    assert_eq!(unique.len(), types.len(), "two methods build one type");
    assert_eq!(unique, (0x02..=0x0b).collect());
}

#[test]
fn only_create_methods_return_an_id() {
    for method in CATALOG {
        let creates = matches!(method.returns, Returns::Created(_));
        assert_eq!(
            creates,
            method.name.starts_with("create-"),
            "{}",
            method.name
        );
    }
}

#[test]
fn only_input_goes_unanswered() {
    let unanswered: Vec<&str> = CATALOG
        .iter()
        .filter(|m| m.returns == Returns::Unanswered)
        .map(|m| m.name)
        .collect();
    assert_eq!(unanswered, ["input"]);
}

#[test]
fn arguments_land_in_their_fields() {
    let resize = method("resize").expect("resize");
    let args = [
        ("rows", Value::Cells(24)),
        ("block", Value::Id(id(1))),
        ("cols", Value::Cells(100)),
    ];
    assert_eq!(
        resize.request(5, &args),
        Ok(Message::Resize {
            req: 5,
            block: BlockId::from_uuid(id(1)),
            size: Size::new(100, 24).expect("non-zero"),
        })
    );
}

#[test]
fn parses_text_arguments() {
    let block = Ty::Id(Entity::Block);
    assert_eq!(block.parse(&id(4).to_string()), Ok(Value::Id(id(4))));
    assert_eq!(Ty::Bytes.parse("é"), Ok(Value::Bytes("é".into())));
    assert_eq!(Ty::Cells.parse("65535"), Ok(Value::Cells(u16::MAX)));
    for (ty, text) in [
        (block, "not-a-uuid"),
        (Ty::Cells, "0"),
        (Ty::Cells, "65536"),
        (Ty::Cells, "-1"),
        (Ty::Cells, ""),
    ] {
        assert_eq!(
            ty.parse(text),
            Err(CallError::Unparsable {
                ty,
                text: text.into()
            })
        );
    }
}

#[test]
fn rejects_bad_argument_lists() {
    let input = method("input").expect("input");
    let data = || ("data", Value::Bytes(vec![1]));
    let block = || ("block", Value::Id(id(2)));
    let cases = [
        (
            vec![block(), data(), ("echo", Value::Text("x".into()))],
            CallError::UnknownParam("echo".into()),
        ),
        (vec![block()], CallError::Missing("data")),
        (vec![block(), data(), data()], CallError::Repeated("data")),
        (
            vec![block(), ("data", Value::Text("x".into()))],
            CallError::WrongType {
                param: "data",
                ty: Ty::Bytes,
            },
        ),
    ];
    for (args, error) in cases {
        assert_eq!(input.request(1, &args), Err(error));
    }
}

#[test]
fn rejects_a_zero_size_naming_the_zero_side() {
    let resize = method("resize").expect("resize");
    for (cols, rows, side) in [(0, 24, "cols"), (80, 0, "rows"), (0, 0, "cols")] {
        let args = [
            ("block", Value::Id(id(1))),
            ("cols", Value::Cells(cols)),
            ("rows", Value::Cells(rows)),
        ];
        assert!(
            matches!(
                resize.request(1, &args),
                Err(CallError::Invalid { param, .. }) if param == side
            ),
            "{cols}x{rows}"
        );
    }
}

#[test]
fn unknown_method_is_none() {
    assert!(method("hello").is_none());
    assert!(method("").is_none());
}
