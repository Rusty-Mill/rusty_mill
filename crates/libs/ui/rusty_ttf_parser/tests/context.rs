//! Behavioural decoding of GSUB type 5 (context) lookups, all three formats,
//! from hand-built tables. (The real-font corpus used for the differential
//! check contained no format 1 table, so this is its only coverage.)

mod common;
use common::{B, font};

use rusty_ttf_parser::gsub::SubstitutionSubtable;
use rusty_ttf_parser::opentype_layout::ContextLookup;
use rusty_ttf_parser::{Face, GlyphId, Tag};

/// A GSUB with one `calt` feature -> lookup 0 of type 5 holding `subtable`.
fn gsub_calt(subtable: &[u8]) -> Vec<u8> {
    let lookup = B::new().u16(5).u16(0).u16(1).u16(8).bytes(subtable);
    let lookup_list = B::new().u16(1).u16(4).bytes(&lookup.0);
    let feature = B::new().u16(0).u16(1).u16(0);
    let feature_list = B::new().u16(1).bytes(b"calt").u16(8).bytes(&feature.0);
    let features_at = 10;
    let lookups_at = features_at + feature_list.len();
    B::new()
        .u16(1)
        .u16(0)
        .u16(0)
        .u16(features_at as u16)
        .u16(lookups_at as u16)
        .bytes(&feature_list.0)
        .bytes(&lookup_list.0)
        .0
}

fn context_of(subtable: &[u8]) -> ContextLookup<'static> {
    let bytes: &'static [u8] =
        Box::leak(font(1000, 20, &[(b"GSUB", gsub_calt(subtable))]).into_boxed_slice());
    let gsub = Face::parse(bytes, 0).unwrap().tables().gsub.expect("GSUB");
    let feature = gsub.features.into_iter().next().unwrap();
    assert_eq!(feature.tag, Tag::from_bytes(b"calt"));
    let lookup = gsub
        .lookups
        .get(feature.lookup_indices.get(0).unwrap())
        .unwrap();
    match lookup.subtables().next() {
        Some(SubstitutionSubtable::Context(c)) => c,
        other => panic!("expected a context subtable, got {other:?}"),
    }
}

#[test]
fn context_format1_decodes_a_glyph_keyed_rule() {
    // Rule: after glyph 5, glyph 6 follows -> run lookup 3 at position 0.
    let subtable = B::new()
        .u16(1) // format
        .u16(8) // coverage offset
        .u16(1) // rule set count
        .u16(14) // rule set 0 offset
        .u16(1)
        .u16(1)
        .u16(5) // coverage: format 1, one glyph, glyph 5   (at 8)
        .u16(1)
        .u16(4) // rule set (at 14): one rule at +4
        .u16(2)
        .u16(1)
        .u16(6) // rule: glyphCount 2, one record, input [6]  (at 18)
        .u16(0)
        .u16(3) // record: sequence index 0, lookup 3
        .0;
    let ContextLookup::Format1 { coverage, sets } = context_of(&subtable) else {
        panic!("format 1")
    };
    assert_eq!(coverage.get(GlyphId(5)), Some(0));
    assert_eq!(coverage.get(GlyphId(6)), None);
    let set = sets
        .get(coverage.get(GlyphId(5)).unwrap())
        .expect("rule set 0");
    assert_eq!(set.len(), 1);
    let rule = set.get(0).expect("rule 0");
    assert_eq!(rule.input.iter().collect::<Vec<_>>(), vec![6]);
    let record = rule.lookups.get(0).unwrap();
    assert_eq!((record.sequence_index, record.lookup_list_index), (0, 3));
    assert!(set.get(1).is_none(), "no second rule");
}

#[test]
fn context_format2_decodes_a_class_keyed_rule_and_null_rule_sets() {
    // Classes: glyph 6 is class 1. Rule set for class 1: input [class 1] -> lookup 3.
    let subtable = B::new()
        .u16(2) // format
        .u16(12) // coverage offset
        .u16(18) // class def offset
        .u16(2) // class rule set count
        .u16(0) // class 0: no rule set (null offset)
        .u16(26) // class 1: rule set offset
        .u16(1)
        .u16(1)
        .u16(5) // coverage (at 12)
        .u16(1)
        .u16(6)
        .u16(1)
        .u16(1) // class def format 1: start 6, one class value = 1 (at 18)
        .u16(1)
        .u16(4) // rule set (at 26)
        .u16(2)
        .u16(1)
        .u16(1) // rule: glyphCount 2, one record, input [class 1] (at 30)
        .u16(0)
        .u16(3)
        .0;
    let ContextLookup::Format2 {
        coverage,
        classes,
        sets,
    } = context_of(&subtable)
    else {
        panic!("format 2")
    };
    assert!(coverage.contains(GlyphId(5)));
    assert_eq!(
        (
            classes.get(GlyphId(6)),
            classes.get(GlyphId(7)),
            classes.get(GlyphId(5))
        ),
        (1, 0, 0)
    );
    assert!(sets.get(0).is_none(), "a null offset means no rule set");
    let rule = sets
        .get(1)
        .and_then(|set| set.get(0))
        .expect("class 1 rule");
    assert_eq!(rule.input.iter().collect::<Vec<_>>(), vec![1]);
    assert_eq!(rule.lookups.get(0).map(|r| r.lookup_list_index), Some(3));
}

#[test]
fn context_format3_decodes_per_position_coverages() {
    let subtable = B::new()
        .u16(3) // format
        .u16(2) // glyph count
        .u16(1) // record count
        .u16(14) // coverage 0 offset
        .u16(20) // coverage 1 offset
        .u16(0)
        .u16(3) // record (at 10)
        .u16(1)
        .u16(1)
        .u16(5) // coverage 0 (at 14)
        .u16(1)
        .u16(1)
        .u16(6) // coverage 1 (at 20)
        .0;
    let ContextLookup::Format3 {
        coverage,
        coverages,
        lookups,
    } = context_of(&subtable)
    else {
        panic!("format 3")
    };
    assert_eq!(coverage.get(GlyphId(5)), Some(0));
    assert_eq!(coverages.len(), 1);
    assert!(coverages.get(0).unwrap().contains(GlyphId(6)));
    assert!(!coverages.get(0).unwrap().contains(GlyphId(5)));
    assert_eq!(
        lookups
            .get(0)
            .map(|r| (r.sequence_index, r.lookup_list_index)),
        Some((0, 3))
    );
}

#[test]
fn malformed_context_subtables_are_dropped_not_misread() {
    // A rule claiming more input glyphs than bytes exist: the subtable is skipped.
    let bad = B::new()
        .u16(1)
        .u16(8)
        .u16(1)
        .u16(14)
        .u16(1)
        .u16(1)
        .u16(5)
        .u16(1)
        .u16(4)
        .u16(9)
        .u16(1)
        .0;
    let bytes = font(1000, 20, &[(b"GSUB", gsub_calt(&bad))]);
    let gsub = Face::parse(&bytes, 0).unwrap().tables().gsub.unwrap();
    let rule_ok = {
        let lookup = gsub.lookups.get(0).unwrap();
        match lookup.subtables().next() {
            Some(SubstitutionSubtable::Context(ContextLookup::Format1 { sets, .. })) => {
                sets.get(0).and_then(|s| s.get(0)).is_some()
            }
            _ => false,
        }
    };
    assert!(!rule_ok, "an over-long rule must not decode");
}
