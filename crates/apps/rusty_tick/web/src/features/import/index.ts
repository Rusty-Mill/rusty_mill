/**
 * Import tasks from other apps. `readImport` recognises the file by its
 * content (not its name) and reads it into the shared model; `applyImport`
 * writes the result. Sources: TickTick backup, Todoist project CSV, any CSV
 * with a title column (Apple Reminders via its export script), and `.ics`
 * (VTODOs and VEVENTs, e.g. from Apple Reminders, Thunderbird or Nextcloud).
 */
import { parseCsv } from '@/lib/csv'
import { parseIcs } from '@/lib/ics'
import { isGeneric, parseGeneric } from './generic'
import { blankTask, type ImportResult } from './model'
import { isTickTick, parseTickTick } from './ticktick'
import { isTodoist, parseTodoist } from './todoist'

export { applyImport, type Applied } from './apply'
export type { ImportResult, ImportedTask } from './model'

function fromIcs(text: string): ImportResult {
  const { tasks, skipped, droppedRepeats } = parseIcs(text)
  return {
    source: 'calendar file',
    tasks: tasks.map((t) => ({ ...blankTask(t.uid, t.title), ...t, key: t.uid })),
    skipped,
    lossy: droppedRepeats,
  }
}

/** Read `text` (the file `fileName`) into tasks; throws with a readable message when no source matches. */
export function readImport(text: string, fileName: string, now: number = Date.now()): ImportResult {
  if (/^\s*BEGIN:VCALENDAR/i.test(text)) return fromIcs(text)
  const rows = parseCsv(text)
  if (isTickTick(rows)) return parseTickTick(text)
  if (isTodoist(rows)) return parseTodoist(text, fileName, now)
  if (isGeneric(rows)) return parseGeneric(text)
  throw new Error('Not a file this can import: expected a TickTick backup, a Todoist CSV, a CSV with a Title column, or an .ics file')
}
