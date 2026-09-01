# rusty_meshed capability-manifest.md

Source: baileyrd/meshed (Python, v1.0 shipped) → Rusty-Mill/rusty_mill, crates/rusty_meshed/ (multi-crate namespace).

| ID | Capability | Category | Source | Existing RustyMill impl | Status | Reason (if OUT-OF-SCOPE) | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| REG-001 | App metadata: title "Meshed Data Product Registry", description, version "0.1.0" exposed via OpenAPI | interface | code | — | REQUIRED | | |
| REG-002 | App factory constructs SQLite engine from `f"sqlite:///{config.registry_db_path}"` with `connect_args={"check_same_thread": False}` | config | code | — | REQUIRED | | |
| REG-003 | Lifespan startup runs `SQLModel.metadata.create_all(engine)`, auto-creating all registered tables on app startup (no migrations) | behavior | code | — | REQUIRED | | |
| REG-004 | Lifespan startup imports side-effect modules (observability.metrics, registry.models, governance.rbac, infrastructure.outbox, transformation.models) purely to populate `SQLModel.metadata` before `create_all` | behavior | code | — | REQUIRED | | |
| REG-005 | CORS middleware allows only origin `http://localhost:5173`, all methods (`*`), all headers (`*`) | config | code | — | REQUIRED | | |
| REG-006 | Routers registered on the app: data_products, ports, contracts, access_grants, metrics, governance, lineage, monitor, transformation | interface | code | — | REQUIRED | | |
| REG-007 | `get_session` raises `RuntimeError("Database engine is not initialized...")` if `set_engine` was never called before first request | behavior | code | — | REQUIRED | | |
| REG-008 | `get_config` constructs a fresh `PlatformConfig()` per request so env var changes are picked up without process restart | behavior | code | — | REQUIRED | | |
| REG-009 | `PlatformConfig.registry_db_path` default `"meshed_registry.db"`, overridable via `MESHED_REGISTRY_DB_PATH` | config | code+test | — | REQUIRED | | |
| REG-010 | `PlatformConfig.registry_base_url` default `"http://localhost:8000"`, overridable via `MESHED_REGISTRY_BASE_URL` | config | code | — | REQUIRED | | |
| REG-011 | `PlatformConfig.kafka_bootstrap_servers` default `"localhost:9092"`, overridable via `MESHED_KAFKA_BOOTSTRAP_SERVERS`, consumed by the metrics endpoint | config | code | — | REQUIRED | | |
| REG-012 | `MaturityTier` str-enum values: `mvp`, `enhanced`, `mature` | interface | code | — | REQUIRED | | |
| REG-013 | `EventType` str-enum values: `delta`, `state`, `measurement` | interface | code | — | REQUIRED | | |
| REG-014 | `DataProduct.name` is indexed but NOT unique | behavior | code | — | REQUIRED | | |
| REG-015 | `DataProduct.domain` is indexed for discovery | behavior | code | — | REQUIRED | | |
| REG-016 | `DataProduct.maturity_tier` defaults to `MaturityTier.MVP` when omitted | behavior | code+test | — | REQUIRED | | |
| REG-017 | `DataProduct.tags` defaults to `"[]"` (JSON-encoded empty list string) when omitted | behavior | code | — | REQUIRED | | |
| REG-018 | `DataProduct → InputPort` relationship cascades `"all, delete-orphan"` | behavior | code | — | REQUIRED | | |
| REG-019 | `DataProduct → OutputPort` relationship cascades `"all, delete-orphan"` | behavior | code | — | REQUIRED | | |
| REG-020 | `OutputPort → DataContract` relationship cascades `"all, delete-orphan"` | behavior | code | — | REQUIRED | | |
| REG-021 | `InputPort.data_product_id` is a required FK to `data_products.id` | behavior | code | — | REQUIRED | | |
| REG-022 | `OutputPort.data_product_id` is a required FK to `data_products.id` | behavior | code | — | REQUIRED | | |
| REG-023 | `DataContract.output_port_id` is a UNIQUE FK to `output_ports.id`, one-contract-per-port at the DB schema level | behavior | code | — | REQUIRED | | |
| REG-024 | `DataContract.slo_completeness_pct` constrained to `[0.0, 100.0]` via `Field(ge=0.0, le=100.0)` at the model layer | behavior | code | — | REQUIRED | | |
| REG-025 | `DataContract.quality_assertions` stored as JSON-encoded string, model-layer default `"[]"` | behavior | code | — | REQUIRED | | |
| REG-026 | `DataProductUpdate`: all fields Optional; PATCH applies only fields explicitly present (`exclude_unset=True` semantics) | behavior | code | — | REQUIRED | | |
| REG-027 | `DataContractCreate.slo_completeness_pct` constrained `[0.0, 100.0]` at the schema layer — out-of-range returns 422 | behavior | code | — | REQUIRED | | |
| REG-028 | `DataContractCreate` validator rejects `schema_ref` empty/all-whitespace → 422 | behavior | code+test | — | REQUIRED | | |
| REG-029 | `DataContractCreate` validator rejects `owner` empty/all-whitespace → 422 | behavior | code+test | — | REQUIRED | | |
| REG-030 | `DataContractCreate` validator rejects an empty `quality_assertions` list → 422 | behavior | code+test | — | REQUIRED | | |
| REG-031 | `DataContractPublic.quality_assertions` always returned as `list[str]`, decoded from JSON string storage | behavior | code+test | — | REQUIRED | | |
| REG-032 | `InputPortCreate.description` optional, defaults to `null` | interface | code | — | REQUIRED | | |
| REG-033 | `OutputPortCreate.event_type` required, must be valid `EventType` member — invalid values return 422 | interface | code | — | REQUIRED | | |
| REG-034 | `POST /data-products/` returns 201 with created record including DB-assigned `id` | interface | code+test | — | REQUIRED | | |
| REG-035 | `POST /data-products/` runs `_DEFAULT_ENGINE.evaluate(...)` before insert; 422 `{"detail": {"governance_violations": [...]}}` on violation | interface | code+test | — | REQUIRED | | |
| REG-036 | Governance policy `require_description_min_length`: description ≥ 20 chars, else `"description must be at least 20 characters (got N)"` | behavior | code+test | — | REQUIRED | | |
| REG-037 | Governance policy `require_semantic_version`: version must match `\d+\.\d+\.\d+`, else `"version must be a semantic version (MAJOR.MINOR.PATCH), got '...'"` | behavior | code+test | — | REQUIRED | | |
| REG-038 | Governance policy `require_domain_lowercase`: domain must be all-lowercase, else `"domain must be lowercase, got '...'"` | behavior | code+test | — | REQUIRED | | |
| REG-039 | `GovernanceEngine.evaluate` catches `ValueError`/`AssertionError` from a policy, records exception message as violation | behavior | code+test | — | REQUIRED | | |
| REG-040 | `GovernanceEngine.evaluate` catches other exceptions, wraps as `"[POLICY-ERROR] {policy_name}: {exc}"` | behavior | code+test | — | REQUIRED | | |
| REG-041 | `GovernanceEngine.evaluate` runs all policies (no short-circuit), returns every violation message | behavior | code+test | — | REQUIRED | | |
| REG-042 | `GovernanceEngine.register_policy` appends a policy callable at runtime, applied on next `evaluate` call | behavior | code+test | — | REQUIRED | | |
| REG-043 | Governance policies evaluated via a single shared module-level `_DEFAULT_ENGINE` singleton, affecting both `POST /data-products` and `POST /governance/evaluate` | behavior | code | — | REQUIRED | | |
| REG-044 | `GET /data-products/` supports `domain` filter (exact match) | interface | code+test | — | REQUIRED | | |
| REG-045 | `GET /data-products/` supports `owner` filter (exact match) | interface | code+test | — | REQUIRED | | |
| REG-046 | `GET /data-products/` supports `tag` filter (substring match against JSON-encoded `tags` via SQL LIKE/contains()) | interface | code+test | — | REQUIRED | | |
| REG-047 | `GET /data-products/` supports `event_type` filter via outer JOIN to output_ports + DISTINCT; product must have ≥1 matching output port | interface | code+test | — | REQUIRED | | |
| REG-048 | `GET /data-products/` filters are additive/AND-combined when supplied together | behavior | code | — | REQUIRED | | |
| REG-049 | `GET /data-products/` `offset` query param: default 0, `ge=0` (negative → 422) | interface | code | — | REQUIRED | | |
| REG-050 | `GET /data-products/` `limit` query param: default 100, `le=100` only — no lower bound | interface | code | — | REQUIRED | | |
| REG-051 | `GET /data-products/{id}` returns 404 `"Data product not found"` when id doesn't exist | interface | code+test | — | REQUIRED | | |
| REG-052 | `PATCH /data-products/{id}` returns 404 `"Data product not found"` when id doesn't exist | interface | code | — | REQUIRED | | |
| REG-053 | `PATCH /data-products/{id}` applies only fields present in the update body | behavior | code | — | REQUIRED | | |
| REG-054 | `PATCH /data-products/{id}` does NOT re-run governance policy evaluation (only POST enforces governance) | behavior | code | — | REQUIRED | | |
| REG-055 | `DELETE /data-products/{id}` returns 204, cascades deletion of input ports, output ports, and (transitively) contracts | interface | code+docs | — | REQUIRED | | |
| REG-056 | `DELETE /data-products/{id}` returns 404 `"Data product not found"` when id doesn't exist | interface | code | — | REQUIRED | | |
| REG-057 | `POST /data-products/` does not enforce uniqueness on `name` — duplicates permitted, no 409 path | behavior | code | — | REQUIRED | | |
| REG-058 | `POST /data-products/` returns 422 if required field (name/owner/version/domain/description) missing | interface | code+test | — | REQUIRED | | |
| REG-059 | `POST /data-products/{id}/input-ports` returns 201 with assigned `id`/`data_product_id` | interface | code+test | — | REQUIRED | | |
| REG-060 | `POST /data-products/{id}/input-ports` returns 404 `"Data product {id} not found."` when parent missing | interface | code+test | — | REQUIRED | | |
| REG-061 | `POST /data-products/{id}/input-ports` stores optional `description`; response echoes `null` when omitted | interface | code+test | — | REQUIRED | | |
| REG-062 | `GET /data-products/{id}/input-ports` returns 404 when parent product doesn't exist | interface | code | — | REQUIRED | | |
| REG-063 | `GET /data-products/{id}/input-ports` lists only ports scoped to given product | interface | code+test | — | REQUIRED | | |
| REG-064 | `DELETE /data-products/{id}/input-ports/{port_id}` returns 204 on success | interface | code+test | — | REQUIRED | | |
| REG-065 | `DELETE .../input-ports/{port_id}` returns 404 if parent product doesn't exist | interface | code | — | REQUIRED | | |
| REG-066 | `DELETE .../input-ports/{port_id}` returns 404 if port doesn't exist or belongs to a different product | interface | code+test | — | REQUIRED | | |
| REG-067 | `POST /data-products/{id}/output-ports` returns 201 with `id`/`data_product_id`/`event_type` | interface | code+test | — | REQUIRED | | |
| REG-068 | `POST .../output-ports` returns 404 when parent product doesn't exist | interface | code+test | — | REQUIRED | | |
| REG-069 | `POST .../output-ports` accepts `event_type` of delta/state/measurement, round-trips verbatim | interface | code+test | — | REQUIRED | | |
| REG-070 | `POST .../output-ports` returns 422 if `event_type` not a valid enum value | interface | code | — | REQUIRED | | |
| REG-071 | `GET .../output-ports` returns 404 when parent product doesn't exist | interface | code | — | REQUIRED | | |
| REG-072 | `GET .../output-ports` lists only ports scoped to given product | interface | code+test | — | REQUIRED | | |
| REG-073 | `DELETE .../output-ports/{port_id}` returns 204 on success | interface | code+test | — | REQUIRED | | |
| REG-074 | `DELETE .../output-ports/{port_id}` returns 404 if parent product doesn't exist | interface | code | — | REQUIRED | | |
| REG-075 | `DELETE .../output-ports/{port_id}` returns 404 if port doesn't exist or belongs to a different product | interface | code+test | — | REQUIRED | | |
| REG-076 | `POST .../output-ports/{port_id}/contract` returns 201 when product+port exist and no contract yet | interface | code+test | — | REQUIRED | | |
| REG-077 | `POST .../contract` returns 404 `"Data product {id} not found."` if product missing | interface | code+test | — | REQUIRED | | |
| REG-078 | `POST .../contract` returns 404 `"Output port {id} not found on data product {id}."` if port missing/mismatched | interface | code+test | — | REQUIRED | | |
| REG-079 | `POST .../contract` returns 409 `"...already has a registered data contract."` if contract exists | interface | code+test | — | REQUIRED | | |
| REG-080 | `POST .../contract` stores `quality_assertions` as `json.dumps(list)` | behavior | code | — | REQUIRED | | |
| REG-081 | `GET .../contract` returns 200 with `quality_assertions` decoded to `list[str]` | interface | code+test | — | REQUIRED | | |
| REG-082 | `GET .../contract` returns 404 `"No data contract found for output port {id}."` when none exists | interface | code+test | — | REQUIRED | | |
| REG-083 | `GET .../contract` returns 404 if product or port missing/mismatched | interface | code | — | REQUIRED | | |
| REG-084 | `DELETE .../contract` returns 204, removes `DataContract` row | interface | code+test | — | REQUIRED | | |
| REG-085 | `DELETE .../contract` returns 404 when no contract exists | interface | code+test | — | REQUIRED | | |
| REG-086 | `DELETE .../contract` returns 404 if product or port missing/mismatched | interface | code | — | REQUIRED | | |
| REG-087 | `POST /access-grants` returns 201 with assigned `id` and server-generated `granted_at` (ISO-8601 UTC) | interface | code+test | — | REQUIRED | | |
| REG-088 | `POST /access-grants` returns 404 `"Output port {id} not found."` if port missing | interface | code+test | — | REQUIRED | | |
| REG-089 | `POST /access-grants` returns 409 if grant exists for same `(output_port_id, consumer_group_id)` | interface | code+test | — | REQUIRED | | |
| REG-090 | `PortAccessGrant.granted_at` default_factory produces `datetime.now(timezone.utc).isoformat()` | behavior | code | — | REQUIRED | | |
| REG-091 | `PortAccessGrant.output_port_id`/`consumer_group_id` both indexed; no DB-level composite uniqueness — enforced only at API layer via 409 check | behavior | code | — | REQUIRED | | |
| REG-092 | `GET /access-grants` lists all grants with no filters | interface | code+test | — | REQUIRED | | |
| REG-093 | `GET /access-grants?output_port_id=X` filters to that port | interface | code+test | — | REQUIRED | | |
| REG-094 | `GET /access-grants?consumer_group_id=X` filters to that consumer group | interface | code+test | — | REQUIRED | | |
| REG-095 | `GET /access-grants` supports combining both filters (AND) | behavior | code | — | REQUIRED | | |
| REG-096 | `GET /access-grants` has no pagination — returns all matching rows unbounded | behavior | code | — | REQUIRED | | |
| REG-097 | `DELETE /access-grants/{id}` returns 204 on success | interface | code+test | — | REQUIRED | | |
| REG-098 | `DELETE /access-grants/{id}` returns 404 `"Access grant {id} not found."` when missing | interface | code+test | — | REQUIRED | | |
| REG-099 | `GET .../output-ports/{id}/resolve?consumer_group_id=X` returns 200 `{"topic_name":..., "schema_subject":...}` when active grant exists | interface | code+test | — | REQUIRED | | |
| REG-100 | `GET .../resolve` returns 403 `"Consumer group '...' does not have access to output port {id}."` when no matching grant | interface | code+test | — | REQUIRED | | |
| REG-101 | `GET .../resolve` returns 404 if product doesn't exist (checked before port/grant lookups) | interface | code+test | — | REQUIRED | | |
| REG-102 | `GET .../resolve` returns 404 if port doesn't exist or doesn't belong to the product (checked before grant lookup) | interface | code+test | — | REQUIRED | | |
| REG-103 | `consumer_group_id` required query param on resolve endpoint (422 if omitted) | interface | code | — | REQUIRED | | |
| REG-104 | `POST /governance/evaluate` always returns 200, even for failing payloads — non-destructive dry-run | interface | code+test | — | REQUIRED | | |
| REG-105 | `POST /governance/evaluate` returns `{"violations": [...], "passed": bool}` | interface | code+test | — | REQUIRED | | |
| REG-106 | `POST /governance/evaluate` does not persist anything | behavior | code+docs | — | REQUIRED | | |
| REG-107 | `GET /lineage/topology` returns `{"dependencies": [...]}` from `LineageTracker.get_topology_dependencies()` against `meshed_registry.db` by default | interface | code | — | REQUIRED | | |
| REG-108 | `GET /lineage/record/{correlation_id}` returns `{"correlation_id":..., "events": [...]}`, `events=[]` (HTTP 200) for unknown id | interface | code | — | REQUIRED | | |
| REG-109 | `lineage.py` module-level `_DEFAULT_DB_PATH` hardcodes `"meshed_registry.db"`, not sourced from `PlatformConfig`/env | config | code | — | REQUIRED | | |
| REG-110 | `GET /data-products/{id}/metrics` returns 404 `"Data product not found"` if product missing | interface | code | — | REQUIRED | | |
| REG-111 | `GET /data-products/{id}/metrics` returns 404 `"Data product has no output ports"` if zero output ports | interface | code | — | REQUIRED | | |
| REG-112 | `GET /data-products/{id}/metrics` always measures `product.output_ports[0]` only | behavior | code | — | REQUIRED | | |
| REG-113 | `GET /data-products/{id}/metrics` accepts `group_id` query param, default `"default"` | interface | code | — | REQUIRED | | |
| REG-114 | `GET /data-products/{id}/metrics` accepts `num_partitions` query param, default `1` | interface | code | — | REQUIRED | | |
| REG-115 | `GET .../metrics` catches `KafkaException`, sets `lag=-1`, `throughput=-1`, includes `"error"` field — returns HTTP 200 | behavior | code | — | REQUIRED | | |
| REG-116 | `GET .../metrics` computes `violation_count` independently of Kafka reachability | behavior | code | — | REQUIRED | | |
| REG-117 | `GET .../metrics` response omits `"error"` key on success | behavior | code | — | REQUIRED | | |
| REG-118 | `GET /monitor/topology` classifies a product as producer/consumer/processor | behavior | code+test | — | REQUIRED | | |
| REG-119 | `GET /monitor/topology` dedupes broker nodes by topic name | behavior | code+test | — | REQUIRED | | |
| REG-120 | `GET /monitor/topology` creates one edge per output port (product→topic) and input port (topic→product) | behavior | code+test | — | REQUIRED | | |
| REG-121 | `GET /monitor/topology` broker node label is last dot-delimited segment of topic name | behavior | code | — | REQUIRED | | |
| REG-122 | `GET /monitor/topology` assigns x/y SVG coordinates via 4-column layout (producer x=90, broker x=350, processor x=560, consumer x=720; y spaced across 500px viewBox, 50px padding) | behavior | code+test | — | REQUIRED | | |
| REG-123 | `GET /monitor/topology` places a lone node in a column at `y = viewbox_height // 2` (250) | behavior | code | — | REQUIRED | | |
| REG-124 | Empty registry returns `{"nodes": [], "edges": []}` from `/monitor/topology` | interface | code+test | — | REQUIRED | | |
| REG-125 | `GET /monitor/events` SSE stream, headers `Cache-Control: no-cache`, `Connection: keep-alive`, `X-Accel-Buffering: no` | interface | code | — | REQUIRED | | |
| REG-126 | SSE `_event_generator` polls `lineage_records` every 1.0s | behavior | code | — | REQUIRED | | |
| REG-127 | SSE `_event_generator` records `last_id` at startup from `MAX(id)` to avoid replaying history | behavior | code | — | REQUIRED | | |
| REG-128 | SSE `_event_generator` fetches at most 50 new rows per poll cycle | behavior | code | — | REQUIRED | | |
| REG-129 | SSE `_event_generator` yields `": heartbeat\n\n"` when no new events found | behavior | code+test | — | REQUIRED | | |
| REG-130 | SSE `_event_generator` swallows all exceptions from polling queries rather than terminating | behavior | code | — | REQUIRED | | |
| REG-131 | SSE `_event_generator` formats each event as `"data: {json}\n\n"` | behavior | code | — | REQUIRED | | |
| REG-132 | SSE lineage payload shape: `{type, from, fromType:"producer", to, toType:"broker", lat:0, kb:0, isErr:false, eventId, timestamp}` — lat/kb/isErr hardcoded | behavior | code | — | REQUIRED | | |
| REG-133 | `GET /monitor/metrics` returns counts: data_products, input_ports, output_ports, contracts, schema_violations (SQLModel) + lineage_events/lineage_records (raw sqlite3) + total_flows | interface | code+test | — | REQUIRED | | |
| REG-134 | `GET /monitor/metrics` defaults lineage_events/lineage_records to 0 if raw sqlite3 query raises | behavior | code | — | REQUIRED | | |
| REG-135 | `monitor.py` module-level `_DEFAULT_DB_PATH` hardcodes `"meshed_registry.db"` independently of `lineage.py`'s own copy | config | code | — | REQUIRED | | |
| REG-136 | `GET /docs` returns 200 (Swagger UI enabled) | interface | test | — | REQUIRED | | |
| REG-137 | `GET /openapi.json` returns valid JSON with a `"paths"` key | interface | test | — | REQUIRED | | |
| REG-138 | `DataProductPublic.tags` returned as raw JSON-encoded string (not decoded), unlike `DataContractPublic.quality_assertions` | behavior | code | — | REQUIRED | | |
| REG-139 | `DataContractCreate.slo_freshness_seconds` plain int, no non-negativity constraint | behavior | code | — | REQUIRED | | |
| REG-140 | `SchemaRegistryEnforcer.DEFAULT_COMPATIBILITY = CompatibilityMode.FULL_TRANSITIVE` | config | code | — | DONE | | `rusty-meshed-schema-registry::SchemaRegistryEnforcer::DEFAULT_COMPATIBILITY` |
| REG-141 | `SchemaRegistryEnforcer(url, client=None)` constructs its own `SchemaRegistryClient({"url": url})` when no client injected | config | code | — | DONE | | `rusty-meshed-schema-registry::SchemaRegistryEnforcer::new`/`with_client` |
| REG-142 | `initialize_global_compatibility()` calls `client.set_compatibility("FULL_TRANSITIVE")` with no subject_name (registry-wide default) | behavior | code+test | — | DONE | | `rusty-meshed-schema-registry::SchemaRegistryEnforcer::initialize_global_compatibility`, test `initialize_global_compatibility_puts_full_transitive_to_config` |
| REG-143 | `set_subject_compatibility(subject, mode)` accepts `CompatibilityMode` enum or coerces raw string | behavior | code+test | — | DONE | | `rusty-meshed-schema-registry::SchemaRegistryEnforcer::set_subject_compatibility` (takes `&str`, matching the source's dynamic-coercion entry point), test `set_subject_compatibility_puts_to_the_subject_specific_path` |
| REG-144 | `set_subject_compatibility` raises `ValueError` (listing valid modes) for unrecognized string, before calling underlying client | behavior | code+test | — | DONE | | `rusty-meshed-schema-registry::SetCompatibilityError::InvalidMode`, test `set_subject_compatibility_rejects_invalid_mode_before_any_request` (no server started -- would hang if it made a request) |
| REG-145 | `get_subject_compatibility(subject)` delegates to `client.get_compatibility(subject_name=subject)` | interface | code+test | — | DONE | | `rusty-meshed-schema-registry::SchemaRegistryEnforcer::get_subject_compatibility`, test `get_subject_compatibility_reads_compatibility_level` |
| REG-146 | `register_schema(subject, schema_str)` wraps into `Schema` object with `"AVRO"` type before `client.register_schema` | behavior | code+test | — | DONE | | `rusty-meshed-schema-registry::SchemaRegistryEnforcer::register_schema`, test `register_schema_returns_the_assigned_id` (asserts `schemaType":"AVRO"` in the request body) |
| REG-147 | `register_schema` returns integer schema id from underlying client on success | interface | code+test | — | DONE | | same as REG-146, test `register_schema_returns_the_assigned_id` |
| REG-148 | `register_schema` catches `SchemaRegistryError` with `error_code==409`, raises `CompatibilityViolation(subject, message)` instead | behavior | code+test | — | DONE | | `rusty-meshed-schema-registry::SchemaRegistryEnforcer::register_schema` (checks HTTP 409, the practical equivalent of confluent-kafka-python's `error_code`), test `register_schema_maps_409_to_compatibility_violation` |
| REG-149 | `register_schema` re-raises `SchemaRegistryError` unchanged for any other error_code | behavior | code+test | — | DONE | | test `register_schema_propagates_non_409_errors_unchanged` |
| REG-150 | `CompatibilityMode` enum: 7 members — BACKWARD, BACKWARD_TRANSITIVE, FORWARD, FORWARD_TRANSITIVE, FULL, FULL_TRANSITIVE, NONE | interface | code | — | DONE | | `rusty-meshed-schema-registry::CompatibilityMode`, test `as_str_matches_the_python_enum_values` |
| REG-151 | `CompatibilityViolation.__str__` formats as `"Schema incompatible with {subject}: {message}"` | behavior | code+test | — | DONE | | `rusty-meshed-schema-registry::CompatibilityViolation`, test `compatibility_violation_formats_as_expected` |
| REG-152 | `CompatibilityViolation` exposes `.subject`/`.message` attributes independently | behavior | code+test | — | DONE | | `rusty-meshed-schema-registry::CompatibilityViolation::subject`/`message`, test `compatibility_violation_formats_as_expected` |
| SDK-001 | `BaseEvent` (`dataclasses_avroschema.pydantic.AvroBaseModel`) is the mandatory base class for every meshed platform event; subclassing gives four auto-populated lineage fields | interface | code+docs | — | REQUIRED | | |
| SDK-002 | `BaseEvent.event_id` defaults via `default_factory=lambda: str(uuid.uuid4())`, fresh UUID4 per instance | behavior | code+test | — | REQUIRED | | |
| SDK-003 | `BaseEvent.correlation_id` has no default and is required — omitting raises `pydantic.ValidationError` | behavior | code+test | — | REQUIRED | | |
| SDK-004 | `BaseEvent.source_event_ids` defaults to `[]` via `default_factory=list`, independent per instance | behavior | code+test | — | REQUIRED | | |
| SDK-005 | `BaseEvent.timestamp` defaults via `default_factory=lambda: datetime.now(timezone.utc).isoformat()` | behavior | code+test | — | REQUIRED | | |
| SDK-006 | `BaseEvent.Meta.namespace = "meshed.base"` (subclasses override via own nested Meta) | config | code | — | REQUIRED | | |
| SDK-007 | `BaseEvent.avro_schema()` (inherited) returns JSON-parseable Avro schema including all 4 lineage fields plus subclass fields | interface | code+test | — | REQUIRED | | |
| SDK-008 | `BaseEvent.serialize()`/`deserialize(bytes)` round-trip Avro bytes preserving all fields | behavior | code+test | — | REQUIRED | | |
| SDK-009 | `OutputPortSpec` immutable (`@dataclass(frozen=True)`): `name`, `topic`, `event_type: type[BaseEvent]`, `event_classification: EventType` | interface | code+test | — | DONE | | `rusty-meshed-sdk::OutputPortSpec<E>` (generic over the event type, `EventType` moved to `rusty-meshed-core` as shared vocabulary), test `constructs_with_the_given_fields` |
| SDK-010 | `OutputPortSpec` mutation after construction raises `dataclasses.FrozenInstanceError` | behavior | code+test | — | DONE | | `rusty-meshed-sdk::OutputPortSpec` -- structural immutability (private fields, no setters) is a compile-time guarantee, stronger than the Python source's runtime check; see the type's own doc comment |
| SDK-011 | `ContractVersionMismatch(expected, actual)` stores `.expected`/`.actual`, stringifies `"Contract version mismatch: expected {expected!r}, got {actual!r}"` | interface | code | — | DONE | | `rusty-meshed-sdk::ContractVersionMismatch`, test `contract_version_mismatch_formats_with_single_quotes` |
| SDK-012 | `RegistryError(message)` stores `.message`; raised by `RegistryClient` on any non-2xx (except `get_contract`'s 404) | interface | code | — | DONE | | `rusty-meshed-sdk::RegistryError`, test `registry_error_formats_as_the_bare_message` (the "raised by RegistryClient" usage itself is SDK-043..054, still open) |
| SDK-013 | `DataProductProducerBase` ABC; subclasses declare `product_name`, `domain`, `version`, `owner`, `description` (default `""`), `output_ports` (default `[]`) | interface | code+test | — | REQUIRED | | |
| SDK-014 | `DataProductProducerBase.__init__` accepts optional `config`, `sr_enforcer`, `registry_client`, `lineage_tracker`, `topic_manager` (DI); defaults built from `PlatformConfig` | interface | code+test | — | REQUIRED | | |
| SDK-015 | `startup()` Step 0 idempotently creates a topic per output port; catches `KafkaException` only when message contains "topic_already_exists"/"already exists" (case-insensitive), else re-raises | behavior | code+test | — | REQUIRED | | |
| SDK-016 | `startup()` Step 1: `schema_str=port.event_type.avro_schema()`, `subject=f"{port.topic}-value"`, calls `sr_enforcer.register_schema(subject, schema_str)` per output port | behavior | code+test | — | REQUIRED | | |
| SDK-017 | `startup()` builds each `AvroSerializer` with `to_dict=lambda obj,ctx: obj.model_dump()`, `conf={"auto.register.schemas": False}` | behavior | code+test | — | REQUIRED | | |
| SDK-018 | `startup()` constructs one `SerializingProducer` per output port, stored in `self._producers` keyed by topic | behavior | code+test | — | REQUIRED | | |
| SDK-019 | `startup()` Step 2 calls `registry_client.register_product(...)` once, reads `product["id"]` | behavior | code+test | — | REQUIRED | | |
| SDK-020 | `startup()` Step 3 calls `registry_client.register_output_port(...)` once per port | behavior | code+test | — | REQUIRED | | |
| SDK-021 | `startup()` Step 4 calls `lineage_tracker.record_job_run(job_name=product_name, job_namespace="meshed", inputs=[], outputs=[(kafka,topic)...])` once | behavior | code+test | — | REQUIRED | | |
| SDK-022 | `publish(topic, event)` raises `TypeError` (mentions "BaseEvent") when event not a `BaseEvent` | behavior | code+test | — | REQUIRED | | |
| SDK-023 | `publish(topic, event)` raises `ValueError` (mentions "not a declared output port", lists topics) when topic unknown | behavior | code+test | — | REQUIRED | | |
| SDK-024 | `publish()` attaches lineage headers as UTF-8 bytes: event_id, correlation_id, source_event_ids (comma-joined; empty→`b""`), timestamp | behavior | code+test | — | REQUIRED | | |
| SDK-025 | `publish()` calls `producer.produce(topic=, value=event, headers=headers, on_delivery=self._delivery_callback)` then `producer.poll(0)` | behavior | code+test | — | REQUIRED | | |
| SDK-026 | `publish()` calls `lineage_tracker.record_event(...)` after every produce | behavior | code+test | — | REQUIRED | | |
| SDK-027 | `flush(timeout=10.0)` calls `.flush(timeout)` on every producer in `self._producers.values()` | behavior | code+test | — | REQUIRED | | |
| SDK-028 | `_delivery_callback(err, msg)` static raises `RuntimeError(f"Message delivery failed: {err}")` when err non-None | behavior | code | — | REQUIRED | | |
| SDK-029 | `DataProductConsumerBase` ABC, abstract `async def process(self, event: BaseEvent) -> None`; subclasses declare `product_name`, `port_name`, `event_type`, `group_id` | interface | code+test | — | REQUIRED | | |
| SDK-030 | `DataProductConsumerBase.__init__` accepts optional `config`, `registry_client`, `lineage_tracker` (DI), defaults from `PlatformConfig` | interface | code+test | — | REQUIRED | | |
| SDK-031 | `startup()` Step 1 resolves topic via `registry_client.get_output_port(product_name, port_name)`, reading `topic_name`, `id`, `data_product_id` | behavior | code+test | — | REQUIRED | | |
| SDK-032 | `startup()` Step 2 calls `registry_client.get_contract(product_id, port_id)`; when contract non-None with `schema_ref` and `event_type.__name__` not a substring, raises `ContractVersionMismatch` | behavior | code+test | — | REQUIRED | | |
| SDK-033 | `startup()` skips contract validation when `get_contract()` returns `None` | behavior | code+test | — | REQUIRED | | |
| SDK-034 | `startup()` Step 3 builds `DeserializingConsumer` with `group.id`, `auto.offset.reset="earliest"`, `enable.auto.commit=False`, `value.deserializer=AvroDeserializer(...)`, `key.deserializer=StringDeserializer("utf_8")` | behavior | code+test | — | REQUIRED | | |
| SDK-035 | `startup()` Step 4 subscribes to exactly the one resolved topic | behavior | code+test | — | REQUIRED | | |
| SDK-036 | `startup()` Step 5 calls `lineage_tracker.record_job_run(job_name=type(self).__name__, job_namespace="meshed", inputs=[(kafka,topic)], outputs=[])` | behavior | code+test | — | REQUIRED | | |
| SDK-037 | `_is_duplicate(event_id)` returns False first time (adds to `_seen_event_ids`), True on repeats — `process()` runs at most once per event_id | behavior | code+test | — | REQUIRED | | |
| SDK-038 | `_seen_event_ids` unbounded in-memory set, no eviction/TTL/persistence — dedup lost on restart, not shared across processes (documented limitation) | behavior | code+docs | — | REQUIRED | | |
| SDK-039 | `_poll_loop(timeout=1.0)` (ThreadPoolExecutor worker) skips None/errored/None-deserialized/duplicate messages, else dispatches `process(event)` via `asyncio.run_coroutine_threadsafe`, blocks on `.result()` | behavior | code+test | — | REQUIRED | | |
| SDK-040 | Offset commit synchronous (`commit(asynchronous=False)`), only after `process()` completes successfully | behavior | code | — | REQUIRED | | |
| SDK-041 | `run()` sets `_running=True`, runs blocking poll loop in single-worker ThreadPoolExecutor via `loop.run_in_executor` | behavior | code+docs | — | REQUIRED | | |
| SDK-042 | `stop()` sets `_running=False`, calls `consumer.close()` only if consumer is not None; safe before `startup()` | behavior | code+test | — | REQUIRED | | |
| SDK-043 | `RegistryClient.__init__(base_url)` strips trailing `/` | behavior | code | — | REQUIRED | | |
| SDK-044 | `register_product(...)` POSTs `{name,domain,version,owner,description,tags:json.dumps(tags or [])}` to `POST {base_url}/data-products/`, returns body on 2xx | interface | code+test | — | REQUIRED | | |
| SDK-045 | `register_product(...)` raises `RegistryError("Failed to register product {name!r}: HTTP {status}")` on non-2xx | behavior | code+test | — | REQUIRED | | |
| SDK-046 | `register_output_port(...)` POSTs `{topic_name,schema_subject,event_type:event_classification,description:name}` to `POST {base_url}/data-products/{id}/output-ports` | interface | code+test | — | REQUIRED | | |
| SDK-047 | `register_output_port(...)` raises `RegistryError` with descriptive message on non-2xx | behavior | code+test | — | REQUIRED | | |
| SDK-048 | `get_output_port(product_name, port_name)` calls `GET .../data-products?name=`, client-filters for exact match, raises `RegistryError("No product found with name {product_name!r}")` if none | behavior | code+test | — | REQUIRED | | |
| SDK-049 | `get_output_port(...)` then `GET .../output-ports`, client-filters for exact port name, raises `RegistryError` if none, else returns first match | behavior | code+test | — | REQUIRED | | |
| SDK-050 | `get_output_port(...)` raises `RegistryError` on any non-2xx from either call | behavior | code+test | — | REQUIRED | | |
| SDK-051 | `get_contract(product_id, port_id)` calls `GET .../output-ports/{port_id}/contract` (singular) | interface | code+test | — | REQUIRED | | |
| SDK-052 | `get_contract(...)` returns `None` on exactly 404, no raise | behavior | code+test | — | REQUIRED | | |
| SDK-053 | `get_contract(...)` raises `RegistryError` for non-404 non-2xx; returns parsed JSON on 2xx | behavior | code+test | — | REQUIRED | | |
| SDK-054 | Every `RegistryClient` method opens a fresh `httpx.AsyncClient(follow_redirects=True)` per call | behavior | code | — | REQUIRED | | |
| SDK-055 | `OutboxEntry` (SQLModel, table `outbox_entries`): id PK, event_type, topic, payload (JSON), headers (JSON, default `"{}"`), created_at, published_at (Optional, default None) | interface | code+test | — | REQUIRED | | |
| SDK-056 | `write_outbox_entry(session, event_type, topic, payload, headers=None)` JSON-serializes payload/headers, sets created_at, `session.add()` — deliberately no `session.commit()` (caller commits atomically with business data) | behavior | code+test | — | REQUIRED | | |
| SDK-057 | `OutboxRelay.POLL_INTERVAL_SECONDS = 2.0` class attribute | config | code | — | REQUIRED | | |
| SDK-058 | `OutboxRelay.__init__(db_url, bootstrap_servers)` creates dedicated SQLAlchemy engine (separate from app engine) + own `confluent_kafka.Producer` | behavior | code+docs | — | REQUIRED | | |
| SDK-059 | `start()` launches `_relay_loop` in a `daemon=True` background thread | behavior | code+test | — | REQUIRED | | |
| SDK-060 | `stop()` sets `threading.Event`, joins relay thread with `timeout=5` | behavior | code+test | — | REQUIRED | | |
| SDK-061 | `_relay_loop()` calls `_relay_pending()` then `stop_event.wait(timeout=POLL_INTERVAL_SECONDS)`, repeats until stop set | behavior | code | — | REQUIRED | | |
| SDK-062 | `_relay_pending()` selects up to 100 rows `published_at IS NULL` ordered by id ASC (FIFO); produces each with headers from JSON, `producer.flush()` per entry, sets `published_at` | behavior | code+test | — | REQUIRED | | |
| SDK-063 | `_relay_pending()` commits the whole batch in one `session.commit()` after the loop | behavior | code+test | — | REQUIRED | | |
| SDK-064 | Exceptions from `producer.produce()`/`flush()` in `_relay_pending()` are not caught — failed entry stays `published_at=None`, retries next cycle (at-least-once, no dedup) | behavior | code+docs | — | REQUIRED | | |
| SDK-065 | `TopicType(str, Enum)`: STATE, EVENTS, COMMANDS, DLQ | interface | code | — | DONE | | `rusty-meshed-sdk::TopicType` |
| SDK-066 | `TopicSpec` dataclass: `name`, `topic_type` (required), `num_partitions=3`, `replication_factor=1`, `retention_ms=2_592_000_000` | config | code+test | — | DONE | | `rusty-meshed-sdk::TopicSpec::new`, test `new_applies_the_same_defaults_as_the_python_dataclass` |
| SDK-067 | `TopicSpec.kafka_config()` returns compact-policy config (`cleanup.policy=compact`, `min.cleanable.dirty.ratio=0.1`, `segment.ms=86400000`) for STATE type | behavior | code+test | — | DONE | | `rusty-meshed-sdk::TopicSpec::kafka_config`, test `state_topic_gets_compact_policy` |
| SDK-068 | `TopicSpec.kafka_config()` returns `{cleanup.policy:delete, retention.ms:str(retention_ms)}` for non-STATE types | behavior | code+test | — | DONE | | `rusty-meshed-sdk::TopicSpec::kafka_config`, tests `events_topic_gets_delete_policy_with_retention`, `dlq_topic_gets_delete_policy`, `commands_topic_gets_delete_policy` |
| SDK-069 | Documented invariant: no code outside `topic_config.py` should build raw Kafka config dicts | docs | docs | — | DONE | | `rusty-meshed-sdk::topic_config` module doc |
| SDK-070 | `TOPIC_PATTERN` regex `^[a-z][a-z0-9-]*\.[a-z][a-z0-9-]*\.[a-z][a-z0-9-]*$` — `{domain}.{product}.{stream-type}`, 3 segments | config | code+test | — | DONE | | `rusty-meshed-sdk::validate_topic_name`, tests `valid_topic_name_accepted`, `two_segment_name_rejected`, `numeric_leading_segment_rejected`, `uppercase_name_rejected`, `uppercase_stream_type_rejected` |
| SDK-071 | `TopicNameError(ValueError)` raised by `_validate_name()` on pattern mismatch; message includes "violates convention" | behavior | code+test | — | DONE | | `rusty-meshed-sdk::TopicNameError`, test `missing_domain_prefix_rejected` |
| SDK-072 | `TopicManager(admin_client)` takes constructed `AdminClient` via DI; docstring: `create_topics()` must only be called from within `TopicManager` | interface | code+docs | — | DONE | | `rusty-meshed-sdk::TopicManager::new` (takes an already-constructed `rusty_kafka::KafkaClient`), module doc |
| SDK-073 | `create_topic(spec)` validates name (raising `TopicNameError`) before any Kafka call | behavior | code+test | — | DONE | | `rusty-meshed-sdk::TopicManager::create_topic`, test `create_topic_rejects_invalid_name_before_any_kafka_call` |
| SDK-074 | `create_topic(spec)` builds `NewTopic(...)`, calls `admin_client.create_topics([...])`, calls `.result()` on each future — synchronous blocking, re-raises `KafkaException` | behavior | code+test | — | DONE | | `rusty-meshed-sdk::TopicManager::create_topic` (awaits `rusty_kafka::KafkaClient::create_topics`, checks each result's error_code), tests `create_state_topic_sends_compact_config`, `create_topic_sets_partitions_and_replication`, `create_topic_propagates_broker_error` |
| SDK-075 | `create_topic(spec)` does NOT itself swallow "already exists" — that's handled one layer up in `DataProductProducerBase.startup()` | behavior | code+test | — | DONE | | `rusty-meshed-sdk::TopicManager::create_topic` propagates any non-zero error code including `TOPIC_ALREADY_EXISTS`, test `create_topic_propagates_broker_error` |
| SDK-076 | On success, `create_topic` stores spec in in-memory `self._registry: dict[str,TopicSpec]`, not persisted | behavior | code | — | DONE | | `rusty-meshed-sdk::TopicManager` (`registry: HashMap<String, TopicSpec>` field), test `list_topics_returns_all_managed_topics` |
| SDK-077 | `deprecate_topic(name)` records `self._deprecated[name]=now`; no Kafka API call, no delete/alter, no verification the name was created via this instance | behavior | code+test | — | DONE | | `rusty-meshed-sdk::TopicManager::deprecate_topic`, test `deprecate_topic_marks_it_deprecated` |
| SDK-078 | `list_topics()` returns one dict per entry in `self._registry` (topics not created via this instance never appear): `name`, `topic_type`, `deprecated: bool`, `deprecated_at` | interface | code+test | — | DONE | | `rusty-meshed-sdk::TopicManager::list_topics` / `TopicStatus`, test `list_topics_returns_all_managed_topics` |
| SDK-079 | Documented invariant: no code outside `topic_manager.py` should call `AdminClient.create_topics()` directly | docs | docs | — | DONE | | `rusty-meshed-sdk::topic_manager` module doc |
| SDK-080 | `meshed.sdk` package `__all__` re-exports exactly `BaseEvent`, `ContractVersionMismatch`, `DataProductConsumerBase`, `DataProductProducerBase`, `OutputPortSpec`, `RegistryError` — `RegistryClient` NOT re-exported at package level | interface | code+test | — | REQUIRED | | |
| SDK-081 | `PlatformConfig` is `pydantic_settings.BaseSettings`, `env_prefix="MESHED_"`; `PlatformConfig()` built from env when `config=None` | config | code+test | — | REQUIRED | | |
| SDK-082 | `PlatformConfig.kafka_bootstrap_servers` default `"localhost:9092"` (env `MESHED_KAFKA_BOOTSTRAP_SERVERS`) | config | code | — | REQUIRED | | |
| SDK-083 | `PlatformConfig.schema_registry_url` default `"http://localhost:8081"` (env `MESHED_SCHEMA_REGISTRY_URL`) | config | code | — | REQUIRED | | |
| SDK-084 | `PlatformConfig.registry_db_path` default `"meshed_registry.db"` (env `MESHED_REGISTRY_DB_PATH`) | config | code | — | REQUIRED | | |
| SDK-085 | `PlatformConfig.registry_base_url` default `"http://localhost:8000"` (env `MESHED_REGISTRY_BASE_URL`) | config | code | — | REQUIRED | | |
| SDK-086 | `EventType(str,Enum)`: DELTA="delta", STATE="state", MEASUREMENT="measurement" — used as `OutputPortSpec.event_classification`, serialized via `.value` | interface | code+test | — | REQUIRED | | |
| GOV-001 | `GovernanceEngine.evaluate()` runs all registered policy callables against a `DataProductCreate` in registration order; a policy "passes" simply by returning `None` (no raise) | behavior | code+test | — | DONE | | `rusty-meshed-governance::GovernanceEngine::evaluate`, tests `engine_no_policies_returns_empty_violations`, `engine_single_passing_policy_returns_empty_violations`, `engine_mixed_policies_returns_only_failing_violations` |
| GOV-002 | `GovernanceEngine.evaluate()` catches `ValueError`/`AssertionError` raised by a policy and appends `str(exc)` verbatim as the violation string (no prefix/wrapping) | behavior | code+test | — | DONE | | `rusty-meshed-governance::GovernanceEngine::evaluate` (a policy returns `Err(String)` in place of raising `ValueError`/`AssertionError`), test `engine_single_failing_policy_returns_one_violation` |
| GOV-003 | `GovernanceEngine.evaluate()` catches any other `Exception` subtype from a policy and wraps it as `"[POLICY-ERROR] {policy.__name__}: {exc}"` instead of the raw message | behavior | code+test | — | DONE | | `rusty-meshed-governance::GovernanceEngine::evaluate` via `std::panic::catch_unwind` (Rust has no exception hierarchy to distinguish "violation" from "unexpected failure" the way Python's `except (ValueError, AssertionError)` vs. `except Exception` does — a genuine Rust panic inside a policy is the closest equivalent; see the crate's module doc for the one cosmetic difference, a stderr print on the caught panic), test `engine_wraps_a_panicking_policy_with_policy_error_prefix` |
| GOV-004 | `GovernanceEngine.register_policy(policy)` appends a callable to the internal policy list at runtime; it is evaluated starting on the *next* `evaluate()` call | interface | code+test | — | DONE | | `rusty-meshed-governance::GovernanceEngine::register_policy`, test `register_policy_appends_callable_evaluated_on_next_call` |
| GOV-005 | `GovernanceEngine.__init__(policies=None)` accepts an optional initial list of `PolicyFn`; `None`/falsy defaults to an empty list (copied via `list(policies)`, not aliased) | interface | code+test | — | DONE | | `rusty-meshed-governance::GovernanceEngine::new`/`Default`, test `engine_no_policies_returns_empty_violations` |
| GOV-006 | `require_description_min_length` raises `ValueError` when `description` is `None` or shorter than 20 characters; message reports the actual measured length | behavior | code+test | — | DONE | | `rusty-meshed-governance::require_description_min_length`, tests `require_description_min_length_rejects_short_description`, `require_description_min_length_accepts_long_description`, `require_description_min_length_treats_missing_description_as_empty` |
| GOV-007 | `require_semantic_version` raises `ValueError` unless `version` fully matches `^\d+\.\d+\.\d+$` (rejects `"v1.0"`, 2-part versions, pre-release/build suffixes, `None`) | behavior | code+test | — | DONE | | `rusty-meshed-governance::require_semantic_version`, tests `require_semantic_version_rejects_incomplete_version`, `require_semantic_version_accepts_valid_semver`, `require_semantic_version_rejects_prerelease_suffix`, `require_semantic_version_reports_none_literal_for_missing_version` |
| GOV-008 | `require_domain_lowercase` raises `ValueError` when `domain` contains any uppercase character (`domain != domain.lower()`) | behavior | code+test | — | DONE | | `rusty-meshed-governance::require_domain_lowercase`, tests `require_domain_lowercase_rejects_mixed_case_domain`, `require_domain_lowercase_accepts_lowercase_domain` |
| GOV-009 | Module-level `_DEFAULT_ENGINE` singleton is pre-loaded with exactly the 3 built-in policies, in this fixed order: description-length, semver, domain-lowercase | config | code | — | DONE | | `rusty-meshed-governance::default_engine`, test `default_engine_runs_all_three_built_in_policies_in_order` (registry crate still needs to wire this into shared app state — REG-035/043 remain open) |
| GOV-010 | `POST /data-products` evaluates the payload through `_DEFAULT_ENGINE` before persisting; any violations abort creation with HTTP 422 and body `{"detail": {"governance_violations": [...]}}`; nothing is written to the DB | interface | code+test | — | REQUIRED | | |
| GOV-011 | `POST /data-products` with zero governance violations persists the product and returns HTTP 201 with the created record (including assigned `id`) | interface | code+test | — | REQUIRED | | |
| GOV-012 | `POST /governance/evaluate` (dry-run) always returns HTTP 200 with `{"violations": [...], "passed": bool}` (`passed` = violations empty) and never persists, regardless of pass/fail | interface | code+test+docs | — | REQUIRED | | |
| GOV-013 | `PortAccessGrant` table (`port_access_grants`): `output_port_id` FK→`output_ports.id` (indexed), `consumer_group_id` (indexed str), `granted_by` (str), `id` PK autoincrement, `granted_at` defaulted at insert time to current UTC ISO-8601 timestamp | interface | code | — | REQUIRED | | |
| GOV-014 | `POST /access-grants` returns 404 when `output_port_id` does not reference an existing `OutputPort` — checked before the duplicate check | interface | code+test | — | REQUIRED | | |
| GOV-015 | `POST /access-grants` returns 409 when a grant already exists for the same `(output_port_id, consumer_group_id)` pair; duplicate check runs only after port-existence passes | behavior | code+test | — | REQUIRED | | |
| GOV-016 | `POST /access-grants` returns 201 with the full `PortAccessGrantPublic` record (`id`, `output_port_id`, `consumer_group_id`, `granted_by`, `granted_at`) on success | interface | code+test | — | REQUIRED | | |
| GOV-017 | `GET /access-grants` lists all grants and supports independent, AND-combinable optional query filters `output_port_id` (exact match) and `consumer_group_id` (exact match) | interface | code+test | — | REQUIRED | | |
| GOV-018 | `DELETE /access-grants/{grant_id}` deletes the grant and returns 204; returns 404 if `grant_id` does not exist | interface | code+test | — | REQUIRED | | |
| GOV-019 | `GET /data-products/{product_id}/output-ports/{port_id}/resolve` enforces checks in strict order: (1) 404 if product doesn't exist, (2) 404 if port doesn't exist OR belongs to a different product, (3) 403 if no `PortAccessGrant` matches `(port_id, consumer_group_id)` — access-grant check runs last | behavior | code+test+docs | — | REQUIRED | | |
| GOV-020 | `GET .../resolve` on success (grant found) returns 200 with `{"topic_name": ..., "schema_subject": ...}` sourced from the `OutputPort` row | interface | code+test | — | REQUIRED | | |
| GOV-021 | `contract_subject_name(consumer_group, producer_subject)` builds `"{consumer_group}.contracts.{producer_subject}"` — pure naming-convention function, no I/O | behavior | code+test | — | REQUIRED | | |
| GOV-022 | `register_consumer_contract()` POSTs `{"schemaType":"AVRO","schema":...}` to `{registry_url}/subjects/{contract_subject}/versions`, raises `httpx.HTTPStatusError` on non-2xx via `raise_for_status()`, returns the registry-assigned schema id from `response.json()["id"]` | interface | code+docs | — | REQUIRED | | |
| GOV-023 | `assert_schema_compatible()` POSTs the producer schema to `{registry_url}/compatibility/subjects/{contract_subject}/versions?verbose=true`; raises `AssertionError` joining registry `messages` (falling back to `"Schema is incompatible"`) whenever `is_compatible` is missing/false; raises `httpx.HTTPStatusError` on non-2xx; returns `None` when compatible — CI schema-compatibility gate | behavior | code+docs+test | — | REQUIRED | | |
| GOV-024 | `SQLiteLineageTransport.__init__` creates the `lineage_events` and `lineage_records` tables via `CREATE TABLE IF NOT EXISTS` on every `LineageTracker()` construction (idempotent) | behavior | code | — | REQUIRED | | |
| GOV-025 | `SQLiteLineageTransport.emit()` silently no-ops (no error, no persistence) for any event object that is not an `openlineage.client.event_v2.RunEvent` instance | behavior | code | — | REQUIRED | | |
| GOV-026 | `SQLiteLineageTransport.emit()` persists only `{job_name, event_type (or "UNKNOWN"), event_time, inputs, outputs}` for a `RunEvent`; `inputs`/`outputs` narrowed to JSON arrays of `{namespace, name}` only — `run.runId` and dataset facets dropped | behavior | code | — | REQUIRED | | |
| GOV-027 | `LineageTracker.record_job_run(job_name, job_namespace, inputs, outputs)` emits exactly one `RunEvent` with `eventType=RunState.COMPLETE`, a freshly generated `uuid4` run id, current UTC ISO-8601 `eventTime`, one `InputDataset`/`OutputDataset` per `(namespace, name)` tuple | interface | code+test | — | REQUIRED | | |
| GOV-028 | `LineageTracker.get_topology_dependencies()` reads only rows where `event_type='COMPLETE'`, returning one `{"consumer": job_name, "input_topic": name}` per distinct `(job_name, input dataset name)` pair — dedup in Python via seen-set | interface | code+test | — | REQUIRED | | |
| GOV-029 | `LineageTracker.record_event(event_id, correlation_id, source_event_ids, product_name, topic_name, event_timestamp)` writes directly to `lineage_records`, bypassing the OpenLineage `RunEvent` model; `source_event_ids` JSON-serialized before storage | interface | code+test+docs | — | REQUIRED | | |
| GOV-030 | `LineageTracker.get_record_lineage(correlation_id)` returns all `lineage_records` rows for that `correlation_id` ordered by `event_timestamp` ascending (plain string sort); `[]` for unknown id rather than raising | behavior | code+test | — | REQUIRED | | |
| GOV-031 | `GET /lineage/topology` always returns HTTP 200 with `{"dependencies": [...]}`, `[]` when no events recorded | interface | code+test+docs | — | REQUIRED | | |
| GOV-032 | `GET /lineage/record/{correlation_id}` always returns HTTP 200 with `{"correlation_id": ..., "events": [...]}`; unknown correlation_id yields `events: []`, not a 404 | interface | code+test+docs | — | REQUIRED | | |
| GOV-033 | `SchemaViolation` table (`schema_violations`): `id` PK autoincrement, `subject` (indexed str), `timestamp` (str, ISO-8601 UTC set at `record_violation()` call time), `error_message` (str) | interface | code | — | REQUIRED | | |
| GOV-034 | `MetricsCollector.compute_lag(group_id, topic, num_partitions)` sums `max(0, high_watermark − committed_offset)` across partitions `0..num_partitions-1`, using a scratch `Consumer` with `group.id=f"_meshed_metrics_{group_id}"` and `enable.auto.commit=False`, always closed in `finally` | behavior | code+test | — | REQUIRED | | |
| GOV-035 | `compute_lag()` treats any committed offset `< 0` (incl. `OFFSET_INVALID` sentinel `-1001`) as `0` committed, so an unconsumed topic's lag equals its full high-watermark | behavior | code+test | — | REQUIRED | | |
| GOV-036 | `MetricsCollector.get_throughput(topic, partition=0)` returns the raw high-watermark offset as a throughput proxy (v1 approximation), via `Consumer` with `group.id="_meshed_metrics_throughput"` | behavior | code+test+docs | — | REQUIRED | | |
| GOV-037 | `MetricsCollector.record_violation(subject, error_message, session)` (static) adds one `SchemaViolation` row with server-generated UTC timestamp; caller commits | interface | code+test | — | REQUIRED | | |
| GOV-038 | `MetricsCollector.get_violation_count(subject, session)` (static) counts `SchemaViolation` rows with exact `subject` match | interface | code+test | — | REQUIRED | | |
| GOV-039 | `MetricsCollector.get_product_metrics(...)` returns `{"lag":..., "throughput":..., "violation_count":...}` | interface | code+test | — | REQUIRED | | |
| GOV-040 | `GET /data-products/{product_id}/metrics`: 404 if product not found; 404 if zero output ports; computes metrics against `product.output_ports[0]` only; on `KafkaException` sets `lag`/`throughput` to `-1` + `"error"` field while `violation_count` still computed independently; optional `group_id` (default `"default"`)/`num_partitions` (default `1`) query params; response keys `product_id, product_name, lag, throughput, violation_count, topic` | interface | code+test | — | REQUIRED | | |
| GOV-041 | `SLOResult` dataclass carries `slo_type, passed, threshold, actual_value, message` | interface | code+test | — | REQUIRED | | |
| GOV-042 | `SLOViolationPayload` dataclass requires `product_name, port_name, slo_type, threshold, actual_value, violation_message`, auto-generates `event_id`/`timestamp`/`correlation_id` via `default_factory` | interface | code+test | — | REQUIRED | | |
| GOV-043 | `SLOMonitor._get_latest_timestamp_seconds_ago()` calls `AdminClient.list_offsets(OffsetSpec.latest())`, handles both raw result and `Future`-wrapped result, returns `float('inf')` on future exception or negative `timestamp_ms`; otherwise `(now_ms − timestamp_ms)/1000` seconds | behavior | code+test | — | REQUIRED | | |
| GOV-044 | `SLOMonitor.check_freshness(topic, partition, threshold_seconds)` passes iff `age_seconds <= threshold_seconds`; 3 distinct message templates (empty-partition/pass/violation) | behavior | code+test | — | REQUIRED | | |
| GOV-045 | `SLOMonitor.check_completeness(...)` uses the same arithmetic as `check_freshness` (liveness proxy, v1) but `slo_type="completeness"` + distinct wording — does NOT measure expected-vs-actual counts (deferred per docstring) | behavior | code+test+docs | — | REQUIRED | | |
| GOV-046 | `SLOMonitor.check_schema_conformance(subject, session)` passes iff `get_violation_count(subject, session) == 0`; `threshold` fixed `0.0`, `actual_value` = violation count as float | behavior | code+test | — | REQUIRED | | |
| GOV-047 | `SLOViolationPublisher.TOPIC` constant = `"mesh.governance.slo-violations"` — single fixed destination for all SLO violation events across products/ports | config | code+docs | — | REQUIRED | | |
| GOV-048 | `SLOViolationPublisher.publish(violation)` serializes via `dataclasses.asdict()`+`json.dumps()` to UTF-8 Kafka value; key = `violation.product_name.encode()`; `event_id`/`correlation_id` as UTF-8 Kafka headers; plain JSON not Avro (deliberate) | interface | code+test+docs | — | REQUIRED | | |
| GOV-049 | `SLOViolationPublisher.flush(timeout=5.0)` delegates to `producer.flush(timeout)` | interface | code+test | — | REQUIRED | | |
| XFM-001 | `SystemStatus` enum values: `legacy`, `dual_write`, `migrated`, `decommissioned` — legacy-system lifecycle state | interface | code+docs | — | REQUIRED | | |
| XFM-002 | `CapabilityDimension` enum values: `domain_ownership`, `data_as_a_product`, `self_serve_platform`, `federated_governance` — the four data-mesh maturity dimensions, each scored 0.0–5.0 | interface | code+docs | — | REQUIRED | | |
| XFM-003 | `DecisionType` enum values: `migrate_track`, `sunset_legacy`, `invest_platform`, `invest_product_teams` | interface | code+docs | — | REQUIRED | | |
| XFM-004 | `TransformationClock` table (`transformation_clock`): `id` PK, `current_quarter: int` default 0; exactly one row (id=1) | interface | code | — | REQUIRED | | |
| XFM-005 | `LegacySystem` table (`legacy_systems`): `id` PK, `track` (unique, indexed), `name`, `target_data_product`, `status` (default LEGACY), `status_since_quarter` (default 0) | interface | code | — | REQUIRED | | |
| XFM-006 | `CapabilityScore` table (`capability_scores`): `id` PK, `track` (indexed), `dimension`, `quarter` (indexed), `score: float` — append-only, new row per (track, dimension) every quarter | interface | code | — | REQUIRED | | |
| XFM-007 | `TransformationDecision` table (`transformation_decisions`): `id` PK, `quarter` (indexed), `decision_type`, `target`, `applied: bool` default False, `created_at` ISO-8601 UTC via `default_factory` | interface | code | — | REQUIRED | | |
| XFM-008 | `TransformationEvent` table (`transformation_events`): `id` PK, `quarter` (indexed), `event_type: str`, `track: str | None`, `message: str`, `timestamp` ISO-8601 UTC via `default_factory` | interface | code | — | REQUIRED | | |
| XFM-009 | `get_or_create_clock`: returns the singleton `TransformationClock` row, or creates it with `id=1, current_quarter=0` if none exists yet | behavior | code | — | REQUIRED | | |
| XFM-010 | `queue_decision`: persists a `TransformationDecision` targeted at `quarter = current_quarter + 1`, `applied=False`, and returns the row | behavior | code+test | — | REQUIRED | | |
| XFM-011 | `advance_quarter` — `MIGRATE_TRACK` on a track currently `LEGACY`: transitions to `DUAL_WRITE`, sets `status_since_quarter = next_q`, applies `+0.5 domain_ownership` / `+0.5 data_as_a_product`, emits `wave_started` event (with track) | behavior | code+test | — | REQUIRED | | |
| XFM-012 | `advance_quarter` — `MIGRATE_TRACK` on a track NOT `LEGACY` (or unknown track): rejected — no status change, no score delta, emits `decision_rejected` event | behavior | code+test | — | REQUIRED | | |
| XFM-013 | `advance_quarter` — `SUNSET_LEGACY` on a track currently `DUAL_WRITE` (clean cutover): transitions to `MIGRATED`, sets `status_since_quarter = next_q`, applies `+0.5 federated_governance` / `+0.3 data_as_a_product`, emits `system_decommissioned` event | behavior | code+test | — | REQUIRED | | |
| XFM-014 | `advance_quarter` — `SUNSET_LEGACY` on a track currently `LEGACY` (risky sunset): transitions to `DECOMMISSIONED`, sets `status_since_quarter = next_q`, applies `-1.0 federated_governance` / `-0.5 data_as_a_product`, emits `maturity_regression` event | behavior | code+test | — | REQUIRED | | |
| XFM-015 | `advance_quarter` — `SUNSET_LEGACY` on a track not `LEGACY`/`DUAL_WRITE` (e.g. already `MIGRATED`/`DECOMMISSIONED`) or unknown track: rejected, no state change, emits `decision_rejected` event | behavior | code+test | — | REQUIRED | | |
| XFM-016 | `advance_quarter` — `INVEST_PLATFORM`: applies `+0.3 self_serve_platform` to every track mesh-wide (not track-scoped), emits a single `decision_applied` event with `track=None` | behavior | code+test | — | REQUIRED | | |
| XFM-017 | `advance_quarter` — `INVEST_PRODUCT_TEAMS`: applies `+0.2 domain_ownership` / `+0.2 data_as_a_product` to every track mesh-wide, emits a single `decision_applied` event with `track=None` | behavior | code+test | — | REQUIRED | | |
| XFM-018 | `advance_quarter` — dual-write auto-completion: any track still `DUAL_WRITE` where `next_q - status_since_quarter >= 2` (`_DUAL_WRITE_MIN_QUARTERS`) auto-transitions to `MIGRATED` even with no queued `SUNSET_LEGACY` this quarter, applying the same clean-cutover delta and emitting `system_decommissioned` | behavior | code+test | — | REQUIRED | | |
| XFM-019 | `advance_quarter` — every computed `CapabilityScore` is clamped to `[0.0, 5.0]` (`_clamp`) | behavior | code | — | REQUIRED | | |
| XFM-020 | `advance_quarter` — score carry-forward: for every `(track, dimension)` a fresh `CapabilityScore` row is written each quarter = prior quarter's score + this quarter's accumulated delta; untouched tracks carry forward unchanged | behavior | code+test | — | REQUIRED | | |
| XFM-021 | `advance_quarter` — marks every processed `TransformationDecision.applied = True` so it isn't reprocessed on a later call | behavior | code | — | REQUIRED | | |
| XFM-022 | `advance_quarter` — increments `TransformationClock.current_quarter` by exactly 1 and commits unconditionally, even with zero pending decisions | behavior | code+test | — | REQUIRED | | |
| XFM-023 | `get_state` — returns snapshot: `quarter`, `legacy_systems` (`track`, `name`, `target_data_product`, `status`, `status_since_quarter` per system), `capability` (track → dimension-value map of latest scores), `maturity_trend`, `pending_decisions`, `decision_history` | behavior | code+test | — | REQUIRED | | |
| XFM-024 | `get_state` — `pending_decisions`: all `TransformationDecision` rows where `applied=False`, each `{id, quarter, decision_type, target}` | behavior | code | — | REQUIRED | | |
| XFM-025 | `get_state` — `decision_history`: `applied=True` rows ordered by `quarter DESC`, capped at the 20 most recent, each `{id, quarter, decision_type, target}` (no pagination beyond 20) | behavior | code | — | REQUIRED | | |
| XFM-026 | `_maturity_trend` — one `{quarter, maturity_index}` entry per quarter from 0 through the current quarter; `maturity_index` = mean of every `CapabilityScore.score` recorded that quarter across all tracks/dimensions, rounded to 2 decimals | behavior | code+test | — | REQUIRED | | |
| XFM-027 | `GET /transformation/state` — 200; calls `seed_transformation_state` (auto-seed on first access) then returns `get_state()` | interface | code+test | — | REQUIRED | | |
| XFM-028 | `POST /transformation/decisions` — 201; body `{decision_type, target}`; auto-seeds first; for `MIGRATE_TRACK`/`SUNSET_LEGACY` validates `target` is an existing `LegacySystem.track`, else 404 `"Unknown track {target!r}."`; for `INVEST_PLATFORM`/`INVEST_PRODUCT_TEAMS` validates `target ∈ {"platform","product_teams"}`, else 422 with message naming the valid set; on success returns `{id, quarter, decision_type, target}` | interface | code+test | — | REQUIRED | | |
| XFM-029 | `POST /transformation/advance` — 200; auto-seeds first, calls `advance_quarter`, returns the full resulting snapshot | interface | code+test | — | REQUIRED | | |
| XFM-030 | `GET /transformation/events` — SSE stream, `media_type="text/event-stream"`, headers `Cache-Control: no-cache`, `Connection: keep-alive`, `X-Accel-Buffering: no` | interface | code | — | REQUIRED | | |
| XFM-031 | SSE event generator — on connect, seeks `last_id = MAX(id)` from `transformation_events` (defaults to 0 if table/db missing) so only events newer than connect-time are streamed, not full history | behavior | code | — | REQUIRED | | |
| XFM-032 | SSE event generator — polls every 1.0s, selects up to 50 rows with `id > last_id` ordered ascending, yields each as `data: {json}\n\n` with fields `{id, quarter, eventType (camelCase), track, message, timestamp}` | behavior | code | — | REQUIRED | | |
| XFM-033 | SSE event generator — emits a `: heartbeat\n\n` comment line whenever a poll cycle finds no new events | behavior | code | — | REQUIRED | | |
| XFM-034 | SSE event generator — swallows all sqlite3 exceptions silently (both initial `last_id` lookup and each poll) rather than raising or closing the stream | behavior | code | — | REQUIRED | | |
| XFM-035 | `DecisionCreate` request schema: `{decision_type: DecisionType, target: str}`, used as the `POST /transformation/decisions` body | interface | code | — | REQUIRED | | |
| XFM-036 | `seed_transformation_state` — idempotent: no-op (returns immediately) if a `TransformationClock` row already exists | behavior | code+test | — | REQUIRED | | |
| XFM-037 | `seed_transformation_state` — seeds exactly 3 legacy systems: `personnel-lifecycle`/"Personnel Legacy DB", `position-management`/"Position Management Spreadsheets", `readiness-reporting`/"Readiness Manual Reports", each `target_data_product` equal to its own track slug | behavior | code+test | — | REQUIRED | | |
| XFM-038 | `seed_transformation_state` — each seeded `LegacySystem` starts `status=LEGACY`, `status_since_quarter=0` | behavior | code+test | — | REQUIRED | | |
| XFM-039 | `seed_transformation_state` — seeds one `CapabilityScore` row per `(track, dimension)` at `quarter=0` with baseline score `1.0`, covering all 4 dimensions × 3 tracks (12 rows) | behavior | code+test | — | REQUIRED | | |
| XFM-040 | `seed_transformation_state` — seeds `TransformationClock(id=1, current_quarter=0)` | behavior | code+test | — | REQUIRED | | |
| XFM-041 | `PlatformConfig.kafka_bootstrap_servers: str`, default `"localhost:9092"`, env `MESHED_KAFKA_BOOTSTRAP_SERVERS` | config | code | — | DONE | | `rusty-meshed-core::PlatformConfig`, test `every_field_is_overridable_via_its_prefixed_env_var` |
| XFM-042 | `PlatformConfig.schema_registry_url: str`, default `"http://localhost:8081"`, env `MESHED_SCHEMA_REGISTRY_URL`, documented convention "no trailing slash" (not enforced in code) | config | code | — | DONE | | `rusty-meshed-core::PlatformConfig`, test `every_field_is_overridable_via_its_prefixed_env_var` |
| XFM-043 | `PlatformConfig.default_num_partitions: int`, default `3`, validation `ge=1`, env `MESHED_DEFAULT_NUM_PARTITIONS` | config | code | — | DONE | | `rusty-meshed-core::PlatformConfig`, tests `every_field_is_overridable_via_its_prefixed_env_var`, `default_num_partitions_below_one_is_rejected`, `non_integer_value_is_rejected` |
| XFM-044 | `PlatformConfig.default_replication_factor: int`, default `1`, validation `ge=1`, env `MESHED_DEFAULT_REPLICATION_FACTOR` | config | code | — | DONE | | `rusty-meshed-core::PlatformConfig`, tests `every_field_is_overridable_via_its_prefixed_env_var`, `default_replication_factor_below_one_is_rejected` |
| XFM-045 | `PlatformConfig.default_retention_ms: int`, default `2_592_000_000` (30 days), validation `ge=1`, env `MESHED_DEFAULT_RETENTION_MS` | config | code | — | DONE | | `rusty-meshed-core::PlatformConfig`, tests `every_field_is_overridable_via_its_prefixed_env_var`, `default_retention_ms_below_one_is_rejected` |
| XFM-046 | `PlatformConfig.registry_db_path: str`, default `"meshed_registry.db"`, env `MESHED_REGISTRY_DB_PATH` | config | code+test | — | DONE | | `rusty-meshed-core::PlatformConfig`, test `every_field_is_overridable_via_its_prefixed_env_var` |
| XFM-047 | `PlatformConfig.registry_base_url: str`, default `"http://localhost:8000"`, env `MESHED_REGISTRY_BASE_URL` | config | code | — | DONE | | `rusty-meshed-core::PlatformConfig`, test `every_field_is_overridable_via_its_prefixed_env_var` |
| XFM-048 | `PlatformConfig` global `env_prefix="MESHED_"` applied to every field via `SettingsConfigDict(env_prefix="MESHED_")` | config | code | — | DONE | | `rusty-meshed-core::PlatformConfig`, test `unprefixed_env_var_is_ignored` |
| XFM-049 | `meshed.__version__ = "0.1.0"` package version constant | interface | code | — | REQUIRED | | |
| CLI-001 | `meshed` Typer app registered as console-script entry point (`meshed = "meshed.cli.app:app"`), exposes 4 subcommands: `health`, `lineage`, `metrics`, `slo` | interface | code | — | REQUIRED | | |
| CLI-002 | `meshed --help` exits 0, output includes "health" and "metrics" | interface | test+code | — | REQUIRED | | |
| CLI-003 | `meshed health <product>` — required positional `product`; default `--format table` | interface | code+test | — | REQUIRED | | |
| CLI-004 | `meshed health` `--format`/`-f` option, enum table\|json, default table | interface | code+test | — | REQUIRED | | |
| CLI-005 | `meshed health --format json` prints JSON: `name`, `domain`, `owner`, `maturity_tier`, `ports` (list of `{topic,schema_subject,slo_status}`), `slo_status` | interface | code+test | — | REQUIRED | | |
| CLI-006 | `meshed health <missing>` prints `Error: Data product '<name>' not found.` (red), exits 1 | behavior | code+test | — | REQUIRED | | |
| CLI-007 | `meshed health --format table` prints Rich table `Data Product: {name}` (Field/Value); if ports non-empty, second table `Output Ports` (Topic/Schema Subject/SLO Status) | interface | code+test(partial) | — | REQUIRED | | |
| CLI-008 | `meshed health` SLO status: per-port `"configured"` if `port.contract is not None` else `"unconfigured"`; top-level = configured if any port configured | behavior | code | — | REQUIRED | | |
| CLI-009 | `meshed lineage <product_name>` — required positional; default `--format table` | interface | code | — | REQUIRED | | |
| CLI-010 | `meshed lineage` `--format`/`-f` option, enum table\|json, default table | interface | code | — | REQUIRED | | |
| CLI-011 | `meshed lineage` `--db-path` option, default `""`; empty resolves to `PlatformConfig().registry_db_path` | interface | code | — | REQUIRED | | |
| CLI-012 | `meshed lineage` calls `LineageTracker(db_path=).get_topology_dependencies()`, filters to `dep["consumer"]==product_name` | behavior | code | — | REQUIRED | | |
| CLI-013 | `meshed lineage --format json` prints filtered `{consumer,input_topic}` list as JSON | interface | code | — | REQUIRED | | |
| CLI-014 | `meshed lineage --format table` (deps found) prints Rich table `Lineage Topology: {product_name}` (Consumer Product/Input Topic) | interface | code | — | REQUIRED | | |
| CLI-015 | `meshed lineage --format table` (no deps) prints yellow `No lineage topology recorded for '{product_name}'.`, exit 0 | behavior | code | — | REQUIRED | | |
| CLI-016 | `meshed metrics <product>` — required positional; default `--format table` | interface | code+test | — | REQUIRED | | |
| CLI-017 | `meshed metrics` `--group-id`/`-g` optional, default None; effective group = `f"meshed-cli-{product}"` when omitted | interface | code | — | REQUIRED | | |
| CLI-018 | `meshed metrics` `--format`/`-f`, enum table\|json, default table | interface | code+test | — | REQUIRED | | |
| CLI-019 | `meshed metrics <missing>` prints `Error: Data product '<name>' not found.` (red), exits 1 | behavior | code | — | REQUIRED | | |
| CLI-020 | `meshed metrics <no-output-ports>` prints `Warning: Data product '<name>' has no output ports.` (yellow), exits 1 | behavior | code | — | REQUIRED | | |
| CLI-021 | `meshed metrics` only inspects `dp.output_ports[0]` | behavior | code | — | REQUIRED | | |
| CLI-022 | `meshed metrics` success path: `MetricsCollector(bootstrap_servers=).get_product_metrics(group_id=effective, topic=port.topic_name, num_partitions=1, subject=port.schema_subject, session=)`, surfaces lag/throughput/violation_count | behavior | code+test | — | REQUIRED | | |
| CLI-023 | `meshed metrics` on exception from `get_product_metrics`: `lag="unavailable"`, `throughput="unavailable"`, falls back to `get_violation_count` for violation_count — exits 0 | behavior | code | — | REQUIRED | | |
| CLI-024 | `meshed metrics --format json` prints `{product,lag,throughput,violation_count}` | interface | code+test | — | REQUIRED | | |
| CLI-025 | `meshed metrics --format table` Rich table `Metrics: {product}` (Metric/Value rows: Product/Lag/Throughput/Violation Count) | interface | code+test | — | REQUIRED | | |
| CLI-026 | `meshed slo <product>` — required positional; default `--format table` | interface | code+test | — | REQUIRED | | |
| CLI-027 | `meshed slo` `--format`/`-f`, enum table\|json, default table | interface | code | — | REQUIRED | | |
| CLI-028 | `meshed slo` `--registry-url` option, default `"http://localhost:8000"`; help says "unused in v1; reserved" — never read in function body (dead) | config | code | — | REQUIRED | | |
| CLI-029 | `meshed slo` `--bootstrap-servers`/`-b`, default `"localhost:9092"` | interface | code+test | — | REQUIRED | | |
| CLI-030 | `meshed slo <missing>` prints `Error: Data product '<name>' not found.` (red), exits 1 | behavior | code | — | REQUIRED | | |
| CLI-031 | `meshed slo <no-output-ports>` prints `Warning: Data product '<name>' has no output ports.` (yellow), exits 1 | behavior | code | — | REQUIRED | | |
| CLI-032 | `meshed slo` for port with `contract is None`: emits `slo_type="all"`, `status="unconfigured"`, `threshold="—"`, `actual="—"`, `message="No data contract — SLO not configured"` | behavior | code | — | REQUIRED | | |
| CLI-033 | `meshed slo` freshness dimension: on `check_freshness()` exception, emits `status="unavailable"`, `threshold=f"{s}s"`, `actual="unavailable"`, `message=f"Kafka unavailable: {exc}"` — exits 0 | behavior | code | — | REQUIRED | | |
| CLI-034 | `meshed slo` freshness success: `status`=PASS/FAIL by `freshness.passed`; `threshold=f"{t:.0f}s"`; `actual="∞"` if inf else `f"{v:.1f}s"` | behavior | code | — | REQUIRED | | |
| CLI-035 | `meshed slo` completeness dimension ("v1 liveness"): `check_completeness(topic,partition=0,threshold_seconds=contract.slo_freshness_seconds)`, same formatting rules | behavior | code+test | — | REQUIRED | | |
| CLI-036 | `meshed slo` schema-conformance dimension: direct `sqlite3.connect` query `SELECT COUNT(*) FROM schema_violations WHERE subject=?` (bypasses SQLModel session — CLI runs outside FastAPI process); `status=PASS` iff count==0, `threshold="0"`, `actual=str(count)` | behavior | code | — | REQUIRED | | |
| CLI-037 | `meshed slo` publishes `SLOViolationPayload` via `SLOViolationPublisher.publish()` for each FAIL dimension only (never for unconfigured/unavailable) | behavior | code+test | — | REQUIRED | | |
| CLI-038 | `meshed slo` constructs `SLOViolationPublisher(bootstrap_servers=)` in try/except; on failure `publisher=None`, command proceeds without publishing | behavior | code+test | — | REQUIRED | | |
| CLI-039 | `meshed slo` calls `publisher.flush(timeout=5.0)` exactly once after the loop (only if publisher constructed); flush exceptions swallowed | behavior | code+test | — | REQUIRED | | |
| CLI-040 | `meshed slo --format table` Rich table `SLO Status: {product}` (Port/SLO Type/Status/Threshold/Actual/Message); colors green=PASS,red=FAIL,yellow=unavailable,dim=unconfigured | interface | code | — | REQUIRED | | |
| CLI-041 | `meshed slo --format json` prints full `results` list as JSON | interface | code | — | REQUIRED | | |
| CLI-042 | `_get_violation_count_direct(db_path,subject)` returns 0 on any sqlite exception rather than raising | behavior | code | — | REQUIRED | | |
| CLI-043 | `scripts/demo_outbox.py` — reads `MESHED_COMPOSE_UP` (non-empty enables relay), `DEMO_DB_PATH` (default `"demo_outbox.db"`), `KAFKA_BOOTSTRAP_SERVERS` (default `"localhost:9092"`); writes one hardcoded `OutboxEntry` in a single SQLite transaction, prints progress | behavior | code+docs | — | REQUIRED | | |
| CLI-044 | `demo_outbox.py` when `MESHED_COMPOSE_UP` unset: skips relay, prints all outbox rows, `sys.exit(0)` | behavior | code | — | REQUIRED | | |
| CLI-045 | `demo_outbox.py` when `MESHED_COMPOSE_UP` set: starts `OutboxRelay` thread, polls 0.25s up to 10s for `published_at`; success prints details; timeout prints ERROR, `sys.exit(1)` | behavior | code | — | REQUIRED | | |
| CLI-046 | `scripts/init_registry.py` — sets Schema Registry global compatibility to FULL_TRANSITIVE via `SchemaRegistryClient({"url":url}).set_compatibility(...)`; URL from `MESHED_SCHEMA_REGISTRY_URL` (default `"http://localhost:8081"`); prints confirmation | behavior | code+docs | — | REQUIRED | | |
| CLI-047 | `main.py` — `main()` prints `"Hello from meshed!"`, `if __name__=="__main__"` guard; 84 bytes; NOT registered in pyproject.toml scripts and not referenced anywhere else — vestigial scaffolding | interface | code | — | REQUIRED | | |
| CLI-048 | compose.yaml `kafka` service: image `confluentinc/confluent-local:7.7.1`, ports 9092/8082, full KRaft env config, healthcheck `kafka-topics --list` | config | interface | — | REQUIRED | | |
| CLI-049 | compose.yaml `schema-registry` service: image `confluentinc/cp-schema-registry:7.7.1`, port 8081, depends_on kafka healthy, healthcheck `curl /subjects` | config | interface | — | REQUIRED | | |
| CLI-050 | compose.yaml `kafka-ui` service: image `provectus/kafka-ui:latest`, port 8080, depends_on schema-registry healthy, no own healthcheck | config | interface | — | REQUIRED | | |
| CLI-051 | `tests/test_compose.py::test_kafka_reachable` — `AdminClient.list_topics(timeout=10)` must not raise; skipped unless `MESHED_COMPOSE_UP` set | behavior | test | — | REQUIRED | | |
| CLI-052 | `tests/test_compose.py::test_schema_registry_health` — `GET :8081/subjects` returns 200; skipped unless `MESHED_COMPOSE_UP` set | behavior | test | — | REQUIRED | | |
| CLI-053 | `tests/test_compose.py::test_kafka_ui_reachable` — `GET :8080` returns 200; skipped unless `MESHED_COMPOSE_UP` set | behavior | test | — | REQUIRED | | |
| DOM-001 | `BaseEvent` lineage contract inherited by every domain event: `event_id` (auto UUID4), `correlation_id` (required, no default), `source_event_ids` (default `[]`, independent), `timestamp` (auto UTC); `Meta.namespace="meshed.base"` | interface | code+test | — | REQUIRED | | |
| DOM-002 | Event `PersonnelAssigned` (ns `meshed.domains.personnel`): `person_id`,`position_id`,`unit_uic`,`duty_title`,`grade`,`effective_date`,`transaction_date` (all required + BaseEvent fields) | interface | code+test | — | REQUIRED | | |
| DOM-003 | Event `PersonnelPromoted`: `person_id`,`from_grade`,`to_grade`,`effective_date`,`transaction_date` | interface | code+test | — | REQUIRED | | |
| DOM-004 | Event `PersonnelSeparated`: `person_id`,`separation_reason` (free-form),`effective_date`,`transaction_date` | interface | code+test | — | REQUIRED | | |
| DOM-005 | Event `StatusChanged`: `person_id`,`previous_status`,`new_status`,`effective_date`,`transaction_date` | interface | code+test | — | REQUIRED | | |
| DOM-006 | Event `PositionAuthorizationChanged` (ns `meshed.domains.position`): `position_id`,`unit_uic`,`authorized_grade`,`duty_title`,`authorization_status`,`effective_date`,`transaction_date` | interface | code+test | — | REQUIRED | | |
| DOM-007 | Event `PositionFilled`: `position_id`,`person_id`,`unit_uic`,`effective_date`,`transaction_date` | interface | code+test | — | REQUIRED | | |
| DOM-008 | Event `PositionVacated`: `position_id`,`person_id`,`unit_uic`,`vacancy_reason`,`effective_date`,`transaction_date` | interface | code+test | — | REQUIRED | | |
| DOM-009 | Event `PositionModified`: generic field-diff — `position_id`,`unit_uic`,`field_changed`,`old_value`,`new_value`,`effective_date`,`transaction_date` | interface | code+test | — | REQUIRED | | |
| DOM-010 | Event `UnitReadinessAssessed` (ns `meshed.domains.readiness`): `unit_uic`,`readiness_pct` (0-100, not enforced),`assessed_at`,`effective_date`,`transaction_date`; classified `EventType.MEASUREMENT` (others are DELTA) | interface | code+test | — | REQUIRED | | |
| DOM-011 | All 9 domain event classes: `avro_schema()` valid JSON record type, includes `effective_date`/`transaction_date` as Avro string(-nullable), plus all 4 BaseEvent lineage fields | interface | test+code | — | REQUIRED | | |
| DOM-012 | `PersonnelLifecycleProducer` metadata: product_name="personnel-lifecycle", domain="manpower", version="1.0.0", owner="manpower-team" | config | code+test | — | REQUIRED | | |
| DOM-013 | `PersonnelLifecycleProducer.output_ports` — 4 ports (assignments/promotions/separations/status-changes), all DELTA, mapped topics under `manpower.personnel-lifecycle.*` | interface | code+test | — | REQUIRED | | |
| DOM-014 | `PersonnelLifecycleProducer.publish()` override: does NOT produce to Kafka directly — writes an outbox row (`write_outbox_entry`) with payload/headers, commits atomically with any business write | behavior | code+test | — | REQUIRED | | |
| DOM-015 | `PersonnelLifecycleProducer.publish()` validation: `TypeError` (mentions BaseEvent) if not a BaseEvent; `ValueError` (mentions "not a declared output port") if topic unknown | behavior | code+test | — | REQUIRED | | |
| DOM-016 | `PersonnelLifecycleProducer.publish()` calls `lineage_tracker.record_event(...)` immediately after outbox commit | behavior | code+test | — | REQUIRED | | |
| DOM-017 | `PersonnelLifecycleProducer.__init__(...,db_url=None)`: defaults to `sqlite:///{config.registry_db_path}`; own SQLite engine + `SQLModel.metadata.create_all`; `OutboxRelay` bound to same db_url but a SEPARATE engine instance (deliberate, avoids cross-thread SQLite issues) | behavior | code+test(partial) | — | REQUIRED | | |
| DOM-018 | `PersonnelLifecycleProducer.startup()` calls `super().startup()` then `self._outbox_relay.start()` | behavior | code+test | — | REQUIRED | | |
| DOM-019 | `PersonnelLifecycleProducer.shutdown()` calls `self._outbox_relay.stop()` (waits up to 5s) | behavior | code+test | — | REQUIRED | | |
| DOM-020 | `PositionManagementProducer` metadata: product_name="position-management", domain="manpower", version="1.0.0", owner="manpower-team" | config | code+test | — | REQUIRED | | |
| DOM-021 | `PositionManagementProducer.output_ports` — 4 ports (authorization-changes/fills/vacancies/modifications), all DELTA; uses base `publish()` directly (no outbox override, unlike Personnel) | interface | code+test | — | REQUIRED | | |
| DOM-022 | `ReadinessAssessmentProducer` metadata: product_name="readiness-reporting"; 1 output port `assessments`→`manpower.readiness-reporting.assessments`, classification MEASUREMENT | interface | code+test | — | REQUIRED | | |
| DOM-023 | `PersonnelAssignmentConsumer` (group_id="readiness-reporting-personnel-consumer"): `process()` asserts `isinstance(event,PersonnelAssigned)`, derives `UnitReadinessAssessed` with correlation_id propagated exactly, `source_event_ids=[event.event_id]`, `readiness_pct` hardcoded 0.75, publishes+flushes | behavior | code+test | — | REQUIRED | | |
| DOM-024 | `PositionFillConsumer` (group_id="readiness-reporting-position-consumer"): same derivation logic keyed off `PositionFilled` | behavior | code+test | — | REQUIRED | | |
| DOM-025 | `ReadinessReportingProduct` wrapper: one `ReadinessAssessmentProducer` shared by both consumers; `startup()` sequential; `run()` uses `asyncio.gather()` — both consumers poll concurrently | behavior | code+test | — | REQUIRED | | |
| DOM-026 | `ScenarioBuilder` dataclass state: `correlation_id` (default uuid4), `base_time` (default now UTC, microsecond=0), internal `_active_persons`,`_authorized_positions`,`_assigned_persons`,`_time_offset_days` | interface | code+test | — | REQUIRED | | |
| DOM-027 | `ScenarioBuilder.add_status_change(...)`: appends `StatusChanged`; if new_status=="ACTIVE" marks person eligible for `add_assignment()`; returns self (chaining) | behavior | code+test | — | REQUIRED | | |
| DOM-028 | `ScenarioBuilder.add_position_authorization(...)`: appends `PositionAuthorizationChanged` (status="AUTHORIZED"), marks position eligible for `add_position_fill()`; returns self | behavior | code+test | — | REQUIRED | | |
| DOM-029 | `ScenarioBuilder.add_assignment(...)`: raises `ValueError` (mentions person_id) unless person was activated first; else appends `PersonnelAssigned`, records event_id; returns self | behavior | code+test | — | REQUIRED | | |
| DOM-030 | `ScenarioBuilder.add_position_fill(...)`: raises `ValueError` (mentions position_id) unless position was authorized first; appends `PositionFilled` with `source_event_ids` linked if present; returns self | behavior | code+test | — | REQUIRED | | |
| DOM-031 | `ScenarioBuilder.add_promotion(...)`: appends `PersonnelPromoted`, no prerequisite check; returns self | behavior | code+test | — | REQUIRED | | |
| DOM-032 | `ScenarioBuilder.add_separation(...)`: appends `PersonnelSeparated`, no prerequisite check; returns self | behavior | code+test | — | REQUIRED | | |
| DOM-033 | `ScenarioBuilder.add_retroactive_correction(...)`: appends `PersonnelAssigned` with `effective_date` in the past while `transaction_date` advances forward, demonstrating effective_date < transaction_date; links source_event_ids to prior assignment | behavior | code+test | — | REQUIRED | | |
| DOM-034 | `ScenarioBuilder.build()`: returns a shallow copy of internal event list in insertion order; mutating result doesn't affect builder state | behavior | code+test | — | REQUIRED | | |
| DOM-035 | `ScenarioBuilder._next_timestamp(days_forward=1)`: advances `_time_offset_days` before computing timestamp — guarantees monotonic non-decreasing timestamps | behavior | code+test | — | REQUIRED | | |
| DOM-036 | Every event from one `ScenarioBuilder` instance carries identical `correlation_id`; two instances get different default correlation_ids | behavior | code+test | — | REQUIRED | | |
| DOM-037 | `run_continuous.py` — reads `KAFKA_BOOTSTRAP_SERVERS` (default localhost:9092), `SCHEMA_REGISTRY_URL` (default :8081), `REGISTRY_API_URL` (default :8100), `SCENARIO_INTERVAL` (float, default 5s) | config | code+docs | — | REQUIRED | | |
| DOM-038 | `run_continuous._create_topics()`: idempotently creates 9 fixed topics (3 partitions, replication 1, delete policy, 30-day retention); skips existing; per-topic failures logged as warnings (non-fatal); unreachable broker propagates from connectivity probe | behavior | code | — | REQUIRED | | |
| DOM-039 | `run_continuous.main()`: if `_create_topics()` raises, logs error, `sys.exit(1)` before producers constructed | behavior | code | — | REQUIRED | | |
| DOM-040 | `run_continuous._build_random_scenario()`: 2-5 random people, 1 random unit, random IDs; activates all (days_forward=0); authorizes positions (random grade E4-E7, random duty); assigns+fills all pairs; 60% chance one promotion; 30% chance one separation | behavior | code | — | REQUIRED | | |
| DOM-041 | `run_continuous.main()` loop: infinite; each cycle builds fresh random scenario, publishes with `sleep(uniform(0.1,0.4))` between events, flushes both producers, sleeps `SCENARIO_INTERVAL`; KeyboardInterrupt logs summary and exits cleanly | behavior | code | — | REQUIRED | | |
| DOM-042 | `run_continuous`'s `_EVENT_TOPIC_MAP` covers only 6 event types — readiness topic created but never published to by this script | behavior | code | — | REQUIRED | | |
| DOM-043 | `run_scenario.py` — explicitly NOT run in CI (manual demo); reads `KAFKA_BOOTSTRAP_SERVERS`, `SCHEMA_REGISTRY_URL`, `REGISTRY_API_URL` (default :8000 — differs from run_continuous's :8100) | config | code+docs | — | REQUIRED | | |
| DOM-044 | `run_scenario._create_topics()`: same idempotent 9-topic logic as run_continuous | behavior | code | — | REQUIRED | | |
| DOM-045 | `run_scenario.main()`: topic-creation failure logs error (mentions podman-compose up), `sys.exit(1)` | behavior | code | — | REQUIRED | | |
| DOM-046 | `run_scenario._build_demo_scenario()`: fixed non-random 14-event scenario (3 people, 3 positions under UNIT-ALPHA, assign/fill all, 1 promotion, 1 retroactive correction) | behavior | code | — | REQUIRED | | |
| DOM-047 | `run_scenario.main()`: publishes all 14 events, logs each, flushes both producers, prints per-topic summary + grand total + shared correlation_id | behavior | code | — | REQUIRED | | |
