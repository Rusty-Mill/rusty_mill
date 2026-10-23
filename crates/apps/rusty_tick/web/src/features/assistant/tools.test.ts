import { describe, expect, it } from 'vitest'
import type { List, Task } from '@/api/types'
import { addDays, startOfDay } from '@/lib/date'
import { nextOccurrence } from '@/lib/recurrence'
import { moveTo, overdue, pickTask, readDay, readRemind, readRepeat, readWhen, remindTrigger, taskTools, type ToolContext } from './tools'

const NOW = new Date(2026, 9, 22, 12).getTime()
const task = (id: string, title: string, over: Partial<Task> = {}): Task =>
  ({ id, title, listId: 'inbox', status: 'open', dueMs: null, deletedMs: null, notes: '', tags: [], items: [], ...over }) as Task
const list = (id: string, name: string, archived = false): List => ({ id, name, archived }) as List

/** A context over `tasks` that records what the tools did. */
function context(tasks: Task[]): { c: ToolContext; done: string[] } {
  const done: string[] = []
  const c: ToolContext = {
    entities: { tasks, lists: [list('inbox', 'Inbox'), list('w', 'Work'), list('old', 'Old', true)], tags: [], inboxId: 'inbox', filters: [], assignees: {} },
    listId: 'inbox',
    now: NOW,
    createTask: async (input) => task('new', input.title, { listId: input.listId }),
    toggleDone: async (id) => void done.push(`toggle ${id}`),
    reopen: async (id) => void done.push(`reopen ${id}`),
    trash: async (id) => void done.push(`trash ${id}`),
    update: async (id, patch) => void done.push(`update ${id} ${JSON.stringify(patch)}`),
    move: async (id, listId) => void done.push(`move ${id} ${listId}`),
  }
  return { c, done }
}

const run = async (c: ToolContext, name: string, args: Record<string, string>): Promise<unknown> =>
  taskTools(c).find((t) => t.name === name)!.handler!(args)

describe('pickTask', () => {
  const tasks = [task('a', 'Buy milk'), task('b', 'Milk the cow'), task('c', 'milk')]

  it('prefers an exact title, then the only title containing the query', () => {
    expect(pickTask(tasks, ' MILK ', 'task')).toEqual({ task: tasks[2] })
    expect(pickTask(tasks, 'cow', 'task')).toEqual({ task: tasks[1] })
  })

  it('names no task for an empty or blank query', () => {
    for (const q of ['', '   ']) expect(pickTask(tasks, q, 'task')).toEqual({ error: 'Say which task.' })
  })

  it('names the candidates rather than guess, and says when nothing matches', () => {
    expect(pickTask(tasks.slice(0, 2), 'milk', 'task')).toEqual({ matches: ['Buy milk', 'Milk the cow'] })
    expect(pickTask(tasks, 'bread', 'open task')).toEqual({ error: 'No open task matches “bread”.' })
  })
})

describe('taskTools', () => {
  const yesterday = addDays(startOfDay(NOW), -1)
  const tasks = [
    task('late', 'Pay rent', { dueMs: yesterday }),
    task('today', 'Call mum', { dueMs: NOW }),
    task('later', 'Book flights', { dueMs: addDays(startOfDay(NOW), 3) }),
    task('closed', 'Buy milk', { status: 'done' }),
    task('binned', 'Old idea', { deletedMs: 1 }),
  ]

  it('adds to the open list', async () => {
    expect(await run(context(tasks).c, 'add_task', { title: 'Buy bread' })).toEqual({ title: 'Buy bread', id: 'new' })
  })

  it('lists what is due, overdue and found, and the live lists', async () => {
    const { c } = context(tasks)
    expect(overdue(tasks, NOW).map((t) => t.id)).toEqual(['late'])
    expect(await run(c, 'due_tasks', { view: 'overdue' })).toEqual({ tasks: [expect.stringMatching(/^Pay rent \(due /)] })
    expect(((await run(c, 'due_tasks', { view: 'today' })) as { tasks: string[] }).tasks).toHaveLength(2)
    expect(((await run(c, 'due_tasks', { view: 'next7' })) as { tasks: string[] }).tasks.map((l) => l.split(' (')[0])).toEqual(['Call mum', 'Book flights'])
    expect(await run(c, 'search_tasks', { query: 'flights' })).toEqual({ tasks: [expect.stringMatching(/^Book flights/)] })
    expect(await run(c, 'list_lists', {})).toEqual({ lists: ['Inbox', 'Work'] })
  })

  it('completes, reopens and trashes the one task named, among the right ones', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'complete_task', { query: 'rent' })).toEqual({ title: 'Pay rent' })
    expect(await run(c, 'complete_task', { query: 'buy milk' })).toEqual({ error: 'No open task matches “buy milk”.' })
    expect(await run(c, 'reopen_task', { query: 'buy milk' })).toEqual({ title: 'Buy milk' })
    expect(await run(c, 'delete_task', { query: 'old idea' })).toEqual({ error: 'No task matches “old idea”.' })
    expect(await run(c, 'delete_task', { query: 'call mum' })).toEqual({ title: 'Call mum' })
    expect(done).toEqual(['toggle late', 'reopen closed', 'trash today'])
  })
  it('changes nothing for an empty, blank or missing query, even with one task to pick', async () => {
    const { c, done } = context([task('only', 'Only task'), task('shut', 'Shut task', { status: 'done' })])
    for (const name of ['complete_task', 'reopen_task', 'delete_task']) {
      for (const args of [{ query: '' }, { query: '  ' }, {}] as Record<string, string>[]) expect(await run(c, name, args)).toEqual({ error: 'Say which task.' })
    }
    expect(done).toEqual([])
  })})

describe('readDay', () => {
  const day = (y: number, m: number, d: number): number => new Date(y, m - 1, d).getTime()

  it('reads the days people say, relative to now (a Thursday)', () => {
    expect(readDay('today', NOW)).toBe(day(2026, 10, 22))
    expect(readDay('Tomorrow', NOW)).toBe(day(2026, 10, 23))
    expect(readDay('friday', NOW)).toBe(day(2026, 10, 23))
    expect(readDay('on Mon', NOW)).toBe(day(2026, 10, 26))
    expect(readDay('thursday', NOW)).toBe(day(2026, 10, 29)) // today’s weekday means next week’s
    expect(readDay('in 3 days', NOW)).toBe(day(2026, 10, 25))
    expect(readDay('next week', NOW)).toBe(day(2026, 10, 29))
    expect(readDay('2026-11-05', NOW)).toBe(day(2026, 11, 5))
    expect(readDay('no date', NOW)).toBeNull()
  })

  it('reads nothing it is not sure of', () => {
    for (const text of ['soonish', '2026-02-30', '5/11', 'in many days', '']) expect(readDay(text, NOW)).toBeUndefined()
  })
})

describe('moveTo', () => {
  const friday = new Date(2026, 9, 23).getTime()

  it("keeps a timed task's time of day, and makes an undated one all-day", () => {
    const timed = task('t', 'Call', { dueMs: new Date(2026, 9, 20, 9, 30).getTime(), isAllDay: false, startMs: null })
    expect(moveTo(timed, friday)).toEqual({ dueMs: new Date(2026, 9, 23, 9, 30).getTime(), isAllDay: false })
    expect(moveTo(task('u', 'Idea', { startMs: null }), friday)).toEqual({ dueMs: friday, isAllDay: true })
  })

  it('drops a start that would follow the new due date, and clears both for no date', () => {
    const ranged = task('r', 'Trip', { startMs: new Date(2026, 9, 25).getTime(), dueMs: new Date(2026, 9, 27).getTime(), isAllDay: true })
    expect(moveTo(ranged, friday)).toEqual({ dueMs: friday, isAllDay: true, startMs: null })
    expect(moveTo(ranged, null)).toEqual({ dueMs: null, startMs: null })
  })
})

describe('update_task', () => {
  const tasks = [task('m', 'Buy milk', { startMs: null }), task('o', 'Buy oats', { startMs: null })]

  it('renames or moves the one task named', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', title: ' Buy oat milk ' })).toEqual({ title: 'Buy oat milk' })
    expect(await run(c, 'update_task', { query: 'oats', due: 'friday' })).toEqual({ title: 'Buy oats', due: expect.any(String) })
    expect(done).toEqual([
      'update m {"title":"Buy oat milk"}',
      `update o ${JSON.stringify({ dueMs: new Date(2026, 9, 23).getTime(), isAllDay: true })}`,
    ])
  })

  it('changes nothing for an unreadable day, a blank or missing change, or an unclear task', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', due: 'soonish' })).toEqual({ error: expect.stringContaining("can't read “soonish”") })
    expect(await run(c, 'update_task', { query: 'milk', title: '  ' })).toEqual({ error: 'Say the new title.' })
    expect(await run(c, 'update_task', { query: 'milk' })).toEqual({ error: 'Say what to change: a new title, a day, a priority, a list, a repeat or a reminder.' })
    expect(await run(c, 'update_task', { query: 'buy', due: 'today' })).toEqual({ matches: ['Buy milk', 'Buy oats'] })
    expect(await run(c, 'update_task', { query: ' ', due: 'today' })).toEqual({ error: 'Say which task.' })
    expect(done).toEqual([])
  })

  it('renames the task named exactly, leaving one whose title merely contains it', async () => {
    const { c, done } = context([task('m', 'Buy milk'), task('t', 'Buy milk to Talk')])
    expect(await run(c, 'update_task', { query: 'Buy milk', title: 'Talk to mum' })).toEqual({ title: 'Talk to mum' })
    expect(done).toEqual(['update m {"title":"Talk to mum"}'])
  })

  it('sets the priority of the one task named, and changes nothing for an unknown one', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', priority: 'high' })).toEqual({ title: 'Buy milk', priority: 'high' })
    expect(await run(c, 'update_task', { query: 'oats', priority: 'none' })).toEqual({ title: 'Buy oats', priority: 'none' })
    expect(await run(c, 'update_task', { query: 'milk', priority: 'urgent' })).toEqual({ error: 'Priority is high, medium, low or none.' })
    expect(await run(c, 'update_task', { query: 'buy', priority: 'low' })).toEqual({ matches: ['Buy milk', 'Buy oats'] })
    expect(done).toEqual(['update m {"priority":5}', 'update o {"priority":0}'])
  })

  it('moves the one task named to the one list named, and changes nothing otherwise', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', list: 'work' })).toEqual({ title: 'Buy milk', list: 'Work' })
    expect(await run(c, 'update_task', { query: 'oats', list: 'inbox' })).toEqual({ title: 'Buy oats', list: 'Inbox' })
    expect(await run(c, 'update_task', { query: 'milk', list: 'Old' })).toEqual({ error: 'No list matches “Old”.' }) // archived
    expect(await run(c, 'update_task', { query: 'milk', list: 'o' })).toEqual({ error: expect.stringMatching(/^Several lists match “o”: Inbox, Work/) })
    expect(await run(c, 'update_task', { query: 'milk', list: ' ' })).toEqual({ error: 'Say which list.' })
    expect(await run(c, 'update_task', { query: 'buy', list: 'Work' })).toEqual({ matches: ['Buy milk', 'Buy oats'] })
    expect(done).toEqual(['move m w', 'move o inbox'])
  })

  it('reschedules the task whose title holds "to list", and moves nothing between lists', async () => {
    const { c, done } = context([task('a', 'Add', { startMs: null }), task('g', 'Add to list groceries', { startMs: null })])
    expect(await run(c, 'update_task', { query: 'Add to list groceries', due: 'friday' })).toEqual({ title: 'Add to list groceries', due: expect.any(String) })
    expect(done).toEqual([`update g ${JSON.stringify({ dueMs: new Date(2026, 9, 23).getTime(), isAllDay: true })}`])
  })

  it('moves the task named to the list named whole, even with "to list" in it', async () => {
    const { c, done } = context([task('m', 'Buy milk'), task('q', 'Buy milk to list "Things')])
    c.entities.lists.push(list('t', 'Things to list Work'))
    expect(await run(c, 'update_task', { query: 'Buy milk', list: 'Things to list Work' })).toEqual({ title: 'Buy milk', list: 'Things to list Work' })
    expect(done).toEqual(['move m t'])
  })
})

describe('readWhen', () => {
  const day = (d: number): number => new Date(2026, 9, d).getTime()

  it('reads a day, a time of day, or both', () => {
    expect(readWhen('friday', NOW)).toEqual({ day: day(23) })
    expect(readWhen('Friday at 3pm', NOW)).toEqual({ day: day(23), time: { h: 15, m: 0 } })
    expect(readWhen('tomorrow 9:30', NOW)).toEqual({ day: day(23), time: { h: 9, m: 30 } })
    expect(readWhen('2026-11-05 15:00', NOW)).toEqual({ day: new Date(2026, 10, 5).getTime(), time: { h: 15, m: 0 } })
    expect(readWhen('3:30 pm', NOW)).toEqual({ time: { h: 15, m: 30 } })
    expect(readWhen('at 9', NOW)).toEqual({ time: { h: 9, m: 0 } })
    expect(readWhen('12am', NOW)).toEqual({ time: { h: 0, m: 0 } })
    expect(readWhen('noon', NOW)).toEqual({ time: { h: 12, m: 0 } })
    expect(readWhen('friday all day', NOW)).toEqual({ day: day(23), time: 'all-day' })
    expect(readWhen('all day', NOW)).toEqual({ time: 'all-day' })
    expect(readWhen('none', NOW)).toBeNull()
  })

  it('reads nothing it is not sure of', () => {
    for (const text of ['3', 'friday 3', '25:00', '13pm', '9:75', 'at 24', 'soonish at 3pm', 'friday at']) expect(readWhen(text, NOW), text).toBeUndefined()
  })
})

describe('update_task with a time', () => {
  const timed = task('t', 'Call mum', { dueMs: new Date(2026, 9, 20, 9, 30).getTime(), isAllDay: false, startMs: null })
  const undated = task('u', 'Buy milk', { startMs: null })

  it('sets the day and time, keeps the day for a time alone, and drops the time for all day', async () => {
    const { c, done } = context([timed, undated])
    await run(c, 'update_task', { query: 'buy milk', due: 'friday at 3pm' })
    await run(c, 'update_task', { query: 'call mum', due: '4:15pm' })
    await run(c, 'update_task', { query: 'buy milk', due: 'at 9' })
    await run(c, 'update_task', { query: 'call mum', due: 'all day' })
    expect(done).toEqual([
      `update u ${JSON.stringify({ dueMs: new Date(2026, 9, 23, 15).getTime(), isAllDay: false })}`,
      `update t ${JSON.stringify({ dueMs: new Date(2026, 9, 20, 16, 15).getTime(), isAllDay: false })}`,
      `update u ${JSON.stringify({ dueMs: new Date(2026, 9, 22, 9).getTime(), isAllDay: false })}`,
      `update t ${JSON.stringify({ dueMs: new Date(2026, 9, 20).getTime(), isAllDay: true })}`,
    ])
  })

  it('changes nothing for a time it cannot read', async () => {
    const { c, done } = context([timed])
    expect(await run(c, 'update_task', { query: 'call mum', due: 'friday at 13pm' })).toEqual({ error: expect.stringContaining("can't read") })
    expect(done).toEqual([])
  })
})

describe('readRepeat', () => {
  it('reads how often people say a task repeats', () => {
    expect(readRepeat('daily')).toBe('RRULE:FREQ=DAILY')
    expect(readRepeat('every week')).toBe('RRULE:FREQ=WEEKLY')
    expect(readRepeat('Every 2 weeks')).toBe('RRULE:FREQ=WEEKLY;INTERVAL=2')
    expect(readRepeat('annually')).toBe('RRULE:FREQ=YEARLY')
    expect(readRepeat('every 3 months')).toBe('RRULE:FREQ=MONTHLY;INTERVAL=3')
    expect(readRepeat('on weekdays')).toBe('RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR')
    expect(readRepeat('every weekday')).toBe('RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR')
    expect(readRepeat('every friday and monday')).toBe('RRULE:FREQ=WEEKLY;BYDAY=MO,FR')
    expect(readRepeat('every mon, wed, fri')).toBe('RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR')
    expect(readRepeat('never')).toBe('')
  })

  it('reads nothing it is not sure of', () => {
    for (const text of ['every blue moon', 'every 0 days', 'every 100 days', 'every monday and someday', 'fortnightly', '']) expect(readRepeat(text), text).toBeUndefined()
  })
})

describe('update_task with a repeat', () => {
  it('sets the rule, gives an undated task today, and clears it for never', async () => {
    const dated = task('d', 'Gym', { dueMs: new Date(2026, 9, 20).getTime(), isAllDay: true, repeatFlag: '' })
    const undated = task('u', 'Water plants', { repeatFlag: '' })
    const { c, done } = context([dated, undated])
    expect(await run(c, 'update_task', { query: 'gym', repeat: 'every monday and friday' })).toEqual({ title: 'Gym', repeat: 'every week on Mon, Fri' })
    expect(await run(c, 'update_task', { query: 'water', repeat: 'every 2 weeks' })).toEqual({ title: 'Water plants', repeat: 'every 2 weeks', due: expect.any(String) })
    expect(await run(c, 'update_task', { query: 'gym', repeat: 'never' })).toEqual({ title: 'Gym', repeat: 'never' })
    expect(done).toEqual([
      'update d {"repeatFlag":"RRULE:FREQ=WEEKLY;BYDAY=MO,FR"}',
      `update u ${JSON.stringify({ repeatFlag: 'RRULE:FREQ=WEEKLY;INTERVAL=2', dueMs: new Date(2026, 9, 22).getTime(), isAllDay: true })}`,
      'update d {"repeatFlag":""}',
    ])
  })

  it('keeps a monthly task on its day of month across February', async () => {
    const jan31 = new Date(2027, 0, 31, 9).getTime()
    const { c, done } = context([task('m', 'Rent', { dueMs: jan31, isAllDay: false, repeatFlag: '' }), task('u', 'Review', { repeatFlag: '' })])
    expect(await run(c, 'update_task', { query: 'rent', repeat: 'monthly' })).toEqual({ title: 'Rent', repeat: 'every month on the 31st' })
    expect(await run(c, 'update_task', { query: 'rent', repeat: 'every 2 months' })).toEqual({ title: 'Rent', repeat: expect.stringContaining('31st') })
    await run(c, 'update_task', { query: 'review', repeat: 'monthly' })
    const rules = done.map((d) => JSON.parse(d.slice(d.indexOf('{'))).repeatFlag as string)
    expect(rules).toEqual(['RRULE:FREQ=MONTHLY;BYMONTHDAY=31', 'RRULE:FREQ=MONTHLY;INTERVAL=2;BYMONTHDAY=31', 'RRULE:FREQ=MONTHLY;BYMONTHDAY=22'])
    const feb = nextOccurrence(rules[0]!, jan31)!
    expect(new Date(feb)).toEqual(new Date(2027, 1, 28, 9))
    expect(new Date(nextOccurrence(rules[0]!, feb)!)).toEqual(new Date(2027, 2, 31, 9))
    const dec31 = new Date(2026, 11, 31, 9).getTime()
    const two = nextOccurrence(rules[1]!, dec31)!
    expect(new Date(two)).toEqual(new Date(2027, 1, 28, 9))
    expect(new Date(nextOccurrence(rules[1]!, two)!)).toEqual(new Date(2027, 3, 30, 9))
    expect(new Date(nextOccurrence(rules[1]!, nextOccurrence(rules[1]!, two)!)!)).toEqual(new Date(2027, 5, 30, 9))
  })

  it('changes nothing for a repeat it cannot read', async () => {
    const { c, done } = context([task('d', 'Gym')])
    expect(await run(c, 'update_task', { query: 'gym', repeat: 'every blue moon' })).toEqual({ error: expect.stringContaining("can't read") })
    expect(done).toEqual([])
  })
})

describe('readRemind', () => {
  it('reads when people say to be reminded', () => {
    expect(readRemind('1 hour before')).toEqual({ n: 1, unit: 'hour' })
    expect(readRemind('A day before')).toEqual({ n: 1, unit: 'day' })
    expect(readRemind('30 mins before')).toEqual({ n: 30, unit: 'minute' })
    expect(readRemind('2 days before')).toEqual({ n: 2, unit: 'day' })
    expect(readRemind('at the time')).toEqual({ n: 0, unit: 'day' })
    expect(readRemind('on the day')).toEqual({ n: 0, unit: 'day' })
    expect(readRemind('none')).toBe('none')
  })

  it('reads nothing it is not sure of', () => {
    for (const text of ['soon', '1 week before', '100 hours before', '31 days before', '1 hour after', 'an hour', '']) expect(readRemind(text), text).toBeUndefined()
  })

  it('writes the triggers the date picker writes', () => {
    expect(remindTrigger({ n: 0, unit: 'day' }, false)).toBe('TRIGGER:PT0S')
    expect(remindTrigger({ n: 5, unit: 'minute' }, false)).toBe('TRIGGER:-PT5M')
    expect(remindTrigger({ n: 1, unit: 'hour' }, false)).toBe('TRIGGER:-PT1H')
    expect(remindTrigger({ n: 1, unit: 'day' }, false)).toBe('TRIGGER:-P1D')
    expect(remindTrigger({ n: 0, unit: 'day' }, true)).toBe('TRIGGER:PT9H')
    expect(remindTrigger({ n: 1, unit: 'day' }, true)).toBe('TRIGGER:-PT15H')
    expect(remindTrigger({ n: 2, unit: 'day' }, true)).toBe('TRIGGER:-PT39H')
    expect(remindTrigger({ n: 1, unit: 'hour' }, true)).toBeUndefined()
  })
})

describe('update_task with a reminder', () => {
  const timed = task('t', 'Dentist', { dueMs: new Date(2026, 9, 23, 15).getTime(), isAllDay: false, reminders: ['TRIGGER:-PT5M'] })
  const allDay = task('a', 'Bins', { dueMs: new Date(2026, 9, 23).getTime(), isAllDay: true, reminders: [] })

  it('sets one reminder from the due time, or 09:00 for an all-day task, and clears it for none', async () => {
    const { c, done } = context([timed, allDay])
    expect(await run(c, 'update_task', { query: 'dentist', remind: '1 hour before' })).toEqual({ title: 'Dentist', remind: '1 hour before' })
    expect(await run(c, 'update_task', { query: 'bins', remind: 'a day before' })).toEqual({ title: 'Bins', remind: '1 day before, at 09:00' })
    expect(await run(c, 'update_task', { query: 'bins', remind: 'on the day' })).toEqual({ title: 'Bins', remind: 'on the day, at 09:00' })
    expect(await run(c, 'update_task', { query: 'dentist', remind: 'none' })).toEqual({ title: 'Dentist', remind: 'none' })
    expect(done).toEqual([
      'update t {"reminders":["TRIGGER:-PT1H"]}',
      'update a {"reminders":["TRIGGER:-PT15H"]}',
      'update a {"reminders":["TRIGGER:PT9H"]}',
      'update t {"reminders":[]}',
    ])
  })

  it('changes nothing for a reminder it cannot read or set', async () => {
    const { c, done } = context([timed, allDay, task('u', 'Someday', { reminders: [] })])
    expect(await run(c, 'update_task', { query: 'dentist', remind: 'whenever' })).toEqual({ error: expect.stringContaining("can't read") })
    expect(await run(c, 'update_task', { query: 'bins', remind: '1 hour before' })).toEqual({ error: expect.stringContaining('all day') })
    expect(await run(c, 'update_task', { query: 'someday', remind: '1 hour before' })).toEqual({ error: expect.stringContaining('no date') })
    expect(done).toEqual([])
  })
})
