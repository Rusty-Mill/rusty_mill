#!/usr/bin/env python3
"""Regenerates `a2a_vectors.json`: wire-format vectors for every message in spec/a2a.proto,
produced by Google's Python protobuf runtime (an implementation independent of this crate).

    python3 -m venv venv && venv/bin/pip install grpcio-tools protobuf
    venv/bin/python tests/golden/gen_vectors.py

Each vector is {type, name, input, canonical} (hex). `input` is what a peer might send (possibly
not canonical: unknown fields, repeated occurrences, fields out of order); `canonical` is what
the reference parses it to and serializes back, with unknown fields discarded and map keys
sorted, which is what this crate must re-encode. The random vectors are seeded, so a rerun with
the same runtime reproduces the file.
"""
import json
import os
import random
import sys
import tempfile

from google.protobuf import json_format, message_factory
from google.protobuf.descriptor import FieldDescriptor as FD
from grpc_tools import protoc

HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = os.path.join(HERE, "..", "..", "spec")
OUT = os.path.join(HERE, "a2a_vectors.json")

tmp = tempfile.mkdtemp()
inc = os.path.join(os.path.dirname(__import__("grpc_tools").__file__), "_proto")
api = sorted(os.path.relpath(os.path.join(r, f), SPEC + "/googleapis") for r, _, fs in os.walk(SPEC + "/googleapis") for f in fs if f.endswith(".proto"))
rc = protoc.main(["protoc", f"-I{SPEC}", f"-I{SPEC}/googleapis", f"-I{inc}", f"--python_out={tmp}", "a2a.proto", *api])
assert rc == 0, "protoc failed"
sys.path.insert(0, tmp)
import a2a_pb2  # noqa: E402

rng = random.Random(20261021)
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
    for name, d in a2a_pb2.DESCRIPTOR.message_types_by_name.items():
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
    c = cls(a2a_pb2.DESCRIPTOR.message_types_by_name[typ])()
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

# Hand-built: what a peer may send that is not the canonical encoding.
sub = ld(1, b"a") + ld(2, b"b")
add("Task", "unknown fields are dropped", ld(1, b"t1") + vi(99, 5) + ld(98, b"zz") + ld(2, b"c1"))
add("Task", "last occurrence of a scalar wins", ld(1, b"first") + ld(1, b"second"))
add("Task", "fields out of order", ld(2, b"ctx") + ld(1, b"id"))
add("Task", "sub-message occurrences merge", ld(3, vi(1, 2)) + ld(3, ld(2, ld(1, b"m1"))))
add("Task", "present but empty sub-message", ld(3, b""))
add("TaskStatus", "an enum value the schema does not know", vi(1, 99))
add("Part", "oneof: the last member wins", ld(1, b"text") + ld(3, b"http://u"))
add("Part", "oneof: a member set to its default is still set", ld(1, b""))
add("Part", "bytes member", ld(2, b"\x00\xff\x10"))
add("GetTaskRequest", "optional int32 present as zero", ld(2, b"t") + vi(3, 0))
add("GetTaskRequest", "optional int32 negative", vi(3, -5))
add("AgentCard", "map entries: a repeated key is replaced",
    ld(8, ld(1, b"k") + ld(2, ld(5, ld(1, b"d1")))) + ld(8, ld(1, b"k") + ld(2, ld(5, ld(1, b"d2")))))
add("AgentCard", "map entry with no value", ld(8, ld(1, b"only-key")))
add("AgentCard", "map entry with no key", ld(8, ld(2, ld(5, ld(1, b"d")))))
add("AgentCard", "map entries are sorted on re-encode", ld(8, ld(1, b"z") + ld(2, b"")) + ld(8, ld(1, b"a") + ld(2, b"")))
add("SecurityRequirement", "map of string lists", ld(1, ld(1, b"oauth") + ld(2, ld(1, b"read") + ld(1, b"write"))))
add("AuthorizationCodeOAuthFlow", "map of strings", ld(4, ld(1, b"s1") + ld(2, b"d1")) + ld(4, ld(1, b"s2") + ld(2, b"")))
add("OAuthFlows", "deprecated member", ld(3, ld(1, b"https://a")))
add("StreamResponse", "payload: the last member wins", ld(1, ld(1, b"t")) + ld(2, ld(1, b"m")))
add("StreamResponse", "payload: the same member twice merges", ld(1, ld(1, b"t")) + ld(1, ld(2, b"ctx")))
add("ListTaskPushNotificationConfigsRequest", "tenant has the highest field number", ld(4, b"acme") + ld(1, b"t") + vi(2, 7))
add("Message", "repeated strings keep their order", ld(7, b"e2") + ld(7, b"e1") + ld(8, b"r"))

json.dump({"generator": "tests/golden/gen_vectors.py", "protobuf": __import__("google.protobuf").protobuf.__version__, "vectors": vectors}, open(OUT, "w"), indent=0)
print(f"wrote {len(vectors)} vectors for {len(messages())} messages to {OUT}")
