/**
 * Re-export wrapper so the default `pnpm test` glob (`tests/*.test.ts`)
 * picks up the tests that live as a sibling of the implementation under
 * `shell/src/plugins/nexus/fileProperties/propertyEditors.test.ts`.
 *
 * Same shim pattern as `tests/unique-note-settings.test.ts`.
 */
import '../src/plugins/nexus/fileProperties/propertyEditors.test.ts'
