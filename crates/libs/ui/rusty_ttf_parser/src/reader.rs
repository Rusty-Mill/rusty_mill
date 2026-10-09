//! Bounds-checked big-endian readers.
//!
//! Nothing here panics, allocates or uses `unsafe`: every read returns `None`
//! when the bytes are not there, so a truncated or hostile font degrades to
//! "table absent" rather than a crash.

use core::marker::PhantomData;

/// Read a big-endian `u8` at byte offset `o`.
pub(crate) fn u8_at(d: &[u8], o: usize) -> Option<u8> {
    d.get(o).copied()
}

/// Read a big-endian `u16` at byte offset `o`.
pub(crate) fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    let b = d.get(o..o.checked_add(2)?)?;
    Some(u16::from_be_bytes(b.try_into().ok()?))
}

/// Read a big-endian `i16` at byte offset `o`.
pub(crate) fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}

/// Read a big-endian `u32` at byte offset `o`.
pub(crate) fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    let b = d.get(o..o.checked_add(4)?)?;
    Some(u32::from_be_bytes(b.try_into().ok()?))
}

/// The sub-slice of `d` starting at a 32-bit offset (`None` if out of range).
pub(crate) fn tail32(d: &[u8], offset: u32) -> Option<&[u8]> {
    d.get(usize::try_from(offset).ok()?..)
}

/// A fixed-size big-endian value stored in an array.
pub trait FromData: Sized {
    /// Encoded size in bytes.
    const SIZE: usize;
    /// Decode from exactly [`Self::SIZE`] bytes.
    fn parse(data: &[u8]) -> Option<Self>;
}

impl FromData for u16 {
    const SIZE: usize = 2;
    fn parse(data: &[u8]) -> Option<Self> {
        u16_at(data, 0)
    }
}

/// A table that can be decoded from the start of a byte slice.
pub trait FromSlice<'a>: Sized {
    /// Decode from `data`, which begins at the table's first byte.
    fn parse(data: &'a [u8]) -> Option<Self>;
}

/// A cursor over a slice; each read advances it.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub(crate) fn u16(&mut self) -> Option<u16> {
        let v = u16_at(self.data, self.pos)?;
        self.pos += 2;
        Some(v)
    }

    pub(crate) fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.pos.checked_add(n)?;
        (end <= self.data.len()).then(|| self.pos = end)
    }

    /// Read `count` consecutive `T`s as a lazy array.
    pub(crate) fn array<T: FromData>(&mut self, count: u16) -> Option<LazyArray16<'a, T>> {
        let bytes = usize::from(count) * T::SIZE;
        let slice = self.data.get(self.pos..self.pos.checked_add(bytes)?)?;
        self.pos += bytes;
        Some(LazyArray16 {
            data: slice,
            len: count,
            _marker: PhantomData,
        })
    }
}

/// A length-prefixed run of fixed-size values, decoded on access.
pub struct LazyArray16<'a, T> {
    data: &'a [u8],
    len: u16,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for LazyArray16<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for LazyArray16<'_, T> {}

impl<T> core::fmt::Debug for LazyArray16<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "LazyArray16(len = {})", self.len)
    }
}

impl<T> Default for LazyArray16<'_, T> {
    fn default() -> Self {
        LazyArray16 {
            data: &[],
            len: 0,
            _marker: PhantomData,
        }
    }
}

impl<'a, T: FromData> LazyArray16<'a, T> {
    /// Number of elements.
    pub fn len(&self) -> u16 {
        self.len
    }

    /// `true` when there are no elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Element `index`, or `None` past the end.
    pub fn get(&self, index: u16) -> Option<T> {
        if index >= self.len {
            return None;
        }
        let start = usize::from(index) * T::SIZE;
        T::parse(self.data.get(start..start + T::SIZE)?)
    }

    /// The array without its first `n` elements.
    pub(crate) fn tail(&self, n: u16) -> Self {
        let n = n.min(self.len);
        LazyArray16 {
            data: self.data.get(usize::from(n) * T::SIZE..).unwrap_or(&[]),
            len: self.len - n,
            _marker: PhantomData,
        }
    }

    /// Iterate over the elements.
    pub fn iter(&self) -> impl Iterator<Item = T> + 'a
    where
        T: 'a,
    {
        let array = *self;
        (0..array.len).filter_map(move |i| array.get(i))
    }

    /// First index whose element is not `Less` under `cmp` (a partition point),
    /// for sorted arrays.
    pub(crate) fn partition_point(&self, mut is_before: impl FnMut(&T) -> bool) -> u16 {
        let (mut lo, mut hi) = (0u16, self.len);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.get(mid) {
                Some(item) if is_before(&item) => lo = mid + 1,
                _ => hi = mid,
            }
        }
        lo
    }
}

/// An array of 16-bit offsets, each resolving to a `T` relative to `base`.
/// An offset of zero means "absent" and yields `None`.
pub struct OffsetArray16<'a, T> {
    base: &'a [u8],
    offsets: LazyArray16<'a, u16>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for OffsetArray16<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for OffsetArray16<'_, T> {}

impl<T> core::fmt::Debug for OffsetArray16<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "OffsetArray16(len = {})", self.offsets.len)
    }
}

impl<'a, T> OffsetArray16<'a, T> {
    pub(crate) fn new(base: &'a [u8], offsets: LazyArray16<'a, u16>) -> Self {
        OffsetArray16 {
            base,
            offsets,
            _marker: PhantomData,
        }
    }

    /// Number of offsets (including absent ones).
    pub fn len(&self) -> u16 {
        self.offsets.len
    }

    /// `true` when there are no offsets.
    pub fn is_empty(&self) -> bool {
        self.offsets.len == 0
    }
}

impl<'a, T: FromSlice<'a>> OffsetArray16<'a, T> {
    /// The table at index `index`, or `None` if absent or malformed.
    pub fn get(&self, index: u16) -> Option<T> {
        let offset = self.offsets.get(index)?;
        if offset == 0 {
            return None;
        }
        T::parse(self.base.get(usize::from(offset)..)?)
    }
}

/// A `count`-prefixed offset array whose offsets are relative to the array's
/// own start: the shape of a rule set or a ligature set.
impl<'a, T> FromSlice<'a> for OffsetArray16<'a, T> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let count = r.u16()?;
        let offsets = r.array::<u16>(count)?;
        Some(OffsetArray16::new(data, offsets))
    }
}
