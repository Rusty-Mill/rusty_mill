//! Checked input at every public entry point: offsets from another builder, struct layouts and
//! field slots. A rejected call must leave the builder exactly as it was.

use rusty_flatbuffers::{Builder, Error, Offset, Table, MAX_SLOTS};

/// A builder holding one string, finished around an empty table: its bytes show its state.
fn snapshot(b: Builder) -> Vec<u8> {
    let mut b = b;
    let root = b.start_table().finish().unwrap();
    b.finish(root, None).unwrap()
}

/// Offsets from a builder that wrote `n` strings, so they are larger or smaller than anything
/// the destination builder has written.
fn foreign(n: usize) -> Offset {
    let mut other = Builder::new();
    let mut last = other.create_string("x").unwrap();
    for _ in 1..n {
        last = other.create_string("foreign-string").unwrap();
    }
    last
}

#[test]
fn an_offset_from_another_builder_is_rejected_whether_larger_or_smaller() {
    for (theirs, ours) in [(1usize, 6usize), (6, 1)] {
        let make = || {
            let mut b = Builder::new();
            for _ in 0..ours {
                b.create_string("local-string").unwrap();
            }
            b
        };
        let alien = foreign(theirs);

        let mut b = make();
        let mut t = b.start_table();
        assert_eq!(t.add_offset(0, alien), Err(Error::ForeignOffset));
        t.finish().unwrap();

        let mut b = make();
        assert_eq!(
            b.create_offset_vector(&[alien]).err(),
            Some(Error::ForeignOffset)
        );

        let b = make();
        assert_eq!(b.finish(alien, None).err(), Some(Error::ForeignOffset));

        // A rejected call changes nothing: the builder finishes as if it never happened.
        let mut with = make();
        let mut t = with.start_table();
        let _ = t.add_offset(0, alien);
        let table = t.finish().unwrap();
        let with = with.finish(table, None).unwrap();
        let mut without = make();
        let table = without.start_table().finish().unwrap();
        assert_eq!(with, without.finish(table, None).unwrap());
    }
}

#[test]
fn offsets_from_the_same_builder_still_work_in_all_three_places() {
    let mut b = Builder::new();
    let s = b.create_string("ok").unwrap();
    let v = b.create_offset_vector(&[s, s]).unwrap();
    let mut t = b.start_table();
    t.add_offset(0, v).unwrap();
    let root = t.finish().unwrap();
    let buf = b.finish(root, None).unwrap();
    let vec = Table::root(&buf).unwrap().vector(0).unwrap().unwrap();
    assert_eq!(
        (vec.string(0).unwrap(), vec.string(1).unwrap()),
        ("ok", "ok")
    );
}

#[test]
fn invalid_struct_layouts_are_rejected_in_release_and_debug_alike() {
    let four = [0u8; 4];
    let mut b = Builder::new();
    b.create_string("keep").unwrap();
    let mut t = b.start_table();
    assert_eq!(
        t.add_struct(0, &four, 0),
        Err(Error::InvalidLayout),
        "align 0"
    );
    assert_eq!(
        t.add_struct(0, &four, 3),
        Err(Error::InvalidLayout),
        "align 3"
    );
    assert_eq!(
        t.add_struct(0, &four, 8),
        Err(Error::InvalidLayout),
        "size not a multiple"
    );
    assert_eq!(
        t.add_struct(0, &[], 4),
        Err(Error::InvalidLayout),
        "empty struct"
    );
    t.finish().unwrap();

    let item = [0u8; 4];
    let short = [0u8; 3];
    let mut b = Builder::new();
    assert_eq!(
        b.create_struct_vector(&[&item], 4, 0).err(),
        Some(Error::InvalidLayout)
    );
    assert_eq!(
        b.create_struct_vector(&[&item], 4, 3).err(),
        Some(Error::InvalidLayout)
    );
    assert_eq!(
        b.create_struct_vector(&[&item], 6, 4).err(),
        Some(Error::InvalidLayout),
        "stride"
    );
    assert_eq!(
        b.create_struct_vector(&[&item], 0, 4).err(),
        Some(Error::InvalidLayout),
        "zero size"
    );
    assert_eq!(
        b.create_struct_vector(&[&item, &short], 4, 4).err(),
        Some(Error::InvalidLayout),
        "item length"
    );
    // Nothing above wrote anything.
    assert_eq!(snapshot(b), snapshot(Builder::new()));
}

#[test]
fn slots_are_bounded_at_every_entry_point() {
    let mut b = Builder::new();
    let s = b.create_string("s").unwrap();
    let mut t = b.start_table();
    for slot in [usize::MAX, MAX_SLOTS, MAX_SLOTS + 1] {
        assert_eq!(t.add_scalar(slot, 1u8, 0), Err(Error::SlotTooLarge));
        assert_eq!(t.add_offset(slot, s), Err(Error::SlotTooLarge));
        assert_eq!(t.add_struct(slot, &[0u8; 4], 4), Err(Error::SlotTooLarge));
    }
    // The last representable slot works and reads back; the vtable is at its u16 limit.
    t.add_scalar(MAX_SLOTS - 1, 7u8, 0).unwrap();
    let root = t.finish().unwrap();
    let buf = b.finish(root, None).unwrap();
    let table = Table::root(&buf).unwrap();
    assert_eq!(table.scalar(MAX_SLOTS - 1, 0u8).unwrap(), 7);
    assert_eq!(
        table.scalar(MAX_SLOTS, 9u8).unwrap(),
        9,
        "one past the vtable"
    );
}

#[test]
fn reading_an_unaddressable_slot_is_absent_not_a_panic() {
    let mut b = Builder::new();
    let mut t = b.start_table();
    t.add_scalar(0, 1u8, 0).unwrap();
    let root = t.finish().unwrap();
    let buf = b.finish(root, None).unwrap();
    let t = Table::root(&buf).unwrap();
    for slot in [
        usize::MAX,
        usize::MAX / 2,
        usize::MAX / 2 - 1,
        u32::MAX as usize,
    ] {
        assert_eq!(t.scalar(slot, 5u8).unwrap(), 5);
        assert!(t.string(slot).unwrap().is_none());
        assert!(t.table(slot).unwrap().is_none());
        assert!(t.vector(slot).unwrap().is_none());
        assert!(t.struct_bytes(slot, 4).unwrap().is_none());
    }
}
