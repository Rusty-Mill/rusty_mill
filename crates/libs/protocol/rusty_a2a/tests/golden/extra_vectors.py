"""The a2a hand-built vectors: what a peer may send that is not the canonical encoding.
Run by `rusty_prost/tools/gen_vectors.py --extra` (see its docstring for the command)."""
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

