/// A fixed-size value stored little-endian in a buffer.
pub trait Scalar: Copy + PartialEq {
    /// Size in bytes (also the alignment).
    const SIZE: usize;
    /// Decodes from the first `SIZE` bytes, or `None` if `bytes` is shorter.
    fn read_le(bytes: &[u8]) -> Option<Self>;
    /// Appends the bytes in reverse order, which is how the builder (back to front) writes.
    fn push_rev(self, rev: &mut Vec<u8>);
}

macro_rules! scalar {
    ($($t:ty),*) => {$(
        impl Scalar for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            fn read_le(bytes: &[u8]) -> Option<Self> {
                Some(<$t>::from_le_bytes(bytes.get(..Self::SIZE)?.try_into().ok()?))
            }
            fn push_rev(self, rev: &mut Vec<u8>) {
                rev.extend(self.to_le_bytes().iter().rev());
            }
        }
    )*};
}
scalar!(u8, i8, u16, i16, u32, i32, u64, i64, f32, f64);

impl Scalar for bool {
    const SIZE: usize = 1;
    fn read_le(bytes: &[u8]) -> Option<Self> {
        bytes.first().map(|b| *b != 0)
    }
    fn push_rev(self, rev: &mut Vec<u8>) {
        rev.push(u8::from(self));
    }
}
