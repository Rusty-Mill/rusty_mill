/** Sample data for the in-browser demo: five lists, ~30 tasks with a spread of dates, priorities and tags. */
import { addDays, atTime, startOfDay } from '@/lib/date'
import { newId } from '@/lib/id'
import type { List, Priority, Tag, Task } from './types'
import { INBOX_ID } from './memory'

export function seedData(now: number): { lists: List[]; tasks: Task[]; tags: Tag[] } {
  const day = startOfDay(now)
  const at = (offset: number, h?: number, m = 0) => (h === undefined ? addDays(day, offset) : atTime(addDays(day, offset), h, m))

  const list = (name: string, color: string, order: number): List => ({
    id: newId(now), name, color, archived: false, viewMode: 'list', sortType: '', sortOrder: order, updatedMs: now, etag: '1',
  })
  const inbox: List = { id: INBOX_ID, name: 'Inbox', color: null, archived: false, viewMode: 'list', sortType: '', sortOrder: Number.MIN_SAFE_INTEGER / 2, updatedMs: now, etag: '1' }
  const work = list('Work', '#4772fa', 0)
  const personal = list('Personal', '#0cce9c', 1024)
  const shopping = list('Shopping', '#ed70a5', 2048)
  const learning = list('Learning', '#faa700', 3072)
  const lists = [inbox, work, personal, shopping, learning]

  let order = 0
  const task = (listId: string, title: string, o: Partial<Task> = {}): Task => ({
    id: newId(now), listId, parentId: null, title, notes: '', kind: 'text', status: 'open', priority: 0 as Priority,
    startMs: null, dueMs: null, isAllDay: false, timeZone: '', reminders: [], repeatFlag: '', exDates: [], items: [], tags: [],
    sortOrder: (order += 1024), createdMs: now - order, updatedMs: now, completedMs: null, deletedMs: null, etag: '1', ...o,
  })
  const item = (title: string, done = false, i = 0) => ({ id: newId(now), title, done, sortOrder: i })

  const tasks: Task[] = [
    task(inbox.id, 'Try quick add: "call mum tomorrow 5pm !high #family"'),
    task(inbox.id, 'Read the welcome notes', { notes: 'Press N to add a task, J/K to move, Space to complete, Ctrl+K to search.' }),
    task(inbox.id, 'Sort out the garage', { priority: 1 }),
    task(work.id, 'Submit quarterly report', { dueMs: at(-2), isAllDay: true, priority: 5, tags: ['q4'] }),
    task(work.id, 'Prepare slides for Friday review', { dueMs: at(0, 17), priority: 5, tags: ['q4'], items: [item('Outline', true, 0), item('Charts', false, 1), item('Speaker notes', false, 2)], kind: 'checklist' }),
    task(work.id, 'Reply to design feedback', { dueMs: at(0), isAllDay: true, priority: 3 }),
    task(work.id, 'One-on-one with Sam', { dueMs: at(1, 10), priority: 3, repeatFlag: 'RRULE:FREQ=WEEKLY;BYDAY=WE', reminders: ['TRIGGER:PT0S'] }),
    task(work.id, 'Book conference travel', { dueMs: at(4), isAllDay: true, tags: ['travel'] }),
    task(work.id, 'Refactor the importer', { dueMs: at(9), isAllDay: true, priority: 1 }),
    task(work.id, 'Update the on-call runbook', { priority: 1 }),
    task(work.id, 'Archive old tickets'),
    task(personal.id, 'Dentist appointment', { dueMs: at(2, 9, 30), priority: 3, reminders: ['TRIGGER:-PT1H'] }),
    task(personal.id, 'Pay electricity bill', { dueMs: at(-1), isAllDay: true, priority: 5, repeatFlag: 'RRULE:FREQ=MONTHLY;BYMONTHDAY=28' }),
    task(personal.id, 'Water the plants', { dueMs: at(0), isAllDay: true, repeatFlag: 'RRULE:FREQ=WEEKLY;BYDAY=TU,SA' }),
    task(personal.id, 'Call mum', { dueMs: at(1, 17), tags: ['family'] }),
    task(personal.id, "Plan Anna's birthday", { dueMs: at(12), isAllDay: true, tags: ['family'] }),
    task(personal.id, 'Renew passport', { dueMs: at(30), isAllDay: true, priority: 3, tags: ['travel'] }),
    task(personal.id, 'Go for a run', { dueMs: at(3, 7), tags: ['health'] }),
    task(shopping.id, 'Milk, eggs, bread', { dueMs: at(0), isAllDay: true, kind: 'checklist', items: [item('Milk', true, 0), item('Eggs', false, 1), item('Bread', false, 2)] }),
    task(shopping.id, 'New running shoes', { tags: ['health'] }),
    task(shopping.id, 'Birthday card'),
    task(shopping.id, 'Light bulbs'),
    task(learning.id, 'Finish the Rust chapter on lifetimes', { dueMs: at(5), isAllDay: true, priority: 3 }),
    task(learning.id, 'Watch the database internals talk', { dueMs: at(6, 20), tags: ['db'] }),
    task(learning.id, 'Write notes on mmap-backed stores', { priority: 1, tags: ['db'] }),
    task(learning.id, 'Practise 30 minutes of Spanish', { dueMs: at(0, 20), repeatFlag: 'RRULE:FREQ=DAILY' }),
    // Completed, so the Completed view has something in it.
    task(work.id, 'Send the invoice', { status: 'done', completedMs: at(-1, 15), dueMs: at(-1), isAllDay: true }),
    task(work.id, 'Fix the flaky test', { status: 'done', completedMs: at(-2, 11) }),
    task(personal.id, 'Book the car service', { status: 'done', completedMs: at(-3, 9) }),
    task(shopping.id, 'Buy printer paper', { status: 'done', completedMs: at(-6, 18) }),
  ]
  const tag = (label: string, color: string | null, o: number): Tag => ({ name: label.toLowerCase(), label, color, parent: null, sortOrder: o * 1024, etag: '1' })
  const tags = [tag('Q4', '#4772fa', 0), tag('Travel', '#0cce9c', 1), tag('Family', '#ed70a5', 2), tag('Health', '#faa700', 3), tag('DB', null, 4)]
  return { lists, tasks, tags }
}
