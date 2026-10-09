//! The GSUB (glyph substitution) table: features, lookups and the substitution
//! subtables for types 1 (single), 4 (ligature), 5 (context) and 6 (chained
//! context), with extension lookups (type 7) resolved transparently.
//!
//! Multiple (2), alternate (3) and reverse-chaining (8) subtables are reported
//! as [`SubstitutionSubtable::Unsupported`]; applying lookups is the caller's
//! job, this crate only decodes them.

use crate::opentype_layout::{ChainedContextLookup, ContextLookup, Coverage};
use crate::reader::{
    FromData, FromSlice, LazyArray16, OffsetArray16, Reader, i16_at, tail32, u16_at, u32_at,
};
use crate::{GlyphId, Tag};

/// A parsed GSUB table.
#[derive(Clone, Copy, Debug)]
pub struct Table<'a> {
    /// Every feature record, in table order (a tag may repeat per script/language).
    pub features: FeatureList<'a>,
    /// The lookups features refer to by index.
    pub lookups: LookupList<'a>,
}

impl<'a> Table<'a> {
    /// Parse the raw bytes of a `GSUB` table.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        if r.u16()? != 1 {
            return None;
        }
        r.skip(2)?; // minor version
        r.skip(2)?; // script list: not needed by callers that pick features by tag
        let feature_list = data.get(usize::from(r.u16()?)..)?;
        let lookup_list = data.get(usize::from(r.u16()?)..)?;
        Some(Table {
            features: FeatureList::parse(feature_list)?,
            lookups: LookupList {
                inner: OffsetArray16::parse(lookup_list)?,
            },
        })
    }
}

/// The `FeatureList` table.
#[derive(Clone, Copy, Debug)]
pub struct FeatureList<'a> {
    data: &'a [u8],
    records: LazyArray16<'a, FeatureRecord>,
}

#[derive(Clone, Copy, Debug)]
struct FeatureRecord {
    tag: Tag,
    offset: u16,
}

impl FromData for FeatureRecord {
    const SIZE: usize = 6;
    fn parse(data: &[u8]) -> Option<Self> {
        Some(FeatureRecord {
            tag: Tag(u32_at(data, 0)?),
            offset: u16_at(data, 4)?,
        })
    }
}

/// One feature: a tag and the lookups it enables.
#[derive(Clone, Copy, Debug)]
pub struct Feature<'a> {
    /// Four-byte feature tag, e.g. `liga`.
    pub tag: Tag,
    /// Indices into the [`LookupList`], in the order they apply.
    pub lookup_indices: LazyArray16<'a, u16>,
}

impl<'a> FeatureList<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let count = r.u16()?;
        Some(FeatureList {
            data,
            records: r.array(count)?,
        })
    }

    /// Number of features.
    pub fn len(&self) -> u16 {
        self.records.len()
    }

    /// `true` when there are no features.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Feature `index`, or `None` if past the end or malformed.
    pub fn get(&self, index: u16) -> Option<Feature<'a>> {
        let record = self.records.get(index)?;
        let table = self.data.get(usize::from(record.offset)..)?;
        let mut r = Reader::new(table);
        r.skip(2)?; // feature params offset
        let count = r.u16()?;
        Some(Feature {
            tag: record.tag,
            lookup_indices: r.array(count)?,
        })
    }
}

impl<'a> IntoIterator for FeatureList<'a> {
    type Item = Feature<'a>;
    type IntoIter = FeatureIter<'a>;

    fn into_iter(self) -> FeatureIter<'a> {
        FeatureIter {
            list: self,
            next: 0,
        }
    }
}

/// Iterator over a [`FeatureList`]; malformed entries are skipped.
#[derive(Clone, Copy, Debug)]
pub struct FeatureIter<'a> {
    list: FeatureList<'a>,
    next: u16,
}

impl<'a> Iterator for FeatureIter<'a> {
    type Item = Feature<'a>;

    fn next(&mut self) -> Option<Feature<'a>> {
        while self.next < self.list.len() {
            let i = self.next;
            self.next += 1;
            if let Some(feature) = self.list.get(i) {
                return Some(feature);
            }
        }
        None
    }
}

/// The `LookupList` table.
#[derive(Clone, Copy, Debug)]
pub struct LookupList<'a> {
    inner: OffsetArray16<'a, Lookup<'a>>,
}

impl<'a> LookupList<'a> {
    /// Number of lookups.
    pub fn len(&self) -> u16 {
        self.inner.len()
    }

    /// `true` when there are no lookups.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Lookup `index`, or `None` if past the end or malformed.
    pub fn get(&self, index: u16) -> Option<Lookup<'a>> {
        self.inner.get(index)
    }
}

/// One lookup: a type and the subtables that implement it.
#[derive(Clone, Copy, Debug)]
pub struct Lookup<'a> {
    kind: u16,
    data: &'a [u8],
    offsets: LazyArray16<'a, u16>,
}

impl<'a> FromSlice<'a> for Lookup<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let kind = r.u16()?;
        r.skip(2)?; // lookup flags
        let count = r.u16()?;
        Some(Lookup {
            kind,
            data,
            offsets: r.array(count)?,
        })
    }
}

impl<'a> Lookup<'a> {
    /// The decoded subtables, in order; malformed ones are skipped.
    pub fn subtables(&self) -> Subtables<'a> {
        Subtables {
            lookup: *self,
            next: 0,
        }
    }
}

/// Iterator over a [`Lookup`]'s subtables.
#[derive(Clone, Copy, Debug)]
pub struct Subtables<'a> {
    lookup: Lookup<'a>,
    next: u16,
}

impl<'a> Iterator for Subtables<'a> {
    type Item = SubstitutionSubtable<'a>;

    fn next(&mut self) -> Option<SubstitutionSubtable<'a>> {
        while self.next < self.lookup.offsets.len() {
            let offset = self.lookup.offsets.get(self.next);
            self.next += 1;
            let parsed = offset
                .and_then(|o| self.lookup.data.get(usize::from(o)..))
                .and_then(|data| SubstitutionSubtable::parse(self.lookup.kind, data));
            if parsed.is_some() {
                return parsed;
            }
        }
        None
    }
}

/// A decoded GSUB subtable.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum SubstitutionSubtable<'a> {
    /// Type 1: replace one glyph with another.
    Single(SingleSubstitution<'a>),
    /// Type 4: replace a run of glyphs with a ligature glyph.
    Ligature(LigatureSubstitution<'a>),
    /// Type 5: run nested lookups when a glyph sequence matches.
    Context(ContextLookup<'a>),
    /// Type 6: as `Context`, with backtrack and lookahead conditions.
    ChainContext(ChainedContextLookup<'a>),
    /// A lookup type this crate does not decode (2, 3, 8, or unknown).
    Unsupported(u16),
}

impl<'a> SubstitutionSubtable<'a> {
    fn parse(kind: u16, data: &'a [u8]) -> Option<Self> {
        match kind {
            1 => SingleSubstitution::parse(data).map(Self::Single),
            4 => LigatureSubstitution::parse(data).map(Self::Ligature),
            5 => ContextLookup::parse(data).map(Self::Context),
            6 => ChainedContextLookup::parse(data).map(Self::ChainContext),
            7 => {
                // Extension: format 1, real type, 32-bit offset from here. An
                // extension may not point at another extension.
                if u16_at(data, 0)? != 1 {
                    return None;
                }
                let real = u16_at(data, 2)?;
                if real == 7 {
                    return None;
                }
                Self::parse(real, tail32(data, u32_at(data, 4)?)?)
            }
            other => Some(Self::Unsupported(other)),
        }
    }
}

/// GSUB type 1.
#[derive(Clone, Copy, Debug)]
pub enum SingleSubstitution<'a> {
    /// Format 1: add `delta` to the glyph id.
    Format1 {
        /// Glyphs the substitution applies to.
        coverage: Coverage<'a>,
        /// Signed offset added to the input glyph id (wrapping at 16 bits).
        delta: i16,
    },
    /// Format 2: look the replacement up by coverage index.
    Format2 {
        /// Glyphs the substitution applies to.
        coverage: Coverage<'a>,
        /// Replacement glyphs, indexed by coverage index.
        substitutes: LazyArray16<'a, GlyphId>,
    },
}

impl<'a> SingleSubstitution<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let format = r.u16()?;
        let coverage = Coverage::parse(data.get(usize::from(r.u16()?)..)?)?;
        match format {
            1 => Some(SingleSubstitution::Format1 {
                coverage,
                delta: i16_at(data, 4)?,
            }),
            2 => {
                let count = r.u16()?;
                Some(SingleSubstitution::Format2 {
                    coverage,
                    substitutes: r.array(count)?,
                })
            }
            _ => None,
        }
    }

    /// The glyphs this substitution applies to.
    pub fn coverage(&self) -> Coverage<'a> {
        match self {
            SingleSubstitution::Format1 { coverage, .. }
            | SingleSubstitution::Format2 { coverage, .. } => *coverage,
        }
    }
}

/// GSUB type 4.
#[derive(Clone, Copy, Debug)]
pub struct LigatureSubstitution<'a> {
    /// First glyphs of every ligature.
    pub coverage: Coverage<'a>,
    /// Ligatures grouped by first glyph (indexed by coverage index).
    pub ligature_sets: LigatureSets<'a>,
}

/// The ligatures that start with one glyph.
pub type LigatureSet<'a> = OffsetArray16<'a, Ligature<'a>>;
/// Ligature sets indexed by coverage index.
pub type LigatureSets<'a> = OffsetArray16<'a, LigatureSet<'a>>;

/// One ligature: its components after the first glyph, and the glyph it forms.
#[derive(Clone, Copy, Debug)]
pub struct Ligature<'a> {
    /// The ligature glyph.
    pub glyph: GlyphId,
    /// Component glyphs following the first.
    pub components: LazyArray16<'a, GlyphId>,
}

impl<'a> FromSlice<'a> for Ligature<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let glyph = GlyphId(r.u16()?);
        let count = r.u16()?;
        Some(Ligature {
            glyph,
            components: r.array(count.checked_sub(1)?)?,
        })
    }
}

impl<'a> LigatureSubstitution<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        if r.u16()? != 1 {
            return None;
        }
        let coverage = Coverage::parse(data.get(usize::from(r.u16()?)..)?)?;
        let count = r.u16()?;
        Some(LigatureSubstitution {
            coverage,
            ligature_sets: OffsetArray16::new(data, r.array(count)?),
        })
    }
}
