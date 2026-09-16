/**
 * Re-export wrapper so the default `pnpm test` glob (`tests/*.test.ts`)
 * picks up the uniqueNote settings tests that live as a sibling of the
 * implementation under
 * `shell/src/plugins/nexus/uniqueNote/uniqueNoteSettings.test.ts`.
 *
 * Same shim pattern as `tests/daily-note-format.test.ts`.
 */
import '../src/plugins/nexus/uniqueNote/uniqueNoteSettings.test.ts'
