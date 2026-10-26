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
        Ty::Lines => Value::Lines(0),
    }
}

fn samples(method: &Method) -> Vec<(&'static str, Value)> {
    method
        .params
        .iter()
        .map(|p| (p.name, sample(p.ty)))
        .collect()
}

/// Kebab case, optionally after a block type and a dot.
fn is_method_name(name: &str) -> bool {
    match name.split_once('.') {
        Some((ty, verb)) => is_kebab(ty) && is_kebab(verb),
        None => is_kebab(name),
    }
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
        assert!(is_method_name(method.name), "{}", method.name);
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
    let ty = |m: &Method| encode(&m.request(1, &samples(m)).expect(m.name)).expect("encode")[4];
    let (block, other): (Vec<&Method>, Vec<&Method>) =
        CATALOG.iter().partition(|m| m.is_block_method());
    let types: Vec<u8> = other.iter().map(|m| ty(m)).collect();
    let unique: BTreeSet<u8> = types.iter().copied().collect();
    assert_eq!(unique.len(), types.len(), "two methods build one type");
    // 0x0d was Capture; 0x0f is BlockCall.
    assert_eq!(unique, (0x02..=0x0e).filter(|t| *t != 0x0d).collect());
    assert!(block.iter().all(|m| ty(m) == 0x0f));
}

#[test]
fn block_methods_take_the_block_first_and_send_the_rest() {
    let block_methods: Vec<&str> = CATALOG
        .iter()
        .filter(|m| m.is_block_method())
        .map(|m| m.name)
        .collect();
    assert_eq!(
        block_methods,
        ["terminal.format", "terminal.pwd", "terminal.title"]
    );
    for name in block_methods {
        let method = block_method(name).expect(name);
        assert_eq!(method.params[0].ty, Ty::Id(Entity::Block), "{name}");
        assert_eq!(method.needs, Features::BLOCK_CALL, "{name}");
        assert_eq!(method.returns, Returns::Text, "{name}");
    }
    let format = method("terminal.format").expect("format");
    let args = [("history", Value::Lines(3)), ("block", Value::Id(id(1)))];
    assert_eq!(
        format.request(2, &args),
        Ok(Message::BlockCall {
            req: 2,
            block: BlockId::from_uuid(id(1)),
            method: "terminal.format".into(),
            args: vec![("history".into(), Value::Lines(3))],
        })
    );
    assert!(block_method("list").is_none());
    assert!(block_method("terminal.nope").is_none());
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
fn methods_need_the_features_their_messages_are_gated_by() {
    let needs = |name| method(name).expect(name).needs;
    assert_eq!(needs("attach"), Features::CHECKPOINT_V3);
    assert_eq!(needs("sync"), Features::SYNC);
    assert_eq!(needs("terminal.format"), Features::BLOCK_CALL);
    assert_eq!(needs("list"), Features::INSPECT);
    assert_eq!(needs("create-session"), Features::NONE);
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
    assert_eq!(Ty::Lines.parse("0"), Ok(Value::Lines(0)));
    for (ty, text) in [
        (block, "not-a-uuid"),
        (Ty::Cells, "0"),
        (Ty::Cells, "65536"),
        (Ty::Cells, "-1"),
        (Ty::Cells, ""),
        (Ty::Lines, "-1"),
        (Ty::Lines, "4294967296"),
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
