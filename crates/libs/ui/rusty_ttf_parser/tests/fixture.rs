//! Decoding of the real `ligtest.ttf` fixture (also used by `rusty_term`'s
//! shaper tests, whose expected glyph ids these mirror).

use rusty_ttf_parser::gsub::SubstitutionSubtable;
use rusty_ttf_parser::opentype_layout::{ChainedContextLookup, ContextLookup};
use rusty_ttf_parser::{Face, FaceParsingError, GlyphId, Tag};

const FONT: &[u8] = include_bytes!("data/ligtest.ttf");

fn face() -> Face<'static> {
    Face::parse(FONT, 0).expect("fixture parses")
}

#[test]
fn reads_head_maxp_and_cmap() {
    let f = face();
    assert!(f.units_per_em() > 0);
    assert!(f.number_of_glyphs() >= 9);
    assert_eq!(f.glyph_index('f'), Some(GlyphId(5)));
    assert_eq!(f.glyph_index('i'), Some(GlyphId(6)));
    assert_eq!(f.glyph_index('<'), Some(GlyphId(4)));
    assert_eq!(f.glyph_index('='), Some(GlyphId(2)));
    assert_eq!(f.glyph_index('?'), None);
}

#[test]
fn liga_feature_points_at_a_ligature_lookup() {
    let gsub = face().tables().gsub.expect("GSUB");
    let liga = Tag::from_bytes(b"liga");
    let feature = gsub
        .features
        .into_iter()
        .find(|f| f.tag == liga)
        .expect("liga");
    assert!(!feature.lookup_indices.is_empty());

    let lookup = gsub
        .lookups
        .get(feature.lookup_indices.get(0).unwrap())
        .unwrap();
    let lig = lookup
        .subtables()
        .find_map(|s| match s {
            SubstitutionSubtable::Ligature(l) => Some(l),
            _ => None,
        })
        .expect("a ligature subtable");

    // f i -> glyph 7.
    let set = lig
        .ligature_sets
        .get(lig.coverage.get(GlyphId(5)).unwrap())
        .unwrap();
    let ligature = set.get(0).unwrap();
    assert_eq!(ligature.glyph, GlyphId(7));
    assert_eq!(
        ligature.components.iter().collect::<Vec<_>>(),
        vec![GlyphId(6)]
    );
    assert!(lig.coverage.get(GlyphId(2)).is_none());
}

#[test]
fn calt_feature_has_a_chained_context_driving_a_single_substitution() {
    let gsub = face().tables().gsub.expect("GSUB");
    let calt = Tag::from_bytes(b"calt");
    let feature = gsub
        .features
        .into_iter()
        .find(|f| f.tag == calt)
        .expect("calt");

    let mut chained = 0;
    let mut nested_single = 0;
    for index in feature.lookup_indices.iter() {
        for subtable in gsub.lookups.get(index).unwrap().subtables() {
            if let SubstitutionSubtable::ChainContext(c) = subtable {
                chained += 1;
                let records = match c {
                    ChainedContextLookup::Format3 { lookups, .. } => lookups,
                    ChainedContextLookup::Format1 { coverage, sets } => {
                        let set = sets.get(coverage.get(GlyphId(4)).unwrap()).unwrap();
                        set.get(0).unwrap().lookups
                    }
                    ChainedContextLookup::Format2 { .. } => continue,
                };
                for record in records.iter() {
                    let target = gsub.lookups.get(record.lookup_list_index).unwrap();
                    nested_single += target
                        .subtables()
                        .filter(|s| matches!(s, SubstitutionSubtable::Single(_)))
                        .count();
                }
            }
        }
    }
    assert!(chained > 0, "calt should contain a chained context lookup");
    assert!(
        nested_single > 0,
        "its nested lookup should be a single substitution"
    );
}

#[test]
fn context_lookup_type_is_reachable_through_the_public_api() {
    // Compile-time check that the public enum covers what shapers match on.
    fn _accepts(_: ContextLookup<'_>) {}
}

#[test]
fn rejects_garbage_with_a_typed_error() {
    assert_eq!(
        Face::parse(b"", 0).unwrap_err(),
        FaceParsingError::UnknownMagic
    );
    assert_eq!(
        Face::parse(b"not a font file", 0).unwrap_err(),
        FaceParsingError::UnknownMagic
    );
    assert_eq!(
        Face::parse(FONT, 1).unwrap_err(),
        FaceParsingError::FaceIndexOutOfBounds
    );
    assert_eq!(
        Face::parse(&FONT[..20], 0).unwrap_err(),
        FaceParsingError::MalformedFont
    );
}
