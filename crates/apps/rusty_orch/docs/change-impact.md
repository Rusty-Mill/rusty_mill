# Proposed-change impact (Stage 1)

From a clean Git checkout, generate a report without running agents or validation:

```sh
rusty_mill_board --projects /path/to/projects.json --export-projects > /tmp/projects.json
rusty-remind-me impact-export 'rusty_atomic_file compatibility' > /tmp/requirements.json
rusty_orch inspect --repo /path/to/rusty_mill --impact /tmp/change.json > /tmp/impact.json
```

`projects.json` uses Millboard's existing validated file format. The raw response
of `board_projects` is also supported (`{"repos": {"repo": {"project": ["paths"]}}}`),
as is the single-repository flat form (repository key `default`). Ownership
follows Millboard's first matching project in name order, with path-component
boundaries and the repository's default project as fallback. It does not guess
that repository names identify applications.

`rusty-remind-me impact-export [QUERY]` reads the configured RemindMe store via
its existing local/daemon path. It emits a complete, unfiltered engine-backed
inventory, including archived, superseded, sensitive and deleted records. Keep
this explicit export private as you would a backup. The optional query reuses
existing retrieval (and its ordinary access updates); ranked context is never
used to decide which binding requirements to include. Omitting QUERY avoids
search altogether. Orch only reads the exported JSON and does not open databases,
run migrations, contact services or execute commands from the request.

Create `/tmp/change.json` (file locations are relative to this request):

```json
{
  "schema_version": 1,
  "repository": "rusty_mill",
  "changes": [{"package": "rusty_atomic_file"}],
  "projects": "projects.json",
  "requirements": "requirements.json"
}
```

Use the repository key from Millboard's registry. A change is exactly one
`{"package": "<full Cargo id or unique package name>"}` or
`{"path": "crates/foundation/rusty_atomic_file/src/lib.rs"}`. Multiple proposed
changes are supported. Deleted paths work; paths outside the repository or
through escaping symlinks fail. Unrecognized packages, unowned paths, unresolved
application ownership and non-Cargo consumers are reported as uncertainty and
broaden validation. A change to workspace-level configuration requires workspace
validation. Cargo resolve uses all features and all dependency kinds, including
optional, build, development and target-specific edges, and follows non-member
adapters too.

Record a durable requirement in an existing RemindMe memory's metadata:

```json
{
  "factory_requirement": {
    "binding": true,
    "repository": "rusty_mill",
    "projects": ["rusty_orch", "rusty_remind_me", "rusty_mill_board"],
    "packages": ["rusty_atomic_file"],
    "validation": ["Crash recovery and atomic replacement compatibility"]
  }
}
```

Project names must refer to the Millboard registry, not assumed repo basenames.
Optional `packages`, `paths`, `projects`, and `validation` are arrays of nonempty
strings; omit empty arrays. No scope means repository-wide. Scope selectors are
conservative unions: a package/path match records the named projects as consumers;
project contracts are included whenever that project requires validation. Path
contracts apply to changed paths and affected consumer crate directories. Unknown
scopes cannot exclude contracts. Inactive applications retain binding requirements.
An archived, expired, deleted or superseded memory is still included with lifecycle
evidence: use `binding: false` to explicitly withdraw it. Ordinary memories remain
context, not binding requirements. Existing requirements must be explicitly scoped
with this metadata to participate; this increment does not infer normative meaning
from prose or migrate existing memories.

The report separates resolved dependency edges, recorded ownership and requirements,
and inferred applications/retrieval context. It lists direct and transitive consumers,
required package/project validation and explicitly recorded checks. Cargo validation
entries are command templates: select the listed package IDs using Cargo `-p` options.
Project-specific non-Cargo checks require the owning project's validation instructions.
`validation.status` always says required, not executed. An unrelated project is excluded
when graph/ownership evidence supports it; unresolved consumers broaden the scope.
The repository's default ownership of a shared library is an affected project, not
an application inferred from a repo name.

For revision comparison, refresh the two exports, save the old report separately,
and add `"previous": "old-impact.json"` to the request. `changed_requirements`
compares all marked requirement IDs, updated-at revisions and fingerprints, detecting
scope/content/lifecycle changes and additions/removals even if not selected before.
`source_changed` compares source commits. Input hashes bind the registry, full snapshot,
request and resolved metadata; semantic search rank is never a completeness proof.
Exports are a point-in-time observation; a live store change is detected only after
refreshing its export. The report does not claim a service-wide revision fence.

A clean source revision is required. Keep input/output files outside the checkout.
Missing Python 3.11+, Git, frozen metadata cache or current lockfile, malformed or
incomplete snapshots and concurrent input/source changes produce an error with no
partial report. Existing `inspect` without `--impact` retains its previous behavior.
The 120-second process deadline and 1 MiB output limit still apply (oversized reports
fail explicitly). Execution, security gates, credentials and publication are unchanged.
