/**
 * TickTick's backup (web app: Settings → Account → Generate Backup): a CSV with
 * a few preamble lines, then a header naming `Folder Name`, `List Name`,
 * `Title`, ..., `taskId`, `parentId`. Columns are read by name, so added or
 * reordered ones do not matter. Status is `0` open, `1`/`2` done, `-1` won't do.
 * Checklist lines in `Content` start with `▫` (open) or `▪` (done).
 */
import type { Status } from '@/api/types'
import { parseCsv } from '@/lib/csv'
import { blankTask, columns, keepInNotes, parseWhen, readPriority, readRepeat, readTags, unreadDates, type ImportResult, type ImportedTask } from './model'

export const isTickTick = (rows: string[][]): boolean => headerIndex(rows) >= 0

const headerIndex = (rows: string[][]): number => rows.findIndex((r) => r.includes('List Name') && r.includes('Title') && r.includes('taskId'))

const STATUS: Record<string, Status> = { '0': 'open', '1': 'done', '2': 'done', '-1': 'wontdo' }

/** `TRIGGER:-PT5M`, comma-separated; bare durations get the prefix. */
const readReminders = (text: string): string[] =>
  text
    .split(',')
    .map((r) => r.trim())
    .filter((r) => /^(TRIGGER:)?-?P/i.test(r))
    .map((r) => (/^TRIGGER:/i.test(r) ? r : `TRIGGER:${r}`))

function splitChecklist(content: string): Pick<ImportedTask, 'notes' | 'items'> {
  const items: ImportedTask['items'] = []
  const notes: string[] = []
  for (const line of content.split('\n')) {
    const m = /^\s*([▫▪])\s*(.*)$/.exec(line)
    if (m && m[2]!.trim()) items.push({ title: m[2]!.trim(), done: m[1] === '▪' })
    else notes.push(line)
  }
  return { notes: notes.join('\n').trim(), items }
}

export function parseTickTick(text: string): ImportResult {
  const rows = parseCsv(text)
  const at = headerIndex(rows)
  if (at < 0) throw new Error('No TickTick header row found')
  const col = columns(rows[at]!)
  const out: ImportResult = { source: 'TickTick', tasks: [], skipped: 0, lossy: 0 }
  for (const [i, row] of rows.slice(at + 1).entries()) {
    const title = col(row, 'title')
    if (!title) {
      out.skipped++
      continue
    }
    const zone = col(row, 'timezone')
    const allDay = col(row, 'is all day').toLowerCase() === 'true'
    const [startText, dueText] = [col(row, 'start date'), col(row, 'due date')]
    const start = parseWhen(startText, allDay, zone)
    const due = parseWhen(dueText, allDay, zone)
    const rule = col(row, 'repeat')
    const repeatFlag = readRepeat(rule)
    const checklist = col(row, 'kind').toUpperCase() === 'CHECKLIST' || col(row, 'is check list').toUpperCase() === 'Y'
    const body = checklist ? splitChecklist(col(row, 'content')) : { notes: col(row, 'content'), items: [] }
    const unread = unreadDates([['TickTick start date', startText, start], ['TickTick due date', dueText, due]])
    if (unread.length > 0 || (rule && !repeatFlag)) out.lossy++
    const parent = col(row, 'parentid')
    const task: ImportedTask = {
      ...blankTask(col(row, 'taskid') || `row-${i}`, title),
      list: col(row, 'list name'),
      parentKey: parent && parent !== '0' ? parent : null,
      items: body.items,
      notes: unread.reduce((notes, [label, text]) => keepInNotes(notes, label, text), body.notes),
      priority: readPriority(col(row, 'priority')),
      startMs: start && due && start.ms !== due.ms ? start.ms : null,
      dueMs: (due ?? start)?.ms ?? null,
      isAllDay: !!(due ?? start)?.allDay,
      timeZone: allDay || col(row, 'is floating').toLowerCase() === 'true' ? '' : zone,
      reminders: readReminders(col(row, 'reminder')),
      repeatFlag,
      tags: readTags(col(row, 'tags')),
      status: STATUS[col(row, 'status')] ?? 'open',
    }
    out.tasks.push(task)
  }
  return out
}
