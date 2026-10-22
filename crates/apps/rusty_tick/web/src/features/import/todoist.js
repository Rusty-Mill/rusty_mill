/**
 * Todoist's project export (project ⋯ → Export as CSV): one file per project,
 * so the list is named after the file. Header `TYPE,CONTENT,DESCRIPTION,
 * PRIORITY,INDENT,...,DATE,...`. `task` rows are tasks, a `note` row is a
 * comment on the task above (kept in its notes), `section` rows are dropped.
 * Priority `1` is the highest. `@label` words in the content become tags.
 * `DATE` is free text: ISO dates and what quick add reads are used, anything
 * else (`every monday`) is kept in the notes, since guessing would be worse.
 */
import { parseQuickAdd } from '@/lib/nlp/parse';
import { parseCsv } from '@/lib/csv';
import { blankTask, columns, keepInNotes, legacyKeys, parseWhen, uniqueKeys } from './model';
export const isTodoist = (rows) => {
    const head = rows[0]?.map((h) => h.trim().toUpperCase()) ?? [];
    return head[0] === 'TYPE' && head.includes('CONTENT');
};
const PRIORITY = { '1': 5, '2': 3, '3': 1 };
function readDate(text, now) {
    if (/^\d{4}-\d{2}-\d{2}/.test(text.trim()))
        return parseWhen(text); // invalid ISO stays invalid: quick add would roll it over
    const quick = parseQuickAdd(text, now);
    return quick.dueMs !== null && quick.title === '' ? { ms: quick.dueMs, allDay: quick.isAllDay } : null;
}
const listName = (fileName) => fileName.replace(/\.csv$/i, '').trim();
export function parseTodoist(text, fileName, now) {
    const rows = parseCsv(text);
    if (!isTodoist(rows))
        throw new Error('No Todoist header row found');
    const col = columns(rows[0]);
    const key = uniqueKeys();
    const legacyKey = legacyKeys();
    const list = listName(fileName);
    const out = { source: 'Todoist', tasks: [], skipped: 0, lossy: 0 };
    let last = null;
    let root = null;
    for (const row of rows.slice(1)) {
        const type = col(row, 'type').toLowerCase();
        const content = col(row, 'content');
        if (type === 'note' && last && content) {
            last.notes = [last.notes, content].filter(Boolean).join('\n\n');
            continue;
        }
        const tags = [...content.matchAll(/(?:^|\s)@([\p{L}\p{N}_-]+)/gu)].map((m) => m[1]);
        const title = content.replace(/(?:^|\s)@[\p{L}\p{N}_-]+/gu, '').trim();
        if (type !== 'task' || !title) {
            if (type !== '')
                out.skipped++; // blank rows separate sections
            last = null;
            continue;
        }
        const dateText = col(row, 'date');
        const when = dateText ? readDate(dateText, now) : null;
        const notes = col(row, 'description');
        if (dateText && !when)
            out.lossy++;
        const indent = Number(col(row, 'indent')) || 1;
        const task = {
            ...blankTask(key(list, title), title),
            legacy: [{ key: legacyKey(`${list}|${title}`) }],
            list,
            parentKey: indent > 1 && root ? root.key : null,
            notes: dateText && !when ? keepInNotes(notes, 'Todoist date', dateText) : notes,
            priority: PRIORITY[col(row, 'priority')] ?? 0,
            dueMs: when?.ms ?? null,
            isAllDay: !!when?.allDay,
            tags,
        };
        out.tasks.push(task);
        last = task;
        if (indent <= 1)
            root = task;
    }
    return out;
}
