/**
 * Any CSV with a header row and a title column: spreadsheets, other apps'
 * exports, and Apple Reminders through `tools/export-apple-reminders.js`.
 * Header names are matched case-insensitively against the aliases below.
 * Dates are ISO (`2026-10-05` is all day); priority is `0|1|3|5` or a word;
 * `completed` is true for `true`, `yes`, `y`, `1`, `x`, `done` or `completed`.
 */
import { parseCsv } from '@/lib/csv'
import { blankTask, columns, keepInNotes, parseWhen, readPriority, readRepeat, readTags, uniqueKeys, unreadDates, type ImportResult } from './model'

const TITLE = ['title', 'name', 'task', 'task name', 'subject', 'summary']
const NOTES = ['notes', 'note', 'description', 'body', 'details']
const LIST = ['list', 'list name', 'project', 'folder', 'category']
const DUE = ['due', 'due date', 'due_date', 'deadline', 'date']
const START = ['start', 'start date', 'start_date']
const TAGS = ['tags', 'tag', 'labels', 'label']
const DONE = ['completed', 'done', 'is completed', 'status']
const DONE_WORDS = new Set(['true', 'yes', 'y', '1', 'x', 'done', 'completed'])

export const isGeneric = (rows: string[][]): boolean => rows[0]?.some((h) => TITLE.includes(h.trim().toLowerCase())) ?? false

export function parseGeneric(text: string): ImportResult {
  const rows = parseCsv(text)
  if (!isGeneric(rows)) throw new Error(`No title column found; name one of: ${TITLE.join(', ')}`)
  const col = columns(rows[0]!)
  const key = uniqueKeys()
  const out: ImportResult = { source: 'CSV', tasks: [], skipped: 0, lossy: 0 }
  for (const row of rows.slice(1)) {
    const title = col(row, ...TITLE)
    if (!title) {
      out.skipped++
      continue
    }
    const list = col(row, ...LIST)
    const [dueText, startText, rule] = [col(row, ...DUE), col(row, ...START), col(row, 'repeat', 'rrule')]
    const due = parseWhen(dueText)
    const start = parseWhen(startText)
    const repeatFlag = readRepeat(rule)
    const unread = unreadDates([['Due', dueText, due], ['Start', startText, start]])
    if (unread.length > 0 || (rule && !repeatFlag)) out.lossy++
    out.tasks.push({
      ...blankTask(key(list, title, dueText), title),
      list,
      notes: unread.reduce((notes, [label, text]) => keepInNotes(notes, label, text), col(row, ...NOTES)),
      priority: readPriority(col(row, 'priority')),
      startMs: start && due ? start.ms : null,
      dueMs: (due ?? start)?.ms ?? null,
      isAllDay: !!(due ?? start)?.allDay,
      repeatFlag,
      tags: readTags(col(row, ...TAGS)),
      status: DONE_WORDS.has(col(row, ...DONE).toLowerCase()) ? 'done' : 'open',
    })
  }
  return out
}
