//! Buffers recorded from Google's `protobuf` for Python (upb), proto3, from descriptors
//! built in `gen.py` (the schema is in its docstring). Regenerate with
//! `python3 -I gen.py > mod.rs`; nothing here depends on it at test time.
#![allow(dead_code)]

pub const SCALARS: &str = "08ffffffffffffffffff0110feffffffffffffffff0118ffffffff0f20ffffffffffffffffff01280530ffffffffffffffffff01380145efbeadde49ffffffffffffffff55fbffffff59faffffffffffffff650000c03f69000000000000d0bf";
pub const STRINGS: &str = "720a68c3a96c6c6f20e298837a0300ff10";
pub const NESTED: &str = "8201050807120178";
pub const REPEATED: &str = "8a010d01ac02ffffffffffffffffff0192010161920100920101629a010208019a0105080212016ec201080100000002000000ca0110000000000000f03f00000000000004c0d201080102808080808040";
pub const MAP_ONE: &str = "a201050a016b1001";
pub const MAP_TWO_SORTED: &str = "a201050a01611003a2010e0a016210feffffffffffffffff01";
pub const ONEOF_INT: &str = "a80105";
pub const ONEOF_STR: &str = "b201017a";
pub const ENUM: &str = "b80102";
pub const EMPTY: &str = "";
pub const EMPTY_NESTED: &str = "820100";
pub const FAR_FIELD: &str = "f8ffffff0f01";
pub const WIDE: &str =
    "0809120177188080808080808080800122027a7a2d07000000310800000000000000f8ffffff0f03";
pub const UNPACKED_REPEATED: &str = "880101880102880103";
pub const PACKED_TWICE: &str = "8a010201028a010103";
pub const OVERLONG_VARINT: &str = "088100";
pub const LAST_WINS: &str = "08010802";
pub const MERGED_SUBMESSAGE: &str = "8201020801820103120178";
pub const ONEOF_LAST_WINS: &str = "a80105b201017a";

/// `(name, hex, reference_rejects)`: all decoded as `All`. `strict_*` rows are accepted
/// by the reference and rejected here on purpose.
pub const MALFORMED: &[(&str, &str, bool)] = &[
    ("truncated_varint", "0880", true),
    ("truncated_tag", "80", true),
    ("len_past_end", "720561", true),
    ("len_huge", "72ffffffff0f61", true),
    ("truncated_fixed32", "45010203", true),
    ("truncated_fixed64", "4901020304050607", true),
    ("field_zero", "0000", true),
    ("wire_type_6", "0e", true),
    ("wire_type_7", "0f", true),
    ("invalid_utf8", "7202c328", true),
    ("varint_11_bytes", "088080808080808080808000", true),
    ("packed_varint_truncated", "8a01020180", true),
    ("packed_fixed32_bad_len", "c20103010203", true),
    ("packed_fixed64_bad_len", "ca010700000000000000", true),
    ("nested_truncated", "8201030801", true),
    ("nested_len_past_end", "8201090801", true),
    ("end_group_alone", "9c06", true),
    ("start_group_unterminated", "9b06", true),
    ("strict_group_well_formed", "9b0608019c06", false),
    ("strict_varint_overflow", "08ffffffffffffffffff7f", false),
    ("field_number_too_big", "8080808010", true),
];
