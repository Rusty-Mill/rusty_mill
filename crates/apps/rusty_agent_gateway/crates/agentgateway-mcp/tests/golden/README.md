# Golden wire vectors

`ext_mcp_vectors.json` holds wire-format vectors for every message in `proto/ext_mcp.proto`,
produced by Google's Python protobuf runtime, which is independent of `rusty_prost`.
`tests/wire_vectors.rs` checks the messages in `src/guardrails/wire.rs` against them.

To regenerate (from the repository root):

    python3 -m venv venv && venv/bin/pip install grpcio-tools "protobuf==7.36.2"
    venv/bin/python crates/libs/protocol/rusty_prost/tools/gen_vectors.py \
        --proto crates/apps/rusty_agent_gateway/crates/agentgateway-mcp/proto/ext_mcp.proto \
        --extra crates/apps/rusty_agent_gateway/crates/agentgateway-mcp/tests/golden/extra_vectors.py \
        --out crates/apps/rusty_agent_gateway/crates/agentgateway-mcp/tests/golden/ext_mcp_vectors.json \
        --label "agentgateway-mcp/tests/golden/README.md"

`extra_vectors.py` holds the hand-built edge cases; the random ones are seeded. A different
protobuf runtime version may change the `protobuf` field of the file, not the vectors.
