#!/usr/bin/env python3
"""Records the fixtures in mod.rs from Google's `protobuf` for Python (upb backend):

    python3 -I gen.py > mod.rs

Timestamp, Any, Struct, Value and Empty are the library's own classes. `google.rpc.Status` and
`ErrorInfo` are built from descriptors (googleapis-common-protos is not needed):

    message Status { int32 code = 1; string message = 2; repeated google.protobuf.Any details = 3; }
    message ErrorInfo { string reason = 1; string domain = 2; map<string, string> metadata = 3; }
"""
from google.protobuf import (any_pb2, descriptor_pb2 as d, descriptor_pool, empty_pb2,
                             message_factory, struct_pb2, timestamp_pb2)
from google.protobuf.json_format import ParseDict

F = d.FieldDescriptorProto

fd = d.FileDescriptorProto(name="rpc.proto", package="google.rpc", syntax="proto3",
                           dependency=["google/protobuf/any.proto"])
st = fd.message_type.add(name="Status")
st.field.add(name="code", number=1, type=F.TYPE_INT32, label=F.LABEL_OPTIONAL)
st.field.add(name="message", number=2, type=F.TYPE_STRING, label=F.LABEL_OPTIONAL)
st.field.add(name="details", number=3, type=F.TYPE_MESSAGE, label=F.LABEL_REPEATED,
             type_name=".google.protobuf.Any")
ei = fd.message_type.add(name="ErrorInfo")
ei.field.add(name="reason", number=1, type=F.TYPE_STRING, label=F.LABEL_OPTIONAL)
ei.field.add(name="domain", number=2, type=F.TYPE_STRING, label=F.LABEL_OPTIONAL)
entry = ei.nested_type.add(name="MetadataEntry")
entry.options.map_entry = True
entry.field.add(name="key", number=1, type=F.TYPE_STRING, label=F.LABEL_OPTIONAL)
entry.field.add(name="value", number=2, type=F.TYPE_STRING, label=F.LABEL_OPTIONAL)
ei.field.add(name="metadata", number=3, type=F.TYPE_MESSAGE, label=F.LABEL_REPEATED,
             type_name=".google.rpc.ErrorInfo.MetadataEntry")

pool = descriptor_pool.DescriptorPool()
pool.AddSerializedFile(any_pb2.DESCRIPTOR.serialized_pb)
pool.Add(fd)
Status = message_factory.GetMessageClass(pool.FindMessageTypeByName("google.rpc.Status"))
ErrorInfo = message_factory.GetMessageClass(pool.FindMessageTypeByName("google.rpc.ErrorInfo"))


def ser(m):
    return m.SerializeToString(deterministic=True)


out = {}
out["EMPTY"] = ser(empty_pb2.Empty())
out["TIMESTAMP"] = ser(timestamp_pb2.Timestamp(seconds=1_700_000_000, nanos=123_456_789))
out["TIMESTAMP_NEGATIVE"] = ser(timestamp_pb2.Timestamp(seconds=-1, nanos=999_999_999))
out["TIMESTAMP_ZERO"] = ser(timestamp_pb2.Timestamp())
out["TIMESTAMP_SECONDS_ONLY"] = ser(timestamp_pb2.Timestamp(seconds=60))

v = struct_pb2.Value(); v.null_value = struct_pb2.NULL_VALUE
out["VALUE_NULL"] = ser(v)
out["VALUE_NUMBER"] = ser(struct_pb2.Value(number_value=-1.5))
out["VALUE_ZERO_NUMBER"] = ser(struct_pb2.Value(number_value=0.0))
out["VALUE_STRING"] = ser(struct_pb2.Value(string_value="héllo"))
out["VALUE_FALSE"] = ser(struct_pb2.Value(bool_value=False))
out["VALUE_TRUE"] = ser(struct_pb2.Value(bool_value=True))
out["VALUE_LIST"] = ser(ParseDict([1, "a", None, [True]], struct_pb2.Value()))
out["VALUE_STRUCT"] = ser(ParseDict({"k": 2}, struct_pb2.Value()))
out["VALUE_UNSET"] = ser(struct_pb2.Value())  # no kind set at all

doc = {"name": "x", "n": 1.5, "ok": True, "nil": None, "list": [1, "a", {"k": False}],
       "obj": {"a": {"b": []}}, "": 0}
out["STRUCT"] = ser(ParseDict(doc, struct_pb2.Struct()))
out["STRUCT_EMPTY"] = ser(struct_pb2.Struct())
# No key is a prefix of another: the reference orders these bytewise, like a BTreeMap. (upb puts
# a key *after* the keys it is a prefix of, so STRUCT above differs from us in order only.)
out["STRUCT_PLAIN"] = ser(ParseDict({"alpha": 1, "beta": "x", "gamma": [True, None]},
                                    struct_pb2.Struct()))

out["ANY"] = ser(any_pb2.Any(type_url="type.googleapis.com/test.Inner", value=b"\x08\x07"))
out["ANY_EMPTY"] = ser(any_pb2.Any())

info = ErrorInfo(reason="TASK_NOT_FOUND", domain="a2a-protocol.org")
info.metadata["task"] = "t-1"
info.metadata["attempt"] = "2"
out["ERROR_INFO"] = ser(info)
out["ERROR_INFO_NO_METADATA"] = ser(ErrorInfo(reason="R", domain="d"))

status = Status(code=5, message="task not found")
a = status.details.add()
a.type_url = "type.googleapis.com/google.rpc.ErrorInfo"
a.value = ser(info)
out["STATUS"] = ser(status)
out["STATUS_NO_DETAILS"] = ser(Status(code=3, message="bad"))
# a Status whose details hold two entries, one empty
s2 = Status(code=13)
s2.details.add(type_url="type.googleapis.com/x", value=b"\x01")
s2.details.add()
out["STATUS_TWO_DETAILS"] = ser(s2)

print("//! Buffers recorded from Google's `protobuf` for Python (upb); see `gen.py`.")
print("//! Regenerate with `python3 -I gen.py > mod.rs`; nothing here needs it at test time.")
print("#![allow(dead_code)]\n")
for k, b in out.items():
    print(f'pub const {k}: &str = "{b.hex()}";')
