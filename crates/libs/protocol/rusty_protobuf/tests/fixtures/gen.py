#!/usr/bin/env python3
"""Records the fixtures in mod.rs from the reference implementation (Google's `protobuf`
for Python, upb backend). Nothing in the Rust tests depends on it; it is here so the
recordings can be reproduced:

    python3 -I gen.py > mod.rs

Messages are built from a descriptor (no .proto, no protoc), proto3. The schema:

    message Inner { int32 id = 1; string name = 2; }
    enum Color { ZERO = 0; RED = 1; BLUE = 2; }
    message All {
      int32 i32 = 1; int64 i64 = 2; uint32 u32 = 3; uint64 u64 = 4; sint32 s32 = 5;
      sint64 s64 = 6; bool b = 7; fixed32 f32 = 8; fixed64 f64 = 9; sfixed32 sf32 = 10;
      sfixed64 sf64 = 11; float fl = 12; double db = 13; string s = 14; bytes by = 15;
      Inner inner = 16; repeated int32 packed = 17; repeated string strs = 18;
      repeated Inner inners = 19; map<string, int32> m = 20;
      oneof o { int32 oi = 21; string os = 22; }  Color e = 23;
      repeated fixed32 pf32 = 24; repeated double pdb = 25; repeated sint64 ps64 = 26;
    }
    message Wide {  // Inner plus fields Inner does not know
      int32 id = 1; string name = 2; uint64 extra = 3; bytes blob = 4; fixed32 f = 5;
      fixed64 d = 6; int32 far = 536870911;
    }
"""
from google.protobuf import descriptor_pb2 as d, descriptor_pool, message_factory
from google.protobuf.message import DecodeError

F = d.FieldDescriptorProto
T = F


def field(m, name, num, typ, label=F.LABEL_OPTIONAL, type_name=None, oneof=None):
    f = m.field.add(name=name, number=num, type=typ, label=label)
    if type_name:
        f.type_name = type_name
    if oneof is not None:
        f.oneof_index = oneof


fd = d.FileDescriptorProto(name="t.proto", package="t", syntax="proto3")
inner = fd.message_type.add(name="Inner")
field(inner, "id", 1, T.TYPE_INT32)
field(inner, "name", 2, T.TYPE_STRING)
color = fd.enum_type.add(name="Color")
for n, v in (("ZERO", 0), ("RED", 1), ("BLUE", 2)):
    color.value.add(name=n, number=v)
al = fd.message_type.add(name="All")
for name, num, typ in [
    ("i32", 1, T.TYPE_INT32), ("i64", 2, T.TYPE_INT64), ("u32", 3, T.TYPE_UINT32),
    ("u64", 4, T.TYPE_UINT64), ("s32", 5, T.TYPE_SINT32), ("s64", 6, T.TYPE_SINT64),
    ("b", 7, T.TYPE_BOOL), ("f32", 8, T.TYPE_FIXED32), ("f64", 9, T.TYPE_FIXED64),
    ("sf32", 10, T.TYPE_SFIXED32), ("sf64", 11, T.TYPE_SFIXED64), ("fl", 12, T.TYPE_FLOAT),
    ("db", 13, T.TYPE_DOUBLE), ("s", 14, T.TYPE_STRING), ("by", 15, T.TYPE_BYTES)]:
    field(al, name, num, typ)
field(al, "inner", 16, T.TYPE_MESSAGE, type_name=".t.Inner")
field(al, "packed", 17, T.TYPE_INT32, F.LABEL_REPEATED)
field(al, "strs", 18, T.TYPE_STRING, F.LABEL_REPEATED)
field(al, "inners", 19, T.TYPE_MESSAGE, F.LABEL_REPEATED, ".t.Inner")
entry = al.nested_type.add(name="MEntry")
entry.options.map_entry = True
field(entry, "key", 1, T.TYPE_STRING)
field(entry, "value", 2, T.TYPE_INT32)
field(al, "m", 20, T.TYPE_MESSAGE, F.LABEL_REPEATED, ".t.All.MEntry")
al.oneof_decl.add(name="o")
field(al, "oi", 21, T.TYPE_INT32, oneof=0)
field(al, "os", 22, T.TYPE_STRING, oneof=0)
field(al, "e", 23, T.TYPE_ENUM, type_name=".t.Color")
field(al, "pf32", 24, T.TYPE_FIXED32, F.LABEL_REPEATED)
field(al, "pdb", 25, T.TYPE_DOUBLE, F.LABEL_REPEATED)
field(al, "ps64", 26, T.TYPE_SINT64, F.LABEL_REPEATED)
wide = fd.message_type.add(name="Wide")
for name, num, typ in [
    ("id", 1, T.TYPE_INT32), ("name", 2, T.TYPE_STRING), ("extra", 3, T.TYPE_UINT64),
    ("blob", 4, T.TYPE_BYTES), ("f", 5, T.TYPE_FIXED32), ("d", 6, T.TYPE_FIXED64),
    ("far", 536870911, T.TYPE_INT32)]:
    field(wide, name, num, typ)

pool = descriptor_pool.DescriptorPool()
pool.Add(fd)
cls = {n: message_factory.GetMessageClass(pool.FindMessageTypeByName("t." + n))
       for n in ("Inner", "All", "Wide")}
Inner, All, Wide = cls["Inner"], cls["All"], cls["Wide"]


def ser(m):
    return m.SerializeToString(deterministic=True)


def hx(b):
    return b.hex()


def mk(c, **kw):
    m = c()
    for k, v in kw.items():
        setattr(m, k, v)
    return m


def varint(n):
    out = bytearray()
    while n > 0x7F:
        out.append((n & 0x7F) | 0x80)
        n >>= 7
    out.append(n)
    return bytes(out)


def tag(num, wt):
    return varint((num << 3) | wt)


golden = {}
golden["SCALARS"] = ser(mk(
    All, i32=-1, i64=-2, u32=4294967295, u64=2**64 - 1, s32=-3, s64=-2**63, b=True,
    f32=0xDEADBEEF, f64=2**64 - 1, sf32=-5, sf64=-6, fl=1.5, db=-0.25))
golden["STRINGS"] = ser(mk(All, s="héllo ☃", by=b"\x00\xff\x10"))
m = All(); m.inner.id = 7; m.inner.name = "x"
golden["NESTED"] = ser(m)
m = All(); m.packed.extend([1, 300, -1]); m.strs.extend(["a", "", "b"])
m.inners.add(id=1); m.inners.add(id=2, name="n")
m.pf32.extend([1, 2]); m.pdb.extend([1.0, -2.5]); m.ps64.extend([-1, 1, 2**40])
golden["REPEATED"] = ser(m)
m = All(); m.m["k"] = 1
golden["MAP_ONE"] = ser(m)
m = All(); m.m["b"] = -2; m.m["a"] = 3
golden["MAP_TWO_SORTED"] = ser(m)
golden["ONEOF_INT"] = ser(mk(All, oi=5))
golden["ONEOF_STR"] = ser(mk(All, os="z"))
golden["ENUM"] = ser(mk(All, e=2))
golden["EMPTY"] = ser(All())
m = All(); m.inner.SetInParent()
golden["EMPTY_NESTED"] = ser(m)
golden["FAR_FIELD"] = ser(mk(Wide, far=1))
# Decode-only: written by a newer schema; `Inner` must skip what it does not know.
golden["WIDE"] = ser(mk(Wide, id=9, name="w", extra=2**63, blob=b"zz", f=7, d=8, far=3))

# Hand-built buffers the reference accepts, with what it reads back.
accepted = {
    # field 17 as plain varints instead of one packed run: both are valid for a repeated int32
    "UNPACKED_REPEATED": (tag(17, 0) + b"\x01" + tag(17, 0) + b"\x02" + tag(17, 0) + b"\x03",
                          lambda r: list(r.packed) == [1, 2, 3]),
    # a run split across two packed records is concatenated
    "PACKED_TWICE": (tag(17, 2) + b"\x02\x01\x02" + tag(17, 2) + b"\x01\x03",
                     lambda r: list(r.packed) == [1, 2, 3]),
    "OVERLONG_VARINT": (tag(1, 0) + b"\x81\x00", lambda r: r.i32 == 1),
    "LAST_WINS": (tag(1, 0) + b"\x01" + tag(1, 0) + b"\x02", lambda r: r.i32 == 2),
    "MERGED_SUBMESSAGE": (tag(16, 2) + b"\x02\x08\x01" + tag(16, 2) + b"\x03\x12\x01x",
                          lambda r: r.inner.id == 1 and r.inner.name == "x"),
    "ONEOF_LAST_WINS": (tag(21, 0) + b"\x05" + tag(22, 2) + b"\x01z",
                        lambda r: r.WhichOneof("o") == "os"),
}
for name, (raw, check) in accepted.items():
    r = All(); r.ParseFromString(raw)
    assert check(r), name
    golden[name] = raw

# Malformed buffers: (name, bytes, does the reference reject it?). The reference verdict is
# computed here, not assumed. `strict_*` rows are ones the reference accepts and this crate
# rejects on purpose.
big = b"\xff" * 10
neg = [
    ("truncated_varint", tag(1, 0) + b"\x80"),
    ("truncated_tag", b"\x80"),
    ("len_past_end", tag(14, 2) + b"\x05a"),
    ("len_huge", tag(14, 2) + varint(2**32 - 1) + b"a"),
    ("truncated_fixed32", tag(8, 5) + b"\x01\x02\x03"),
    ("truncated_fixed64", tag(9, 1) + b"\x01\x02\x03\x04\x05\x06\x07"),
    ("field_zero", b"\x00\x00"),
    ("wire_type_6", tag(1, 6)),
    ("wire_type_7", tag(1, 7)),
    ("invalid_utf8", tag(14, 2) + b"\x02\xc3\x28"),
    ("varint_11_bytes", tag(1, 0) + b"\x80" * 10 + b"\x00"),
    ("packed_varint_truncated", tag(17, 2) + b"\x02\x01\x80"),
    ("packed_fixed32_bad_len", tag(24, 2) + b"\x03\x01\x02\x03"),
    ("packed_fixed64_bad_len", tag(25, 2) + b"\x07" + b"\x00" * 7),
    ("nested_truncated", tag(16, 2) + b"\x03\x08\x01"),
    ("nested_len_past_end", tag(16, 2) + b"\x09\x08\x01"),
    ("end_group_alone", tag(99, 4)),
    ("start_group_unterminated", tag(99, 3)),
    ("strict_group_well_formed", tag(99, 3) + tag(1, 0) + b"\x01" + tag(99, 4)),
    ("strict_varint_overflow", tag(1, 0) + big[:-1] + b"\x7f"),
    ("field_number_too_big", varint((2**29) << 3)),
]
rows = []
for name, raw in neg:
    try:
        All().ParseFromString(raw)
        rejects = False
    except DecodeError:
        rejects = True
    rows.append((name, raw, rejects))
    assert rejects or name.startswith("strict_"), "reference accepted " + name

print("//! Buffers recorded from Google's `protobuf` for Python (upb), proto3, from descriptors")
print("//! built in `gen.py` (the schema is in its docstring). Regenerate with")
print("//! `python3 -I gen.py > mod.rs`; nothing here depends on it at test time.")
print("#![allow(dead_code)]\n")
for k, v in golden.items():
    print(f'pub const {k}: &str = "{hx(v)}";')
print("\n/// `(name, hex, reference_rejects)`: all decoded as `All`. `strict_*` rows are accepted")
print("/// by the reference and rejected here on purpose.")
print("pub const MALFORMED: &[(&str, &str, bool)] = &[")
for name, raw, rej in rows:
    print(f'    ("{name}", "{hx(raw)}", {str(rej).lower()}),')
print("];")
