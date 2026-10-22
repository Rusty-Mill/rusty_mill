/**
 * Import tasks from other apps. `readImport` recognises the file by its
 * content (not its name) and reads it into the shared model; `applyImport`
 * writes the result. Sources: TickTick backup, Todoist project CSV, any CSV
 * with a title column (Apple Reminders via its export script), and `.ics`
 * (VTODOs and VEVENTs, e.g. from Apple Reminders, Thunderbird or Nextcloud).
 */
import { parseCsv } from '@/lib/csv';
import { parseIcs } from '@/lib/ics';
import { isGeneric, parseGeneric } from './generic';
import { blankTask, uniqueKeys } from './model';
import { isTickTick, parseTickTick } from './ticktick';
import { isTodoist, parseTodoist } from './todoist';
export { applyImport } from './apply';
/** How many more instances than the file now holds a UID may have had when #829 imported it. */
const ORDINAL_SLACK = 16;
/**
 * Read an `.ics` file into the shared model: the one path for the calendar
 * page's import and this one, so a file imported through both adds its tasks
 * once. An event's identity is its UID plus its `RECURRENCE-ID` (none for the
 * master), so a moved instance keeps its task when others are added or the
 * file is reordered; a repeat of both (a malformed file) gets an occurrence
 * count. Exceptions travel as the master's `exDates`.
 *
 * Earlier importers keyed a row by its UID and position: #829 by `[uid, n]`,
 * before it by the bare UID. A position does not say which instance it was,
 * and moves when instances are added, removed or reordered, so a row offers
 * every position its UID may have had (up to its count here plus
 * `ORDINAL_SLACK`) and the bare UID, and each matches a task whose due date
 * is still this row's, else one edited since that only this row fits.
 */
export function readIcs(text) {
    const { tasks, skipped, droppedRepeats, droppedExceptions } = parseIcs(text);
    const key = uniqueKeys();
    const count = new Map();
    for (const t of tasks)
        count.set(t.uid, (count.get(t.uid) ?? 0) + 1);
    /** #829's keys a row with this UID may have had: `[uid, n]` for each position. */
    const ordinals = (uid) => Array.from({ length: count.get(uid) + ORDINAL_SLACK }, (_, i) => JSON.stringify([uid, i + 1]));
    return {
        source: 'calendar file',
        tasks: tasks.map(({ uid, recurrenceId, ...t }) => ({
            ...blankTask(key(uid, recurrenceId === null ? '' : String(recurrenceId)), t.title),
            ...t,
            legacy: [...ordinals(uid), uid].map((k) => ({ key: k, dueMs: t.dueMs })),
        })),
        skipped,
        lossy: droppedRepeats + droppedExceptions,
    };
}
/** Read `text` (the file `fileName`) into tasks; throws with a readable message when no source matches. */
export function readImport(text, fileName, now = Date.now()) {
    if (/^\s*BEGIN:VCALENDAR/i.test(text))
        return readIcs(text);
    const rows = parseCsv(text);
    if (isTickTick(rows))
        return parseTickTick(text);
    if (isTodoist(rows))
        return parseTodoist(text, fileName, now);
    if (isGeneric(rows))
        return parseGeneric(text);
    throw new Error('Not a file this can import: expected a TickTick backup, a Todoist CSV, a CSV with a Title column, or an .ics file');
}
