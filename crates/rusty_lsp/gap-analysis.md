# rusty_lsp vs. LSP 3.17 — gap analysis

**Run date:** 2026-07-23. **Reference:** the official LSP specification, version
3.17.0 final (`metaModel.json` + prose fragments pinned to the `gh-pages`
branch of `microsoft/language-server-protocol`, with every `proposed`/`3.18`-tagged
entry filtered out). **Target:** `baileyrd/rusty_lsp` at the current `main`
(v0.6.2). **Path:** step 1's "no comparable surface to diff, and no roadmap"
path — read the spec directly, since `rusty_lsp` is a framework with a
deliberately different shape from any comparable crate (see the parity-scope
decision below), and the existing hand-curated roadmap
(`rusty_naner/ECOSYSTEM.md` §5.3) scopes only release hygiene, not LSP
feature coverage.

## Headline finding

**Every one of the 90 methods LSP 3.17 defines (64 requests + 26
notifications) is already wired up** — full lifecycle, both push- and
pull-model diagnostics, notebook document sync, call/type hierarchy, semantic
tokens (full/delta/range + refresh), moniker, linked editing range, inline
value, all workspace file-operation hooks, and position-encoding negotiation
(UTF-8/16/32, including the exact spec-mandated UTF-16 fallback behavior).
There is no missing request or notification handler anywhere in the surface.

Every surviving gap below is at the **capability-negotiation / type-modeling
layer**: places where the wire protocol is already handled correctly (often
through an untyped JSON escape hatch) but a typed, ergonomic Rust API for it
doesn't exist yet, or where a `ServerCapabilities` field only models the
`boolean` half of a `boolean | XOptions` union the spec allows.

## Gap table

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `WorkspaceServerCapabilities::workspace_folders` | type (new field) | spec | both | `WorkspaceFoldersServerCapabilities{supported?, changeNotifications?}` in `ServerCapabilities.workspace` | no | S | `WorkspaceServerCapabilities` (lifecycle.rs:437) currently has only `file_operations`; a server can't advertise interest in workspace-folder change notifications. `Client::workspace_folders()` and `did_change_workspace_folders` already exist on the request/notification side — this is purely the missing capability-advertisement field. |
| `work_done_progress` on 11 `*Options` structs | type (new field) | spec | both | `WorkDoneProgressOptions` base type, extended by `CompletionOptions`, `SignatureHelpOptions`, `CodeActionOptions`, `RenameOptions`, `ExecuteCommandOptions`, `CodeLensOptions`, `DocumentLinkOptions`, `SemanticTokensOptions`, `InlayHintOptions`, `DiagnosticOptions`, `WorkspaceSymbolOptions` | no | S–M | `CallHierarchyOptions`/`TypeHierarchyOptions` already carry `work_done_progress: Option<bool>` (hierarchy.rs:165-178) — these 11 sibling structs are missing the same field despite the spec applying it uniformly. Mechanical, same pattern 11×. |
| Typed `WorkspaceClientCapabilities` accessor | type (new, additive) | spec | both | `ClientCapabilities.workspace.*` (applyEdit, workspaceEdit incl. resourceOperations/failureHandling/changeAnnotationSupport, didChangeConfiguration, didChangeWatchedFiles incl. relativePatternSupport, symbol incl. tagSupport/resolveSupport, executeCommand, workspaceFolders, configuration, semanticTokens.refreshSupport, codeLens.refreshSupport, fileOperations, inlineValue/inlayHint/diagnostics.refreshSupport) | no | M | `ClientCapabilities` is currently fully untyped (`#[serde(transparent)] raw: Map<String, Value>`, lifecycle.rs:208-213) by deliberate design (doc comment lifecycle.rs:200-207) — this adds a typed *read* accessor on top without touching `raw` or removing the existing dotted-path helpers. |
| Typed `WindowClientCapabilities` + `GeneralClientCapabilities` + `notebookDocument.synchronization` accessor | type (new, additive) | spec | both | `ClientCapabilities.window.*` (workDoneProgress, showMessage.messageActionItem, showDocument.support), `.general.*` (staleRequestSupport, regularExpressions, markdown, positionEncodings), `.notebookDocument.synchronization.*` | no | S–M | Smaller, self-contained group; `positionEncodings` already has a dedicated typed accessor (`position_encodings()`) — this covers everything else in the same three sub-trees. |
| Typed `TextDocumentClientCapabilities` — core | type (new, additive) | spec | both | `.textDocument.{synchronization, completion, hover, signatureHelp, declaration, definition, typeDefinition, implementation, references, documentHighlight, documentSymbol}` | no | M | The most commonly-probed capability group (completion item snippet/commit-characters/resolve support, hover content format, go-to-definition link support, etc.) — currently reachable only via `capabilities.get("textDocument.completion.completionItem.snippetSupport")`-style dotted paths. |
| Typed `TextDocumentClientCapabilities` — advanced, part A | type (new, additive) | spec | both | `.textDocument.{codeAction, codeLens, documentLink, colorProvider, formatting, rangeFormatting, onTypeFormatting, rename, foldingRange, selectionRange, publishDiagnostics}` | no | M | Split from the full advanced-features set (spec extraction flagged it as an L-sized single bucket) to keep each issue reviewable. |
| Typed `TextDocumentClientCapabilities` — advanced, part B | type (new, additive) | spec | both | `.textDocument.{callHierarchy, semanticTokens, linkedEditingRange, moniker, typeHierarchy, inlineValue, inlayHint, diagnostic}` | no | M | Second half of the advanced-features split. |
| Typed builders for remaining `*RegistrationOptions` shapes | type (new, additive) | spec | both | e.g. `SemanticTokensRegistrationOptions`, `DiagnosticRegistrationOptions`, `CallHierarchyRegistrationOptions`, `TypeHierarchyRegistrationOptions`, `MonikerRegistrationOptions`, `LinkedEditingRangeRegistrationOptions`, `InlineValueRegistrationOptions`, `ColorRegistrationOptions`, `FoldingRangeRegistrationOptions`, `SelectionRangeRegistrationOptions`, `DeclarationRegistrationOptions`, `TypeDefinitionRegistrationOptions`, `ImplementationRegistrationOptions` | no | M | Lower priority: `client/registerCapability` already works for every one of these today via `Registration::new(id, method, Some(serde_json::json!(...)))` (the raw-JSON escape hatch) — this is purely an ergonomic typed-builder layer, not a protocol gap. Only `TextDocumentRegistrationOptions` and `DidChangeWatchedFilesRegistrationOptions` currently have dedicated typed constructors. |
| Bool-only `ServerCapabilities` provider fields (17 fields) | fn (existing) | spec | both | `hoverProvider`, `definitionProvider`, `declarationProvider`, `typeDefinitionProvider`, `implementationProvider`, `referencesProvider`, `documentHighlightProvider`, `documentSymbolProvider`, `documentFormattingProvider`, `documentRangeFormattingProvider`, `foldingRangeProvider`, `selectionRangeProvider`, `colorProvider`, `monikerProvider`, `linkedEditingRangeProvider`, `inlineValueProvider`, `inlineCompletionProvider` — each spec'd as `boolean \| XOptions \| XRegistrationOptions` | **yes** | L | Currently all 17 are plain `Option<bool>` (lifecycle.rs, confirmed by direct read), unlike sibling fields (`codeActionProvider`, `renameProvider`, `workspaceSymbolProvider`, `callHierarchyProvider`, `typeHierarchyProvider`, `semanticTokensProvider`, …) that already use the `Simple(bool) | Options(...)` enum pattern. Converting these 17 to the same pattern changes each field's declared type — a breaking change to existing public structs — so this is **not** auto-implemented; needs an explicit decision (see below). |

## Explicitly out of scope for this run

- **The roadmap's own backlog** (`ECOSYSTEM.md` §5.3: repo URL, LICENSE files,
  tags, CI, crates.io decision) — confirmed already fully implemented at
  v0.6.2; not re-litigated here (see the parity-scope conversation that
  preceded this analysis).
- **LSP client implementation and a TCP convenience constructor** — the
  roadmap explicitly calls these non-gaps for this project's current
  consumers, and nothing in the 3.17 spec itself requires a *client*
  implementation from a *server* framework.
- **3.18-proposed additions** (`workspace/foldingRange/refresh`,
  `textDocument/inlineCompletion`, `textDocument/rangesFormatting`, etc.) —
  interestingly, `rusty_lsp` already implements several of these ahead of
  their final ratification (see service.rs/client.rs "LSP 3.18, proposed"
  doc comments). Left alone; not this run's concern either way since 3.18
  isn't final.
- **`DocumentSelector`/`TextDocumentFilter` "at least one of
  language/scheme/pattern" validation** — spec prose, not a type-system rule;
  a defensive-validation nit rather than an interop gap. Not filed.

## Next step

Rows 1–8 (the additive ones) are candidates for immediate issue filing and
implementation. Row 9 (bool-only provider fields) is flagged breaking and
needs an explicit go/no-go before any issue for it gets worked, per this
loop's standing rule.
