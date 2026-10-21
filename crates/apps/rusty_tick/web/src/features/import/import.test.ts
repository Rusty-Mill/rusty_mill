import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { applyImport, readImport } from '.'
import { parseWhen } from './model'

const csv = (...rows: string[][]): string => rows.map((r) => r.map((v) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)).join(',')).join('\r\n')

const TICK_HEADER = ['Folder Name', 'List Name', 'Title', 'Kind', 'Tags', 'Content', 'Is Check list', 'Start Date', 'Due Date', 'Reminder', 'Repeat', 'Priority', 'Status', 'Created Time', 'Completed Time', 'Order', 'Timezone', 'Is All Day', 'Is Floating', 'Column Name', 'Column Order', 'View Mode', 'taskId', 'parentId']
const tick = (o: Partial<Record<string, string>>): string[] => TICK_HEADER.map((h) => o[h] ?? '')
const ticktick = (...rows: Partial<Record<string, string>>[]): string =>
  ['"Date: 2026-10-21+0000"', '"Version: 7.1"', '"Status: \n0 Normal\n1 Completed\n2 Archived"', csv(TICK_HEADER, ...rows.map(tick))].join('\n')

const NOW = new Date(2026, 9, 21, 12).getTime()

describe('parseWhen', () => {
  it('reads a date alone as all day at local midnight', () => {
    expect(parseWhen('2026-10-05')).toEqual({ ms: new Date(2026, 9, 5).getTime(), allDay: true })
  })

  it('reads offsets with and without a colon, Z, fractions, and no zone as local', () => {
    expect(parseWhen('2026-10-05T14:00:00+0200')?.ms).toBe(Date.UTC(2026, 9, 5, 12))
    expect(parseWhen('2026-10-05T14:00:00-05:30')?.ms).toBe(Date.UTC(2026, 9, 5, 19, 30))
    expect(parseWhen('2026-10-05T09:00:00.000Z')?.ms).toBe(Date.UTC(2026, 9, 5, 9))
    expect(parseWhen('2026-10-05 09:30')?.ms).toBe(new Date(2026, 9, 5, 9, 30).getTime())
  })

  it('puts an all-day instant on its calendar day in the given zone', () => {
    // Midnight in Shanghai is 16:00 UTC the day before.
    expect(parseWhen('2026-10-04T16:00:00+0000', true, 'Asia/Shanghai')).toEqual({ ms: new Date(2026, 9, 5).getTime(), allDay: true })
  })

  it('refuses what it cannot read', () => {
    expect(parseWhen('next tuesday')).toBeNull()
    expect(parseWhen('')).toBeNull()
  })
})

describe('readImport: TickTick backup', () => {
  it('skips the preamble and reads lists, dates, priority, status, tags, repeat and reminders', () => {
    const r = readImport(
      ticktick(
        { 'List Name': 'Work', Title: 'Ship it', Tags: 'urgent, q4', Content: 'details', 'Due Date': '2026-10-05T14:00:00+0000', Priority: '5', Status: '0', Repeat: 'RRULE:FREQ=WEEKLY;INTERVAL=1', Reminder: 'TRIGGER:PT0S,-PT5M', Timezone: 'Europe/London', taskId: 'a1' },
        { 'List Name': 'Inbox', Title: 'Done one', Status: '2', taskId: 'a2' },
        { 'List Name': 'Inbox', Title: 'Not doing', Status: '-1', taskId: 'a3' },
        { 'List Name': 'Inbox', Title: '', taskId: 'a4' },
      ),
      'TickTick-backup.csv',
    )
    expect(r).toMatchObject({ source: 'TickTick', skipped: 1, lossy: 0 })
    expect(r.tasks[0]).toMatchObject({ key: 'a1', list: 'Work', title: 'Ship it', notes: 'details', tags: ['urgent', 'q4'], priority: 5, dueMs: Date.UTC(2026, 9, 5, 14), isAllDay: false, timeZone: 'Europe/London', repeatFlag: 'RRULE:FREQ=WEEKLY;INTERVAL=1', reminders: ['TRIGGER:PT0S', 'TRIGGER:-PT5M'], status: 'open' })
    expect(r.tasks.map((t) => t.status)).toEqual(['open', 'done', 'wontdo'])
  })

  it('splits checklist content into items and notes, and links subtasks', () => {
    const r = readImport(ticktick({ Title: 'Pack', Kind: 'CHECKLIST', Content: 'for the trip\n▫socks\n▪passport', taskId: 'p' }, { Title: 'Book hotel', taskId: 'c', parentId: 'p' }), 'b.csv')
    expect(r.tasks[0]).toMatchObject({ notes: 'for the trip', items: [{ title: 'socks', done: false }, { title: 'passport', done: true }] })
    expect(r.tasks[1]?.parentKey).toBe('p')
  })

  it('counts a repeat rule it does not understand and drops it', () => {
    const r = readImport(ticktick({ Title: 'Odd', Repeat: 'ERULE:NAME=CUSTOM', taskId: 'x' }), 'b.csv')
    expect(r.lossy).toBe(1)
    expect(r.tasks[0]?.repeatFlag).toBe('')
  })
})

describe('readImport: Todoist CSV', () => {
  const HEAD = ['TYPE', 'CONTENT', 'DESCRIPTION', 'PRIORITY', 'INDENT', 'AUTHOR', 'RESPONSIBLE', 'DATE', 'DATE_LANG', 'TIMEZONE']
  const row = (type: string, content: string, o: { desc?: string; pri?: string; indent?: string; date?: string } = {}): string[] => [type, content, o.desc ?? '', o.pri ?? '', o.indent ?? '', '', '', o.date ?? '', 'en', '']

  it('names the list after the file, maps priority, labels, notes, subtasks and dates', () => {
    const text = csv(
      HEAD,
      row('section', 'Week 1'),
      row('task', 'Draft brief @work', { desc: 'goals', pri: '1', indent: '1', date: '2026-10-05' }),
      row('note', 'a comment'),
      row('task', 'Outline', { pri: '4', indent: '2', date: 'tomorrow' }),
      row('task', 'Weekly review', { indent: '1', date: 'every monday' }),
    )
    const r = readImport(text, 'Launch.csv', NOW)
    expect(r).toMatchObject({ source: 'Todoist', skipped: 1, lossy: 1 })
    expect(r.tasks[0]).toMatchObject({ list: 'Launch', title: 'Draft brief', tags: ['work'], priority: 5, notes: 'goals\n\na comment', dueMs: new Date(2026, 9, 5).getTime(), isAllDay: true })
    expect(r.tasks[1]).toMatchObject({ parentKey: r.tasks[0]!.key, priority: 0, dueMs: new Date(2026, 9, 22).getTime() })
    expect(r.tasks[2]).toMatchObject({ dueMs: null, notes: 'Todoist date: every monday' })
  })
})

describe('readImport: generic CSV', () => {
  it('reads aliased columns, priority words, completion and duplicates as separate tasks', () => {
    const text = csv(['List', 'Title', 'Notes', 'Due', 'Priority', 'Tags', 'Completed'], ['Home', 'Milk', 'semi', '2026-10-05', 'high', 'flagged', ''], ['Home', 'Milk', 'semi', '2026-10-05', 'low', '', 'true'], ['', 'Call', '', 'soon', '', '', ''])
    const r = readImport(text, 'reminders.csv')
    expect(r).toMatchObject({ source: 'CSV', skipped: 0, lossy: 1 })
    expect(r.tasks[0]).toMatchObject({ list: 'Home', title: 'Milk', notes: 'semi', priority: 5, tags: ['flagged'], isAllDay: true, status: 'open' })
    expect(r.tasks[1]).toMatchObject({ priority: 1, status: 'done' })
    expect(new Set(r.tasks.map((t) => t.key)).size).toBe(3)
  })

  it('refuses a file it does not recognise', () => {
    expect(() => readImport('foo,bar\n1,2', 'x.csv')).toThrow(/Title column/)
  })
})

describe('readImport: .ics', () => {
  it('reads VTODOs through the calendar parser into the Inbox', () => {
    const text = ['BEGIN:VCALENDAR', 'BEGIN:VTODO', 'UID:r1', 'SUMMARY:Renew passport', 'DUE;VALUE=DATE:20261105', 'END:VTODO', 'END:VCALENDAR'].join('\r\n')
    const r = readImport(text, 'Reminders.ics')
    expect(r.tasks).toHaveLength(1)
    expect(r.tasks[0]).toMatchObject({ key: 'r1', list: '', title: 'Renew passport', isAllDay: true })
  })
})

describe('applyImport', () => {
  const file = ticktick(
    { 'List Name': 'Work', Title: 'Parent', taskId: 'p', Priority: '3' },
    { 'List Name': 'Elsewhere', Title: 'Child', taskId: 'c', parentId: 'p' },
    { 'List Name': 'Work', Title: 'Grandchild', taskId: 'g', parentId: 'c' },
    { 'List Name': 'inbox', Title: 'Closed', taskId: 'd', Status: '1' },
    { 'List Name': 'Work', Title: 'Pack', Kind: 'CHECKLIST', Content: '▫socks', taskId: 'k' },
  )

  it('creates missing lists by name, nests subtasks one level in their parent list, and sets status', async () => {
    const api = new MemoryAdapter()
    const n = await applyImport(api, readImport(file, 'b.csv'))
    expect(n).toEqual({ tasks: 5, lists: 1, existing: 0 })
    const snap = await api.snapshot()
    const by = (title: string) => snap.tasks.find((t) => t.title === title)!
    const work = snap.lists.find((l) => l.name === 'Work')!
    expect(by('Parent')).toMatchObject({ listId: work.id, priority: 3, parentId: null })
    expect(by('Child')).toMatchObject({ listId: work.id, parentId: by('Parent').id })
    expect(by('Grandchild')).toMatchObject({ listId: work.id, parentId: by('Parent').id })
    expect(by('Closed')).toMatchObject({ listId: snap.inboxId, status: 'done' })
    expect(by('Pack')).toMatchObject({ kind: 'checklist', items: [expect.objectContaining({ title: 'socks', done: false })] })
  })

  it('is idempotent, even after an imported list is renamed', async () => {
    const api = new MemoryAdapter()
    await applyImport(api, readImport(file, 'b.csv'))
    const work = (await api.snapshot()).lists.find((l) => l.name === 'Work')!
    await api.updateList(work.id, { name: 'Job' })
    expect(await applyImport(api, readImport(file, 'b.csv'))).toEqual({ tasks: 0, lists: 0, existing: 5 })
    expect((await api.snapshot()).lists.map((l) => l.name).sort()).toEqual(['Inbox', 'Job'])
  })

  it('treats a parent cycle as top-level tasks instead of failing', async () => {
    const api = new MemoryAdapter()
    const n = await applyImport(api, readImport(ticktick({ Title: 'A', taskId: 'a', parentId: 'b' }, { Title: 'B', taskId: 'b', parentId: 'a' }), 'b.csv'))
    expect(n.tasks).toBe(2)
  })
})
