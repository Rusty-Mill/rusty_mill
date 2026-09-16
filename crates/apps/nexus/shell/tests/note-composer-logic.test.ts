/**
 * Re-export wrapper so the default `pnpm test` glob (`tests/*.test.ts`)
 * picks up the noteComposer logic tests that live as a sibling of the
 * implementation under
 * `shell/src/plugins/nexus/noteComposer/noteComposerLogic.test.ts`.
 *
 * Same shim pattern as `tests/unique-note-settings.test.ts`.
 */
import '../src/plugins/nexus/noteComposer/noteComposerLogic.test.ts'
