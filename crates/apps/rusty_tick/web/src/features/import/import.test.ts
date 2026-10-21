import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { applyImport, readImport } from '.'
import { legacyKeys, parseWhen } from './model'
import { derivedId } from '@/lib/id'

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

  it('refuses out-of-range fields instead of rolling them over into a different deadline', () => {
    for (const bad of ['2026-02-30', '2026-02-29', '2026-13-01', '2026-00-10', '2026-04-31', '2026-10-05T25:00', '2026-10-05T24:00', '2026-10-05T10:60', '2026-10-05T10:00:60', '2026-10-05T10:00+15:00', '2026-10-05T10:00+0560']) {
      expect(parseWhen(bad), bad).toBeNull()
    }
  })

  it('accepts the edges of the valid ranges, leap days included', () => {
    expect(parseWhen('2028-02-29')?.ms).toBe(new Date(2028, 1, 29).getTime())
    expect(parseWhen('2000-02-29')?.ms).toBe(new Date(2000, 1, 29).getTime())
    expect(parseWhen('1900-02-29')).toBeNull()
    expect(parseWhen('2026-12-31T23:59:59Z')?.ms).toBe(Date.UTC(2026, 11, 31, 23, 59, 59))
    expect(parseWhen('2026-10-05T00:00+14:00')?.ms).toBe(Date.UTC(2026, 9, 4, 10))
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

  it('keeps an invalid date in the notes and sets no deadline', () => {
    const r = readImport(ticktick({ Title: 'Bad', Content: 'body', 'Due Date': '2026-02-30T10:00:00+0000', taskId: 'b' }), 'b.csv')
    expect(r.lossy).toBe(1)
    expect(r.tasks[0]).toMatchObject({ dueMs: null, notes: 'body\n\nTickTick due date: 2026-02-30T10:00:00+0000' })
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

  it('keeps an invalid ISO date in the notes rather than letting quick add roll it over', () => {
    const r = readImport(csv(HEAD, row('task', 'Leap', { date: '2026-02-30' })), 'P.csv', NOW)
    expect(r.lossy).toBe(1)
    expect(r.tasks[0]).toMatchObject({ dueMs: null, notes: 'Todoist date: 2026-02-30' })
  })

  it('gives duplicate and suffix-looking titles distinct keys that repeat on re-import', () => {
    const text = csv(HEAD, row('task', 'Task'), row('task', 'Task'), row('task', 'Task#2'), row('task', 'a|b'))
    const once = readImport(text, 'Project.csv', NOW).tasks.map((t) => t.key)
    expect(new Set(once).size).toBe(4)
    expect(readImport(text, 'Project.csv', NOW).tasks.map((t) => t.key)).toEqual(once)
  })

  it('imports every one of them, and nothing on a second import', async () => {
    const text = csv(HEAD, row('task', 'Task'), row('task', 'Task'), row('task', 'Task#2'))
    const api = new MemoryAdapter()
    expect((await applyImport(api, readImport(text, 'Project.csv', NOW))).tasks).toBe(3)
    expect((await applyImport(api, readImport(text, 'Project.csv', NOW))).existing).toBe(3)
    expect((await api.snapshot()).tasks.map((t) => t.title).sort()).toEqual(['Task', 'Task', 'Task#2'])
  })
})

describe('readImport: generic CSV', () => {
  it('reads aliased columns, priority words, completion and duplicates as separate tasks', () => {
    const text = csv(['List', 'Title', 'Notes', 'Due', 'Priority', 'Tags', 'Completed'], ['Home', 'Milk', 'semi', '2026-10-05', 'high', 'flagged', ''], ['Home', 'Milk', 'semi', '2026-10-05', 'low', '', 'true'], ['', 'Call', '', 'soon', '', '', ''])
    const r = readImport(text, 'reminders.csv')
    expect(r).toMatchObject({ source: 'CSV', skipped: 0, lossy: 1 })
    expect(r.tasks[2]).toMatchObject({ dueMs: null, notes: 'Due: soon' })
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
    expect(n).toEqual({ tasks: 5, lists: 1, existing: 0, resumed: 0 })
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
    expect(await applyImport(api, readImport(file, 'b.csv'))).toEqual({ tasks: 0, lists: 0, existing: 5, resumed: 0 })
    expect((await api.snapshot()).lists.map((l) => l.name).sort()).toEqual(['Inbox', 'Job'])
  })

  /** A MemoryAdapter whose next status write fails, as a dropped request would. */
  const failingStatusOnce = (): MemoryAdapter => {
    const api = new MemoryAdapter()
    const update = api.updateTask.bind(api)
    let failed = false
    api.updateTask = async (id, patch, etag) => {
      if (patch.status !== undefined && !failed) {
        failed = true
        throw new Error('network down')
      }
      return update(id, patch, etag)
    }
    return api
  }
  const closed = ticktick({ Title: 'Closed', taskId: 'd', Status: '1' }, { Title: 'Later', taskId: 'e' })

  it('finishes a status write lost between create and update on the next import', async () => {
    const api = failingStatusOnce()
    await expect(applyImport(api, readImport(closed, 'b.csv'))).rejects.toThrow('network down')
    expect((await api.snapshot()).tasks.find((t) => t.title === 'Closed')?.status).toBe('open')
    expect(await applyImport(api, readImport(closed, 'b.csv'))).toEqual({ tasks: 1, lists: 0, existing: 0, resumed: 1 })
    expect((await api.snapshot()).tasks.find((t) => t.title === 'Closed')?.status).toBe('done')
    expect(await applyImport(api, readImport(closed, 'b.csv'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 0 })
  })

  it('leaves a task edited since the failed write alone, rather than overwrite the edit', async () => {
    const api = failingStatusOnce()
    await expect(applyImport(api, readImport(closed, 'b.csv'))).rejects.toThrow()
    const task = (await api.snapshot()).tasks.find((t) => t.title === 'Closed')!
    await api.updateTask(task.id, { title: 'Closed, renamed' })
    expect((await applyImport(api, readImport(closed, 'b.csv'))).resumed).toBe(0)
    expect((await api.snapshot()).tasks.find((t) => t.id === task.id)).toMatchObject({ title: 'Closed, renamed', status: 'open' })
  })

  it('does not reopen or re-close a task the user changed after a clean import', async () => {
    const api = new MemoryAdapter()
    await applyImport(api, readImport(closed, 'b.csv'))
    const task = (await api.snapshot()).tasks.find((t) => t.title === 'Closed')!
    await api.updateTask(task.id, { status: 'open' })
    expect((await applyImport(api, readImport(closed, 'b.csv'))).resumed).toBe(0)
    expect((await api.snapshot()).tasks.find((t) => t.id === task.id)?.status).toBe('open')
  })

  it('treats a parent cycle as top-level tasks instead of failing', async () => {
    const api = new MemoryAdapter()
    const n = await applyImport(api, readImport(ticktick({ Title: 'A', taskId: 'a', parentId: 'b' }, { Title: 'B', taskId: 'b', parentId: 'a' }), 'b.csv'))
    expect(n.tasks).toBe(2)
  })
})

describe('applyImport: files imported before the key change (#821)', () => {
  const HEAD = ['TYPE', 'CONTENT', 'DESCRIPTION', 'PRIORITY', 'INDENT', 'AUTHOR', 'RESPONSIBLE', 'DATE', 'DATE_LANG', 'TIMEZONE']
  const task = (content: string, indent = '1'): string[] => ['task', content, '', '', indent, '', '', '', 'en', '']
  const todoist = csv(HEAD, task('Task'), task('Task'), task('Task#2'), task('Parent'), task('Child', '2'))
  /** The id #821 gave a row: `derivedId("import|<source>|<legacy key>")`. */
  const oldId = (source: string, key: string): string => derivedId(`import|${source}|${key}`)

  /** What #821 left after importing `todoist`: the colliding `Task#2` row was never created. */
  async function legacyTodoist(api: MemoryAdapter): Promise<string> {
    const list = await api.createList({ id: derivedId('import|list|p'), name: 'P' })
    await api.createTask({ id: oldId('Todoist', 'P|Task'), listId: list.id, title: 'Task' })
    await api.createTask({ id: oldId('Todoist', 'P|Task#2'), listId: list.id, title: 'Task' })
    await api.createTask({ id: oldId('Todoist', 'P|Parent'), listId: list.id, title: 'Parent' })
    await api.createTask({ id: oldId('Todoist', 'P|Child'), listId: list.id, title: 'Child', parentId: oldId('Todoist', 'P|Parent') })
    return list.id
  }

  it('replays the old keys in file order, collisions included', () => {
    const legacy = legacyKeys()
    expect(['P|Task', 'P|Task', 'P|Task#2'].map((base) => legacy(base))).toEqual(['P|Task', 'P|Task#2', 'P|Task#2'])
  })

  it('gives a colliding old key to the row #821 wrote first (top-level before subtasks), not the first in the file', async () => {
    // #821 wrote Parent and the top-level Task#2 (key P|Task#2), then the first Task child (P|Task);
    // the second Task child (also P|Task#2) collided and was never written.
    const file = csv(HEAD, task('Parent'), task('Task', '2'), task('Task', '2'), task('Task#2'))
    const api = new MemoryAdapter()
    const list = await api.createList({ id: derivedId('import|list|p'), name: 'P' })
    await api.createTask({ id: oldId('Todoist', 'P|Parent'), listId: list.id, title: 'Parent' })
    await api.createTask({ id: oldId('Todoist', 'P|Task#2'), listId: list.id, title: 'Task#2' })
    await api.createTask({ id: oldId('Todoist', 'P|Task'), listId: list.id, title: 'Task', parentId: oldId('Todoist', 'P|Parent') })
    await api.updateTask(oldId('Todoist', 'P|Task#2'), { notes: 'mine' })

    expect(await applyImport(api, readImport(file, 'P.csv', NOW))).toEqual({ tasks: 1, lists: 0, existing: 3, resumed: 0 })
    const tasks = (await api.snapshot()).tasks
    expect(tasks).toHaveLength(4)
    expect(tasks.filter((t) => t.title === 'Task#2')).toEqual([expect.objectContaining({ id: oldId('Todoist', 'P|Task#2'), parentId: null, notes: 'mine' })])
    expect(tasks.filter((t) => t.title === 'Task').map((t) => t.parentId)).toEqual([oldId('Todoist', 'P|Parent'), oldId('Todoist', 'P|Parent')])
    expect(await applyImport(api, readImport(file, 'P.csv', NOW))).toEqual({ tasks: 0, lists: 0, existing: 4, resumed: 0 })
  })

  it('matches each old task once, imports the row #821 lost, and keeps edits and parent links', async () => {
    const api = new MemoryAdapter()
    await legacyTodoist(api)
    await api.updateTask(oldId('Todoist', 'P|Task'), { title: 'Task (edited)' })

    expect(await applyImport(api, readImport(todoist, 'P.csv', NOW))).toEqual({ tasks: 1, lists: 0, existing: 4, resumed: 0 })
    const snap = await api.snapshot()
    const mine = snap.tasks.filter((t) => t.title !== 'Inbox')
    expect(mine.map((t) => t.title).sort()).toEqual(['Child', 'Parent', 'Task', 'Task (edited)', 'Task#2'])
    expect(mine.find((t) => t.title === 'Child')?.parentId).toBe(oldId('Todoist', 'P|Parent'))
    expect(await applyImport(api, readImport(todoist, 'P.csv', NOW))).toEqual({ tasks: 0, lists: 0, existing: 5, resumed: 0 })
    expect((await api.snapshot()).tasks).toHaveLength(5)
  })

  it('finishes a completion #821 lost, and leaves an old task the user edited alone', async () => {
    const generic = csv(['List', 'Title', 'Completed'], ['', 'Lost close', 'true'], ['', 'Edited', 'true'])
    const api = new MemoryAdapter()
    const inbox = (await api.snapshot()).inboxId
    await api.createTask({ id: oldId('CSV', '|Lost close|'), listId: inbox, title: 'Lost close' }) // its status write failed
    await api.createTask({ id: oldId('CSV', '|Edited|'), listId: inbox, title: 'Edited' })
    await api.updateTask(oldId('CSV', '|Edited|'), { notes: 'mine' })

    expect(await applyImport(api, readImport(generic, 'x.csv'))).toEqual({ tasks: 0, lists: 0, existing: 1, resumed: 1 })
    const snap = await api.snapshot()
    expect(snap.tasks).toHaveLength(2)
    expect(snap.tasks.find((t) => t.title === 'Lost close')?.status).toBe('done')
    expect(snap.tasks.find((t) => t.title === 'Edited')).toMatchObject({ status: 'open', notes: 'mine' })
    expect(await applyImport(api, readImport(generic, 'x.csv'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 0 })
  })

  it('creates fresh tasks for rows whose old task the user has since deleted', async () => {
    const api = new MemoryAdapter()
    await legacyTodoist(api)
    await api.purgeTask(oldId('Todoist', 'P|Parent')) // takes Child with it
    const n = await applyImport(api, readImport(todoist, 'P.csv', NOW))
    expect(n).toEqual({ tasks: 3, lists: 0, existing: 2, resumed: 0 })
    const snap = await api.snapshot()
    const parent = snap.tasks.find((t) => t.title === 'Parent')!
    expect(snap.tasks.find((t) => t.title === 'Child')?.parentId).toBe(parent.id)
  })
})

