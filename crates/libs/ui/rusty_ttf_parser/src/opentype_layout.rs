//! OpenType Layout structures shared by lookup tables: coverage, class
//! definitions, and the contextual-lookup rule sets that GSUB types 5 and 6
//! (and GPOS 7 and 8) use.

use crate::GlyphId;
use crate::reader::{FromData, FromSlice, LazyArray16, OffsetArray16, Reader, u16_at};

/// `start..=end` glyphs mapped to `value` (a coverage index or a class).
#[derive(Clone, Copy, Debug)]
pub struct RangeRecord {
    /// First glyph of the range.
    pub start: GlyphId,
    /// Last glyph of the range.
    pub end: GlyphId,
    /// Coverage index of `start`, or the class of every glyph in the range.
    pub value: u16,
}

impl FromData for RangeRecord {
    const SIZE: usize = 6;
    fn parse(data: &[u8]) -> Option<Self> {
        Some(RangeRecord {
            start: GlyphId(u16_at(data, 0)?),
            end: GlyphId(u16_at(data, 2)?),
            value: u16_at(data, 4)?,
        })
    }
}

impl RangeRecord {
    /// Index of the first range that does not end before `glyph`.
    fn find(ranges: &LazyArray16<'_, RangeRecord>, glyph: GlyphId) -> Option<RangeRecord> {
        let i = ranges.partition_point(|r| r.end.0 < glyph.0);
        ranges.get(i).filter(|r| r.start.0 <= glyph.0)
    }
}

/// The set of glyphs a lookup applies to, with each glyph's index in the set.
#[derive(Clone, Copy, Debug)]
pub enum Coverage<'a> {
    /// Format 1: a sorted glyph list.
    Format1 {
        /// Covered glyphs in ascending order.
        glyphs: LazyArray16<'a, GlyphId>,
    },
    /// Format 2: sorted glyph ranges.
    Format2 {
        /// Covered ranges in ascending order.
        ranges: LazyArray16<'a, RangeRecord>,
    },
}

impl FromData for GlyphId {
    const SIZE: usize = 2;
    fn parse(data: &[u8]) -> Option<Self> {
        u16_at(data, 0).map(GlyphId)
    }
}

impl<'a> FromSlice<'a> for Coverage<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let format = r.u16()?;
        let count = r.u16()?;
        match format {
            1 => Some(Coverage::Format1 {
                glyphs: r.array(count)?,
            }),
            2 => Some(Coverage::Format2 {
                ranges: r.array(count)?,
            }),
            _ => None,
        }
    }
}

impl Coverage<'_> {
    /// The glyph's index within the covered set, or `None` if not covered.
    pub fn get(&self, glyph: GlyphId) -> Option<u16> {
        match self {
            Coverage::Format1 { glyphs } => {
                let i = glyphs.partition_point(|g| g.0 < glyph.0);
                (glyphs.get(i)? == glyph).then_some(i)
            }
            Coverage::Format2 { ranges } => {
                let r = RangeRecord::find(ranges, glyph)?;
                Some(r.value.checked_add(glyph.0 - r.start.0)?)
            }
        }
    }

    /// `true` if the glyph is covered.
    pub fn contains(&self, glyph: GlyphId) -> bool {
        self.get(glyph).is_some()
    }
}

/// Assigns each glyph to a class; unlisted glyphs are class 0.
#[derive(Clone, Copy, Debug)]
pub enum ClassDefinition<'a> {
    /// Format 1: a dense run of classes starting at `start`.
    Format1 {
        /// Glyph of the first entry.
        start: GlyphId,
        /// One class per glyph from `start`.
        classes: LazyArray16<'a, u16>,
    },
    /// Format 2: sorted glyph ranges with a class each.
    Format2 {
        /// Class ranges in ascending order.
        ranges: LazyArray16<'a, RangeRecord>,
    },
}

impl<'a> FromSlice<'a> for ClassDefinition<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        match r.u16()? {
            1 => {
                let start = GlyphId(r.u16()?);
                let count = r.u16()?;
                Some(ClassDefinition::Format1 {
                    start,
                    classes: r.array(count)?,
                })
            }
            2 => {
                let count = r.u16()?;
                Some(ClassDefinition::Format2 {
                    ranges: r.array(count)?,
                })
            }
            _ => None,
        }
    }
}

impl ClassDefinition<'_> {
    /// The glyph's class (0 when unlisted).
    pub fn get(&self, glyph: GlyphId) -> u16 {
        match self {
            ClassDefinition::Format1 { start, classes } => glyph
                .0
                .checked_sub(start.0)
                .and_then(|i| classes.get(i))
                .unwrap_or(0),
            ClassDefinition::Format2 { ranges } => {
                RangeRecord::find(ranges, glyph).map_or(0, |r| r.value)
            }
        }
    }
}

/// Apply lookup `lookup_list_index` at `sequence_index` within the matched input.
#[derive(Clone, Copy, Debug)]
pub struct SequenceLookupRecord {
    /// Position within the matched input sequence.
    pub sequence_index: u16,
    /// Index into the lookup list.
    pub lookup_list_index: u16,
}

impl FromData for SequenceLookupRecord {
    const SIZE: usize = 4;
    fn parse(data: &[u8]) -> Option<Self> {
        Some(SequenceLookupRecord {
            sequence_index: u16_at(data, 0)?,
            lookup_list_index: u16_at(data, 2)?,
        })
    }
}

/// One contextual rule: the input glyphs (or classes) after the first, and the
/// lookups to run on a match.
#[derive(Clone, Copy, Debug)]
pub struct SequenceRule<'a> {
    /// Input glyph ids (format 1) or classes (format 2), excluding the first.
    pub input: LazyArray16<'a, u16>,
    /// Nested lookups to apply.
    pub lookups: LazyArray16<'a, SequenceLookupRecord>,
}

impl<'a> FromSlice<'a> for SequenceRule<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let glyph_count = r.u16()?;
        let lookup_count = r.u16()?;
        Some(SequenceRule {
            input: r.array(glyph_count.checked_sub(1)?)?,
            lookups: r.array(lookup_count)?,
        })
    }
}

/// The rules sharing one first glyph or class.
pub type SequenceRuleSet<'a> = OffsetArray16<'a, SequenceRule<'a>>;
/// Rule sets indexed by coverage index (format 1) or class (format 2).
pub type SequenceRuleSets<'a> = OffsetArray16<'a, SequenceRuleSet<'a>>;

/// Context lookup (GSUB type 5).
#[derive(Clone, Copy, Debug)]
pub enum ContextLookup<'a> {
    /// Format 1: rules keyed by glyph.
    Format1 {
        /// Glyphs that may start a rule.
        coverage: Coverage<'a>,
        /// Rule sets by coverage index.
        sets: SequenceRuleSets<'a>,
    },
    /// Format 2: rules keyed by glyph class.
    Format2 {
        /// Glyphs that may start a rule.
        coverage: Coverage<'a>,
        /// Class of each glyph.
        classes: ClassDefinition<'a>,
        /// Rule sets by class.
        sets: SequenceRuleSets<'a>,
    },
    /// Format 3: one coverage per input position.
    Format3 {
        /// Coverage of the first input glyph.
        coverage: Coverage<'a>,
        /// Coverages of the following input glyphs.
        coverages: OffsetArray16<'a, Coverage<'a>>,
        /// Nested lookups to apply.
        lookups: LazyArray16<'a, SequenceLookupRecord>,
    },
}

impl<'a> FromSlice<'a> for ContextLookup<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        match r.u16()? {
            1 => {
                let coverage = Coverage::parse(data.get(usize::from(r.u16()?)..)?)?;
                let count = r.u16()?;
                let sets = OffsetArray16::new(data, r.array(count)?);
                Some(ContextLookup::Format1 { coverage, sets })
            }
            2 => {
                let coverage = Coverage::parse(data.get(usize::from(r.u16()?)..)?)?;
                let classes = ClassDefinition::parse(data.get(usize::from(r.u16()?)..)?)?;
                let count = r.u16()?;
                let sets = OffsetArray16::new(data, r.array(count)?);
                Some(ContextLookup::Format2 {
                    coverage,
                    classes,
                    sets,
                })
            }
            3 => {
                let glyph_count = r.u16()?;
                let lookup_count = r.u16()?;
                let offsets = r.array::<u16>(glyph_count)?;
                let lookups = r.array(lookup_count)?;
                let coverage = Coverage::parse(data.get(usize::from(offsets.get(0)?)..)?)?;
                let coverages = OffsetArray16::new(data, offsets.tail(1));
                Some(ContextLookup::Format3 {
                    coverage,
                    coverages,
                    lookups,
                })
            }
            _ => None,
        }
    }
}

/// One chained rule: surrounding context plus the input.
#[derive(Clone, Copy, Debug)]
pub struct ChainedSequenceRule<'a> {
    /// Glyphs (or classes) before the input, nearest first.
    pub backtrack: LazyArray16<'a, u16>,
    /// Input glyphs (or classes), excluding the first.
    pub input: LazyArray16<'a, u16>,
    /// Glyphs (or classes) after the input.
    pub lookahead: LazyArray16<'a, u16>,
    /// Nested lookups to apply.
    pub lookups: LazyArray16<'a, SequenceLookupRecord>,
}

impl<'a> FromSlice<'a> for ChainedSequenceRule<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let backtrack_count = r.u16()?;
        let backtrack = r.array(backtrack_count)?;
        let input_count = r.u16()?;
        let input = r.array(input_count.checked_sub(1)?)?;
        let lookahead_count = r.u16()?;
        let lookahead = r.array(lookahead_count)?;
        let lookup_count = r.u16()?;
        Some(ChainedSequenceRule {
            backtrack,
            input,
            lookahead,
            lookups: r.array(lookup_count)?,
        })
    }
}

/// The chained rules sharing one first glyph or class.
pub type ChainedSequenceRuleSet<'a> = OffsetArray16<'a, ChainedSequenceRule<'a>>;
/// Chained rule sets indexed by coverage index (format 1) or input class (format 2).
pub type ChainedSequenceRuleSets<'a> = OffsetArray16<'a, ChainedSequenceRuleSet<'a>>;

/// Chained context lookup (GSUB type 6).
#[derive(Clone, Copy, Debug)]
pub enum ChainedContextLookup<'a> {
    /// Format 1: rules keyed by glyph.
    Format1 {
        /// Glyphs that may start a rule.
        coverage: Coverage<'a>,
        /// Rule sets by coverage index.
        sets: ChainedSequenceRuleSets<'a>,
    },
    /// Format 2: rules keyed by glyph class.
    Format2 {
        /// Glyphs that may start a rule.
        coverage: Coverage<'a>,
        /// Classes for the backtrack sequence.
        backtrack_classes: ClassDefinition<'a>,
        /// Classes for the input sequence.
        input_classes: ClassDefinition<'a>,
        /// Classes for the lookahead sequence.
        lookahead_classes: ClassDefinition<'a>,
        /// Rule sets by input class.
        sets: ChainedSequenceRuleSets<'a>,
    },
    /// Format 3: one coverage per position.
    Format3 {
        /// Coverage of the first input glyph.
        coverage: Coverage<'a>,
        /// Backtrack coverages, nearest first.
        backtrack_coverages: OffsetArray16<'a, Coverage<'a>>,
        /// Coverages of the input glyphs after the first.
        input_coverages: OffsetArray16<'a, Coverage<'a>>,
        /// Lookahead coverages.
        lookahead_coverages: OffsetArray16<'a, Coverage<'a>>,
        /// Nested lookups to apply.
        lookups: LazyArray16<'a, SequenceLookupRecord>,
    },
}

impl<'a> FromSlice<'a> for ChainedContextLookup<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let at = |offset: u16| data.get(usize::from(offset)..);
        match r.u16()? {
            1 => {
                let coverage = Coverage::parse(at(r.u16()?)?)?;
                let count = r.u16()?;
                let sets = OffsetArray16::new(data, r.array(count)?);
                Some(ChainedContextLookup::Format1 { coverage, sets })
            }
            2 => {
                let coverage = Coverage::parse(at(r.u16()?)?)?;
                let backtrack_classes = ClassDefinition::parse(at(r.u16()?)?)?;
                let input_classes = ClassDefinition::parse(at(r.u16()?)?)?;
                let lookahead_classes = ClassDefinition::parse(at(r.u16()?)?)?;
                let count = r.u16()?;
                let sets = OffsetArray16::new(data, r.array(count)?);
                Some(ChainedContextLookup::Format2 {
                    coverage,
                    backtrack_classes,
                    input_classes,
                    lookahead_classes,
                    sets,
                })
            }
            3 => {
                let backtrack_count = r.u16()?;
                let backtrack = r.array::<u16>(backtrack_count)?;
                let input_count = r.u16()?;
                let input = r.array::<u16>(input_count)?;
                let lookahead_count = r.u16()?;
                let lookahead = r.array::<u16>(lookahead_count)?;
                let lookup_count = r.u16()?;
                let lookups = r.array(lookup_count)?;
                let coverage = Coverage::parse(at(input.get(0)?)?)?;
                Some(ChainedContextLookup::Format3 {
                    coverage,
                    backtrack_coverages: OffsetArray16::new(data, backtrack),
                    input_coverages: OffsetArray16::new(data, input.tail(1)),
                    lookahead_coverages: OffsetArray16::new(data, lookahead),
                    lookups,
                })
            }
            _ => None,
        }
    }
}
