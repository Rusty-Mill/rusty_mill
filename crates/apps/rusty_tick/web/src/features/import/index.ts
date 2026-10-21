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
import { blankTask, uniqueKeys, type ImportResult } from './model'
import { isTickTick, parseTickTick } from './ticktick'
import { isTodoist, parseTodoist } from './todoist'

export { applyImport, type Applied } from './apply'
export type { ImportResult, ImportedTask } from './model'

/**
 * Read an `.ics` file into the shared model: the one path for the calendar
 * page's import and this one, so a file imported through both adds its tasks
 * once. An event's identity is its UID plus its `RECURRENCE-ID` (none for the
 * master), so a moved instance keeps its task when others are added or the
 * file is reordered; a repeat of both (a malformed file) gets an occurrence
 * count. `legacyKey` is the bare UID this importer used before, so files it
 * already imported still match. Exceptions travel as the master's `exDates`.
 */
export function readIcs(text: string): ImportResult {
  const { tasks, skipped, droppedRepeats, droppedExceptions } = parseIcs(text)
  const key = uniqueKeys()
  return {
    source: 'calendar file',
    tasks: tasks.map(({ uid, recurrenceId, ...t }) => ({ ...blankTask(key(uid, recurrenceId === null ? '' : String(recurrenceId)), t.title), ...t, legacyKey: uid })),
    skipped,
    lossy: droppedRepeats + droppedExceptions,
  }
}

/** Read `text` (the file `fileName`) into tasks; throws with a readable message when no source matches. */
export function readImport(text: string, fileName: string, now: number = Date.now()): ImportResult {
  if (/^\s*BEGIN:VCALENDAR/i.test(text)) return readIcs(text)
  const rows = parseCsv(text)
  if (isTickTick(rows)) return parseTickTick(text)
  if (isTodoist(rows)) return parseTodoist(text, fileName, now)
  if (isGeneric(rows)) return parseGeneric(text)
  throw new Error('Not a file this can import: expected a TickTick backup, a Todoist CSV, a CSV with a Title column, or an .ics file')
}
