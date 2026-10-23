"""The ext_mcp hand-built vectors: what a processor or a gateway may send that is not the
canonical encoding. Run by `rusty_prost/tools/gen_vectors.py --extra` (command in README.md)."""
add("McpRequest", "unknown fields are dropped", ld(2, b"tools/call") + vi(99, 5) + ld(98, b"zz") + ld(1, b"alpha"))
add("McpRequest", "service names keep their order", ld(1, b"b") + ld(1, b"a") + ld(1, b"c"))
add("McpRequest", "params present but empty", ld(2, b"ping") + ld(4, b""))
add("McpRequest", "params absent", ld(2, b"ping"))
add("McpRequest", "last occurrence of a scalar wins", ld(2, b"first") + ld(2, b"second"))
add("McpRequest", "metadata context occurrences merge",
    ld(3, ld(1, ld(1, b"a") + ld(2, ld(3, b"x")))) + ld(3, ld(1, ld(1, b"b") + ld(2, vi(4, 1)))))
add("McpRequest", "headers repeat, one entry per value",
    ld(5, ld(1, b"x-t") + ld(2, b"1")) + ld(5, ld(1, b"x-t") + ld(2, b"2")))
add("McpRequest", "header value that is not UTF-8", ld(5, ld(1, b"x-b") + ld(2, b"\xff\xfe\x00")))
add("McpResponse", "empty result bytes are omitted", ld(2, b"m") + ld(4, b""))
add("McpResponse", "fields out of order", ld(4, b"{}") + ld(2, b"m") + ld(1, b"s"))
add("McpRequestResult", "oneof: the last member wins", ld(1, b"") + ld(2, b"{}"))
add("McpRequestResult", "oneof: pass is present when empty", ld(1, b""))
add("McpRequestResult", "oneof: the same member twice merges", ld(3, vi(1, 1)) + ld(3, ld(2, b"why")))
add("McpRequestResult", "mutation and metadata",
    ld(2, b"{}") + ld(4, ld(1, ld(1, b"k") + ld(2, b"v")) + ld(2, b"X-Drop")) + ld(5, ld(1, ld(1, b"a") + ld(2, vi(4, 1)))))
add("McpResponseResult", "oneof: error then pass", ld(3, vi(1, 2)) + ld(1, b""))
add("AuthorizationError", "a code the schema does not know", vi(1, 99))
add("AuthorizationError", "optional error payload present but empty", ld(3, b""))
add("AuthorizationError", "negative code", vi(1, -1))
add("HeaderMutation", "removals keep their order", ld(2, b"b") + ld(2, b"a"))
add("HeaderMutation", "unknown fields are dropped", ld(1, ld(1, b"k") + ld(2, b"v")) + ld(3, b"x") + vi(9, 1))
