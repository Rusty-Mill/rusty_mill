//! The reader against buffers made by another implementation (`rlbot_flat`/planus), and the
//! builder against its own reader plus the layout rules those buffers show.

mod fixtures;

use rusty_flatbuffers::{Builder, Error, Table};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn f32le(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// `InterfacePacket`: the union type and the message table.
fn message(buf: &[u8]) -> (u8, Table<'_>) {
    let root = Table::root(buf).unwrap();
    (
        root.scalar(0, 0u8).unwrap(),
        root.table(1).unwrap().unwrap(),
    )
}

#[test]
fn reads_a_table_with_a_string_and_bools() {
    let buf = unhex(fixtures::CONNECTION);
    let (kind, t) = message(&buf);
    assert_eq!(kind, 9, "ConnectionSettings is union member 9");
    assert_eq!(t.string(0).unwrap(), Some("rusty/bot1"));
    assert!(t.scalar(1, false).unwrap());
    assert!(!t.scalar(2, false).unwrap());
    assert!(t.scalar(3, false).unwrap());
}

#[test]
fn absent_fields_read_as_the_default() {
    let buf = unhex(fixtures::INIT);
    let (_, t) = message(&buf);
    assert_eq!(t.string(0).unwrap(), None);
    assert_eq!(t.scalar(5, 7u32).unwrap(), 7, "slot past the vtable");
    let buf = unhex(fixtures::STOP);
    let (_, t) = message(&buf);
    assert!(t.scalar(0, false).unwrap());
}

#[test]
fn reads_inline_structs_and_scalars() {
    let buf = unhex(fixtures::INPUT);
    let (kind, t) = message(&buf);
    assert_eq!(kind, 4);
    assert_eq!(t.scalar(0, 0u32).unwrap(), 3);
    let c = t.struct_bytes(1, 24).unwrap().unwrap();
    assert_eq!(f32le(c, 0), 1.0, "throttle");
    assert_eq!(f32le(c, 4), -0.5, "steer");
    assert_eq!(f32le(c, 8), 0.25, "pitch");
    assert_eq!(f32le(c, 16), -1.0, "roll");
    assert_eq!(
        &c[20..24],
        &[1, 1, 0, 0],
        "jump, boost, handbrake, use_item"
    );
}

#[test]
fn reads_a_vector_of_tables_with_optional_structs() {
    let buf = unhex(fixtures::STATE);
    let (kind, t) = message(&buf);
    assert_eq!(kind, 5);
    let balls = t.vector(0).unwrap().unwrap();
    assert_eq!(balls.len(), 2);
    // DesiredBallState -> DesiredPhysics -> Vector3Partial -> optional `Float` structs.
    let float = |v: Table<'_>, slot| v.struct_bytes(slot, 4).unwrap().map(|b| f32le(b, 0));
    let physics = balls.table(0).unwrap().table(0).unwrap().unwrap();
    let loc = physics.table(0).unwrap().unwrap();
    assert_eq!(
        (float(loc, 0), float(loc, 1), float(loc, 2)),
        (Some(1.0), Some(2.0), Some(3.0))
    );
    assert!(physics.table(1).unwrap().is_none(), "rotation omitted");
    let vel = physics.table(2).unwrap().unwrap();
    assert_eq!((float(vel, 1), float(vel, 2)), (Some(100.0), Some(-5.5)));
    assert_eq!(float(vel, 0), Some(0.0), "a present struct keeps its zero");
    let second = balls.table(1).unwrap().table(0).unwrap().unwrap();
    assert!(second.table(0).unwrap().is_none());
    let spin = second.table(3).unwrap().unwrap();
    assert_eq!(float(spin, 0), Some(9.0));
    assert!(matches!(
        balls.table(2),
        Err(Error::IndexOutOfRange { index: 2, len: 2 })
    ));
    assert!(
        t.vector(1).unwrap().is_some_and(|v| v.is_empty()),
        "no cars"
    );
}

#[test]
fn no_prefix_of_any_fixture_panics() {
    for hex in [
        fixtures::CONNECTION,
        fixtures::STOP,
        fixtures::INIT,
        fixtures::INPUT,
        fixtures::STATE,
    ] {
        let buf = unhex(hex);
        for cut in 0..buf.len() {
            let short = &buf[..cut];
            // Walk everything the schema could ask for; errors are fine, panics are not.
            if let Ok(root) = Table::root(short) {
                let _ = root.scalar(0, 0u8);
                if let Ok(Some(m)) = root.table(1) {
                    let _ = m.string(0);
                    let _ = m.vector(0).map(|v| v.map(|v| v.table(0)));
                    let _ = m.struct_bytes(1, 24);
                }
            }
        }
    }
}

#[test]
fn a_byte_flipped_anywhere_never_panics() {
    let buf = unhex(fixtures::STATE);
    for i in 0..buf.len() {
        for flip in [0x01u8, 0x80, 0xff] {
            let mut bad = buf.clone();
            bad[i] ^= flip;
            if let Ok(root) = Table::root(&bad) {
                if let Ok(Some(m)) = root.table(1) {
                    if let Ok(Some(v)) = m.vector(0) {
                        for k in 0..v.len().min(4) {
                            if let Ok(b) = v.table(k) {
                                let _ = b.table(0).map(|p| p.map(|p| p.table(0)));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn builder_output_reads_back_and_follows_the_layout_rules() {
    let mut b = Builder::new();
    let name = b.create_string("rusty/bot1").unwrap();
    let mut t = b.start_table();
    t.add_offset(0, name);
    t.add_scalar(1, true, false);
    t.add_scalar(2, false, false);
    t.add_scalar(3, true, false);
    let msg = t.finish().unwrap();
    let mut r = b.start_table();
    r.add_scalar(0, 9u8, 0);
    r.add_offset(1, msg);
    let root = r.finish().unwrap();
    let buf = b.finish(root, None).unwrap();

    assert_eq!(buf.len() % 4, 0);
    let (kind, t) = message(&buf);
    assert_eq!(kind, 9);
    assert_eq!(t.string(0).unwrap(), Some("rusty/bot1"));
    assert!(t.scalar(1, false).unwrap());
    assert!(!t.scalar(2, false).unwrap());
    assert!(t.scalar(3, false).unwrap());
}

#[test]
fn builder_vectors_structs_and_identifiers_round_trip() {
    let mut b = Builder::new();
    let nums = b.create_vector(&[10u32, 20, 30]).unwrap();
    let names = [
        b.create_string("a").unwrap(),
        b.create_string("bcd").unwrap(),
    ];
    let strings = b.create_offset_vector(&names).unwrap();
    let mut xyz = |x: f32, y: f32, z: f32| {
        let mut v = Vec::new();
        for f in [x, y, z] {
            v.extend(f.to_le_bytes());
        }
        v
    };
    let (p, q) = (xyz(1.0, 2.0, 3.0), xyz(4.0, 5.0, 6.0));
    let pts = b.create_struct_vector(&[&p, &q], 12, 4).unwrap();
    let mut t = b.start_table();
    t.add_offset(0, nums);
    t.add_offset(1, strings);
    t.add_offset(2, pts);
    t.add_struct(3, &p, 4);
    t.add_scalar(4, 2.5f64, 0.0);
    t.add_scalar(5, -7i16, 0);
    let root = t.finish().unwrap();
    let buf = b.finish(root, Some(*b"TEST")).unwrap();

    assert!(rusty_flatbuffers::has_identifier(&buf, *b"TEST"));
    assert!(!rusty_flatbuffers::has_identifier(&buf, *b"NOPE"));
    let t = Table::root(&buf).unwrap();
    let n = t.vector(0).unwrap().unwrap();
    assert_eq!(
        (n.scalar::<u32>(0).unwrap(), n.scalar::<u32>(2).unwrap()),
        (10, 30)
    );
    let s = t.vector(1).unwrap().unwrap();
    assert_eq!((s.string(0).unwrap(), s.string(1).unwrap()), ("a", "bcd"));
    let pts = t.vector(2).unwrap().unwrap();
    assert_eq!(f32le(pts.struct_bytes(1, 12).unwrap(), 8), 6.0);
    assert_eq!(f32le(t.struct_bytes(3, 12).unwrap().unwrap(), 4), 2.0);
    assert_eq!(t.scalar(4, 0.0f64).unwrap(), 2.5);
    assert_eq!(t.scalar(5, 0i16).unwrap(), -7);
    assert_eq!(
        buf.len() % 8,
        0,
        "aligned to the widest scalar written (f64)"
    );
}

#[test]
fn defaults_are_not_stored() {
    let mut b = Builder::new();
    let mut t = b.start_table();
    t.add_scalar(0, 0u32, 0);
    let root = t.finish().unwrap();
    let small = b.finish(root, None).unwrap();
    let mut b = Builder::new();
    let mut t = b.start_table();
    t.add_scalar(0, 1u32, 0);
    let root = t.finish().unwrap();
    let big = b.finish(root, None).unwrap();
    assert!(small.len() < big.len());
    assert_eq!(Table::root(&small).unwrap().scalar(0, 0u32).unwrap(), 0);
}

#[test]
fn bad_utf8_and_bad_vtables_are_errors() {
    let mut b = Builder::new();
    let s = b.create_string("é").unwrap();
    let mut t = b.start_table();
    t.add_offset(0, s);
    let root = t.finish().unwrap();
    let mut buf = b.finish(root, None).unwrap();
    let at = buf.windows(2).position(|w| w == [0xc3, 0xa9]).unwrap();
    buf[at] = 0xff;
    assert_eq!(
        Table::root(&buf).unwrap().string(0),
        Err(Error::InvalidUtf8)
    );

    let mut garbage = vec![0u8; 16];
    garbage[0] = 4; // root at 4
    garbage[4] = 0x7f; // soffset far outside the buffer
    assert!(Table::root(&garbage).unwrap().scalar(0, 0u8).is_err());
    assert!(Table::root(&[]).is_err());
}
