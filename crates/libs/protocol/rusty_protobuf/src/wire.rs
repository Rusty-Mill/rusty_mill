/// The largest field number the format allows (2^29 - 1).
pub const MAX_FIELD_NUMBER: u32 = (1 << 29) - 1;

/// How deeply messages may nest before the reader refuses, the same default as the reference
/// implementations.
pub const DEFAULT_DEPTH_LIMIT: usize = 100;

/// How a field's value is laid out on the wire. Groups (3 and 4) are not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireType {
    /// Base-128 varint: `int32`, `int64`, `uint32`, `uint64`, `sint32`, `sint64`, `bool`, enums.
    Varint = 0,
    /// Eight bytes, little-endian: `fixed64`, `sfixed64`, `double`.
    Fixed64 = 1,
    /// A varint length, then that many bytes: strings, bytes, messages, packed runs.
    Len = 2,
    /// Four bytes, little-endian: `fixed32`, `sfixed32`, `float`.
    Fixed32 = 5,
}
