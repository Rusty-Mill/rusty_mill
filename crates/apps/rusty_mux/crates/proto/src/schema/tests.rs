use std::collections::BTreeSet;

use rusty_mux_core::{BlockId, ClientId, EntityId, SessionId, Size, WindowId};
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
        Ty::Words => Value::Words(vec!["-c".into(), "exit 3".into()]),
        Ty::Flag => Value::Flag(true),
    }
}

fn samples(method: &Method) -> Vec<(&'static str, Value)> {
    method
        .params
        .iter()
        .map(|p| match p.name {
            // Text, but one of four words.
            "side" => (p.name, Value::Text("right".into())),
            _ => (p.name, sample(p.ty)),
        })
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
    let (renames, other): (Vec<&Method>, Vec<&Method>) = other
        .into_iter()
        .partition(|m| m.name.starts_with("rename-"));
    let types: Vec<u8> = other.iter().map(|m| ty(m)).collect();
    let unique: BTreeSet<u8> = types.iter().copied().collect();
    assert_eq!(unique.len(), types.len(), "two methods build one type");
    // 0x0d was Capture; 0x0f is BlockCall, 0x10 Rename.
    let expected = (0x02..=0x14).filter(|t| ![0x0d, 0x0f, 0x10].contains(t));
    assert_eq!(unique, expected.collect());
    assert!(block.iter().all(|m| ty(m) == 0x0f));
    assert_eq!(renames.len(), 4);
    assert!(renames.iter().all(|m| ty(m) == 0x10));
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
        [
            "terminal.format",
            "terminal.send-keys",
            "terminal.pwd",
            "terminal.title",
            "terminal.status",
            "terminal.process"
        ]
    );
    for name in block_methods {
        let method = block_method(name).expect(name);
        assert_eq!(method.params[0].ty, Ty::Id(Entity::Block), "{name}");
        assert_eq!(method.needs, Features::BLOCK_CALL, "{name}");
        let writes = name == "terminal.send-keys";
        let returns = if writes {
            Returns::Unanswered
        } else {
            Returns::Text
        };
        assert_eq!(method.returns, returns, "{name}");
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
    assert!(
        CATALOG
            .iter()
            .all(|m| m.params.len() <= crate::args::MAX_ARGS)
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
            method.name.starts_with("create-") || method.name == "split-block",
            "{}",
            method.name
        );
    }
}

#[test]
fn only_writes_go_unanswered() {
    let unanswered: Vec<&str> = CATALOG
        .iter()
        .filter(|m| m.returns == Returns::Unanswered)
        .map(|m| m.name)
        .collect();
    assert_eq!(unanswered, ["input", "terminal.send-keys"]);
}

#[test]
fn methods_need_the_features_their_messages_are_gated_by() {
    let needs = |name| method(name).expect(name).needs;
    assert_eq!(needs("attach"), Features::CHECKPOINT_V4);
    assert_eq!(needs("sync"), Features::SYNC);
    assert_eq!(needs("terminal.format"), Features::BLOCK_CALL);
    assert_eq!(needs("list"), Features::INSPECT);
    assert_eq!(needs("create-session"), Features::NONE);
    assert_eq!(needs("create-block"), Features::NONE);
    for name in [
        "rename-session",
        "rename-window",
        "rename-block",
        "rename-client",
        "disconnect-client",
    ] {
        assert_eq!(needs(name), Features::LABELS, "{name}");
    }
    for name in ["split-block", "zoom-block", "unzoom-window"] {
        assert_eq!(needs(name), Features::LAYOUT, "{name}");
    }
}

#[test]
fn each_rename_names_its_own_kind() {
    let cases = [
        (
            "rename-session",
            EntityId::Session(SessionId::from_uuid(id(1))),
        ),
        (
            "rename-window",
            EntityId::Window(WindowId::from_uuid(id(1))),
        ),
        ("rename-block", EntityId::Block(BlockId::from_uuid(id(1)))),
        (
            "rename-client",
            EntityId::Client(ClientId::from_uuid(id(1))),
        ),
    ];
    for (name, target) in cases {
        let method = method(name).expect(name);
        let entity = method.params[0].name;
        let args = [
            ("name", Value::Text("x".into())),
            (entity, Value::Id(id(1))),
        ];
        assert_eq!(
            method.request(4, &args),
            Ok(Message::Rename {
                req: 4,
                target,
                name: "x".into()
            }),
            "{name}"
        );
    }
}

#[test]
fn optional_parameters_add_their_features_only_when_given() {
    let create = method("create-block").expect("create-block");
    assert_eq!(create.needs_for(["window", "cols", "rows"]), Features::NONE);
    assert_eq!(create.needs_for(["window", "cwd"]), Features::LAUNCH);
    assert_eq!(create.needs_for(["command", "arg"]), Features::LAUNCH);
    let optional: Vec<&str> = CATALOG
        .iter()
        .flat_map(|m| m.params)
        .filter(|p| p.presence != Presence::Required)
        .map(|p| p.name)
        .collect();
    assert_eq!(
        optional,
        [
            "command", "arg", "cwd", "command", "arg", "cwd", "trim", "unwrap", "vt"
        ]
    );
    let format = method("terminal.format").expect("format");
    assert_eq!(format.needs_for(["block", "history"]), Features::BLOCK_CALL);
    assert_eq!(
        format.needs_for(["history", "vt"]),
        Features::BLOCK_CALL.union(Features::CAPTURE)
    );
}

#[test]
fn create_block_launches_a_program_only_with_a_command() {
    let create = method("create-block").expect("create-block");
    let base = || {
        vec![
            ("window", Value::Id(id(1))),
            ("cols", Value::Cells(80)),
            ("rows", Value::Cells(24)),
        ]
    };
    let launch = |extra: Vec<(&'static str, Value)>| {
        let mut args = base();
        args.extend(extra);
        match create.request(1, &args) {
            Ok(Message::CreateBlock { launch, .. }) => Ok(launch),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e),
        }
    };
    assert_eq!(launch(Vec::new()), Ok(Launch::default()));
    assert_eq!(
        launch(vec![
            ("cwd", Value::Text("/srv".into())),
            ("command", Value::Text("make".into())),
            ("arg", Value::Words(vec!["-j".into(), "4".into()])),
        ]),
        Ok(Launch {
            program: Some(Program {
                command: "make".into(),
                args: vec!["-j".into(), "4".into()],
            }),
            cwd: Some("/srv".into()),
        })
    );
    assert!(matches!(
        launch(vec![("arg", Value::Words(vec!["x".into()]))]),
        Err(CallError::Invalid { param: "arg", .. })
    ));
    assert_eq!(Ty::Words.parse("a b"), Ok(Value::Words(vec!["a b".into()])));
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
    assert_eq!(Ty::Flag.parse("true"), Ok(Value::Flag(true)));
    assert_eq!(Ty::Flag.parse("false"), Ok(Value::Flag(false)));
    for (ty, text) in [
        (block, "not-a-uuid"),
        (Ty::Cells, "0"),
        (Ty::Cells, "65536"),
        (Ty::Cells, "-1"),
        (Ty::Cells, ""),
        (Ty::Lines, "-1"),
        (Ty::Lines, "4294967296"),
        (Ty::Flag, "yes"),
        (Ty::Flag, ""),
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

#[test]
fn split_names_its_side_and_launch() {
    let method = method("split-block").expect("split-block");
    let args = [
        ("block", Value::Id(id(1))),
        ("side", Value::Text("up".into())),
        ("command", Value::Text("top".into())),
    ];
    assert_eq!(
        method.request(3, &args),
        Ok(Message::Split {
            req: 3,
            block: BlockId::from_uuid(id(1)),
            side: rusty_mux_core::Side::Up,
            launch: Launch {
                program: Some(Program {
                    command: "top".into(),
                    args: Vec::new()
                }),
                cwd: None
            },
        })
    );
    let bad = [
        ("block", Value::Id(id(1))),
        ("side", Value::Text("north".into())),
    ];
    assert!(matches!(
        method.request(3, &bad),
        Err(CallError::Invalid { param: "side", .. })
    ));
    assert_eq!(
        method.needs_for(["block", "side", "cwd"]),
        Features::LAYOUT.union(Features::LAUNCH)
    );
}
