#!/usr/bin/env python3
"""Wire-format vectors for every message in a .proto, produced by Google's Python protobuf
runtime (an implementation independent of `rusty_prost`), for a consumer's golden test.

    python3 -m venv venv && venv/bin/pip install grpcio-tools protobuf
    venv/bin/python rusty_prost/tools/gen_vectors.py --proto PATH/x.proto -I DIR [-I DIR ...] \\
        [--compile-all DIR] [--extra extra_vectors.py] --out x_vectors.json --label "<command>"

Each vector is {type, name, input, canonical} (hex). `input` is what a peer might send (possibly
not canonical: unknown fields, repeated occurrences, fields out of order); `canonical` is what
the reference parses it to and serializes back, with unknown fields discarded and map keys
sorted, which is what the consumer must re-encode. The random vectors are seeded, so a rerun
with the same runtime reproduces the file.

`--extra FILE` adds the consumer's hand-built edge cases: FILE is run with `add(type, name, raw)`
and the wire helpers `varint`, `tag`, `ld` (length-delimited field) and `vi` (varint field) in
scope. `--compile-all DIR` also compiles every .proto under DIR (for imports the proto needs).
"""
import argparse
import json
import os
import random
import sys
import tempfile

from google.protobuf import json_format, message_factory
from google.protobuf.descriptor import FieldDescriptor as FD
from grpc_tools import protoc

ap = argparse.ArgumentParser()
ap.add_argument("--proto", required=True)
ap.add_argument("-I", dest="includes", action="append", default=[])
ap.add_argument("--compile-all", action="append", default=[])
ap.add_argument("--extra")
ap.add_argument("--out", required=True)
ap.add_argument("--label", required=True, help="recorded in the file as `generator`")
ap.add_argument("--seed", type=int, default=20261021)
args = ap.parse_args()

tmp = tempfile.mkdtemp()
inc = os.path.join(os.path.dirname(__import__("grpc_tools").__file__), "_proto")
api = sorted(
    os.path.relpath(os.path.join(r, f), root)
    for root in args.compile_all
    for r, _, fs in os.walk(root)
    for f in fs
    if f.endswith(".proto")
)
proto = os.path.abspath(args.proto)
rc = protoc.main(
    ["protoc", f"-I{os.path.dirname(proto)}", *[f"-I{i}" for i in args.includes], f"-I{inc}",
     f"--python_out={tmp}", os.path.basename(proto), *api]
)
assert rc == 0, "protoc failed"
sys.path.insert(0, tmp)
pb2 = __import__(os.path.basename(proto)[: -len(".proto")] + "_pb2")

rng = random.Random(args.seed)
STRINGS = ["", "a", "hello", "ünïcödé ✓", "x" * 300, "task-1", "https://example.org/a?b=c", "\u0000nul", "日本語"]
INTS = [0, 1, -1, 7, 255, 65536, 2**31 - 1, -(2**31)]
JSONS = [None, True, False, 0, 1.5, -2, "s", [], [1, "a", None], {}, {"k": 1, "n": {"a": [True, 2.5]}}]


def cls(desc):
    return message_factory.GetMessageClass(desc)


def rand_scalar(f):
    t = f.type
    if t == FD.TYPE_STRING:
        return rng.choice(STRINGS)
    if t == FD.TYPE_BYTES:
        return bytes(rng.randrange(256) for _ in range(rng.randrange(0, 12)))
    if t == FD.TYPE_BOOL:
        return rng.choice([True, False])
    if t == FD.TYPE_INT32:
        return rng.choice(INTS)
    if t == FD.TYPE_ENUM:
        vals = [v.number for v in f.enum_type.values]
        return rng.choice(vals + [99])  # 99: a value the schema does not know
    raise AssertionError(f"scalar type {t}")


def fill_wkt(m):
    name = m.DESCRIPTOR.full_name
    if name == "google.protobuf.Struct":
        json_format.ParseDict(rng.choice([{}, {"a": 1}, {"k": "v", "n": [1, None, {"z": False}]}]), m)
    elif name == "google.protobuf.Value":
        json_format.ParseDict(rng.choice(JSONS), m)
    elif name == "google.protobuf.Timestamp":
        m.seconds = rng.choice([0, 1, 1700000000, 253402300799])
        m.nanos = rng.choice([0, 1, 999999999])
    # Empty: nothing


def fill(m, depth):
    d = m.DESCRIPTOR
    if d.full_name.startswith("google.protobuf."):
        fill_wkt(m)
        return
    chosen = {}
    for o in d.oneofs:
        if not o.fields[0].has_presence or o.name.startswith("_"):
            continue
        chosen[o.name] = rng.choice(o.fields)
    for f in d.fields:
        in_real_oneof = f.containing_oneof is not None and not f.containing_oneof.name.startswith("_")
        if in_real_oneof and chosen.get(f.containing_oneof.name) is not f:
            continue
        if not in_real_oneof and rng.random() < 0.35:
            continue
        if f.is_repeated and f.message_type is not None and f.message_type.GetOptions().map_entry:
            v = f.message_type.fields_by_name["value"]
            # The reference's "deterministic" map order is stable but not byte order (it puts
            # "aa" before "a"), and the wire format leaves map order open. So several entries
            # share a length, where every implementation agrees, and the odd keys (empty,
            # unicode, long) go one to a map.
            n = rng.randrange(0, 3)
            keys = rng.sample(["k0", "k1", "k2"], n) if n > 1 else [rng.choice(STRINGS)] * n
            for k in keys:
                if v.type == FD.TYPE_MESSAGE:
                    if depth > 0:
                        fill(getattr(m, f.name)[k], depth - 1)
                    else:
                        getattr(m, f.name)[k].SetInParent()
                else:
                    getattr(m, f.name)[k] = rand_scalar(v)
        elif f.is_repeated:
            for _ in range(rng.randrange(0, 3)):
                if f.type == FD.TYPE_MESSAGE:
                    if depth > 0:
                        fill(getattr(m, f.name).add(), depth - 1)
                else:
                    getattr(m, f.name).append(rand_scalar(f))
        elif f.type == FD.TYPE_MESSAGE:
            sub = getattr(m, f.name)
            if depth > 0 or f.message_type.full_name.startswith("google.protobuf."):
                fill(sub, max(depth - 1, 0))
            sub.SetInParent()
        else:
            setattr(m, f.name, rand_scalar(f))


def ser(m):
    return m.SerializeToString(deterministic=True)


def messages():
    out = []
    for name, d in pb2.DESCRIPTOR.message_types_by_name.items():
        out.append(d)
    return out


# ---- raw wire helpers for the hand-built vectors --------------------------------------------
def varint(n):
    out = bytearray()
    n &= (1 << 64) - 1
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def tag(number, wt):
    return varint(number << 3 | wt)


def ld(number, payload):
    return tag(number, 2) + varint(len(payload)) + payload


def vi(number, n):
    return tag(number, 0) + varint(n)


def canon(typ, raw):
    c = cls(pb2.DESCRIPTOR.message_types_by_name[typ])()
    c.ParseFromString(raw)
    c.DiscardUnknownFields()
    return ser(c)


vectors = []


def add(typ, name, raw, canonical=None):
    vectors.append({"type": typ, "name": name, "input": raw.hex(), "canonical": (canonical if canonical is not None else canon(typ, raw)).hex()})


for d in messages():
    C = cls(d)
    add(d.name, "empty", b"", ser(C()))
    for i in range(5):
        m = C()
        fill(m, 3)
        b = ser(m)
        add(d.name, f"random-{i}", b, b)


if args.extra:
    exec(compile(open(args.extra).read(), args.extra, "exec"), {"add": add, "varint": varint, "tag": tag, "ld": ld, "vi": vi})

json.dump({"generator": args.label, "protobuf": __import__("google.protobuf").protobuf.__version__, "vectors": vectors}, open(args.out, "w"), indent=0)
print(f"wrote {len(vectors)} vectors for {len(messages())} messages to {args.out}")
