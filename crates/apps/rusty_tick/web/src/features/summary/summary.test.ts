import { describe, expect, it } from 'vitest'
import { defaultOptions, type SummaryOptions } from './options'
import { buildSummary, selectTasks, type SummaryContext } from './summary'
import { NOW, at, done, list, task } from './testkit'
import { computeRange } from './range'

const lists = [list('inbox', 'Inbox', -1), list('work', 'Work', 0), list('home', 'Home', 1)]
const opts = (over: Partial<SummaryOptions> = {}): SummaryOptions => ({ ...defaultOptions(NOW), template: 'simple', range: 'today', ...over })
const cx = (tasks: ReturnType<typeof task>[], over: Partial<SummaryContext> = {}): SummaryContext => ({ tasks, lists, now: NOW, weekStart: 1, hour12: false, ...over })
const titles = (doc: ReturnType<typeof buildSummary>): string[] => doc.sections.flatMap((s) => s.groups.flatMap((g) => g.items.map((i) => i.title)))
const today = computeRange('today', NOW, 1)!

describe('selectTasks', () => {
  it('places finished tasks by completion time and open ones by due time', () => {
    const a = done(at(2026, 9, 29, 9), { title: 'a', dueMs: at(2026, 9, 20) }) // due long ago, finished today
    const b = task({ title: 'b', dueMs: at(2026, 9, 29, 18) })
    const c = task({ title: 'c', dueMs: at(2026, 9, 30) }) // tomorrow
    const d = task({ title: 'd' }) // no date
    const r = selectTasks([a, b, c, d], opts(), today)
    expect(r.completed.map((t) => t.title)).toEqual(['a'])
    expect(r.open.map((t) => t.title)).toEqual(['b'])
  })

  it('splits at midnight: 23:59:59.999 is yesterday, 00:00:00 is today', () => {
    const before = done(at(2026, 9, 28, 23, 59, 59, 999), { title: 'before' })
    const after = done(at(2026, 9, 29, 0, 0, 0, 0), { title: 'after' })
    const next = done(at(2026, 9, 30, 0, 0, 0, 0), { title: 'next' })
    expect(selectTasks([before, after, next], opts(), today).completed.map((t) => t.title)).toEqual(['after'])
  })

  it('skips trashed tasks', () => {
    expect(selectTasks([done(at(2026, 9, 29, 9), { deletedMs: 1 })], opts(), today).completed).toEqual([])
  })

  it('filters by status', () => {
    const t = [done(at(2026, 9, 29, 9)), task({ dueMs: at(2026, 9, 29, 10) })]
    expect(selectTasks(t, opts({ status: 'completed' }), today).open).toEqual([])
    expect(selectTasks(t, opts({ status: 'open' }), today).completed).toEqual([])
    const both = selectTasks(t, opts({ status: 'all' }), today)
    expect([both.completed.length, both.open.length]).toEqual([1, 1])
  })

  it('filters by list, priority and tag (empty selection means everything)', () => {
    const d = at(2026, 9, 29, 10)
    const t = [
      task({ title: 'w-high', listId: 'work', priority: 5, dueMs: d, tags: ['q4'] }),
      task({ title: 'h-none', listId: 'home', priority: 0, dueMs: d, tags: ['x'] }),
      task({ title: 'h-low', listId: 'home', priority: 1, dueMs: d }),
    ]
    const names = (o: Partial<SummaryOptions>): string[] => selectTasks(t, opts(o), today).open.map((x) => x.title)
    expect(names({})).toHaveLength(3)
    expect(names({ listIds: ['home'] })).toEqual(['h-low', 'h-none'])
    expect(names({ priorities: [5, 1] })).toEqual(['h-low', 'w-high'])
    expect(names({ priorities: [0] })).toEqual(['h-none'])
    expect(names({ tags: ['q4', 'x'] })).toEqual(['h-none', 'w-high'])
    expect(names({ listIds: ['home'], tags: ['q4'] })).toEqual([])
  })

  it('orders by time, then title', () => {
    const d = at(2026, 9, 29, 10)
    const r = selectTasks([task({ title: 'b', dueMs: d }), task({ title: 'a', dueMs: d }), task({ title: 'z', dueMs: at(2026, 9, 29, 8) })], opts(), today)
    expect(r.open.map((t) => t.title)).toEqual(['z', 'a', 'b'])
  })
})

describe('buildSummary', () => {
  const d = at(2026, 9, 29, 17, 30)

  it('is empty, with a subtitle, when nothing matches', () => {
    const doc = buildSummary(opts(), cx([]))
    expect(doc).toMatchObject({ title: 'Simple list', subtitle: 'Sep 29', sections: [], total: 0 })
  })

  it('reports a bad custom range instead of throwing', () => {
    const doc = buildSummary(opts({ range: 'custom', custom: { from: 'x', to: 'y' } }), cx([task()]))
    expect(doc.total).toBe(0)
    expect(doc.subtitle).toMatch(/valid date range/)
  })

  it('daily and weekly reports have Completed and Not completed sections with counts', () => {
    const t = [done(at(2026, 9, 29, 9), { title: 'shipped' }), task({ title: 'todo', dueMs: d })]
    const doc = buildSummary(opts({ template: 'daily' }), cx(t))
    expect(doc.title).toBe('Daily report')
    expect(doc.sections.map((s) => s.heading)).toEqual(['Completed (1)', 'Not completed (1)'])
    expect(doc.total).toBe(2)
  })

  it('omits a section that has no tasks', () => {
    const doc = buildSummary(opts({ template: 'daily' }), cx([done(at(2026, 9, 29, 9))]))
    expect(doc.sections.map((s) => s.heading)).toEqual(['Completed (1)'])
  })

  it('the simple template is one flat list with done tasks first', () => {
    const t = [task({ title: 'todo', dueMs: d }), done(at(2026, 9, 29, 9), { title: 'shipped' })]
    const doc = buildSummary(opts(), cx(t))
    expect(doc.sections).toHaveLength(1)
    expect(doc.sections[0]!.heading).toBeNull()
    expect(titles(doc)).toEqual(['shipped', 'todo'])
    expect(doc.sections[0]!.groups[0]!.items.map((i) => i.done)).toEqual([true, false])
  })

  it('the weekly report groups by day', () => {
    const t = [done(at(2026, 9, 28, 9), { title: 'mon' }), done(at(2026, 9, 29, 9), { title: 'tue1' }), done(at(2026, 9, 29, 11), { title: 'tue2' })]
    const doc = buildSummary(opts({ template: 'weekly', range: 'thisWeek' }), cx(t))
    expect(doc.subtitle).toBe('Sep 28 – Oct 4')
    expect(doc.sections[0]!.groups.map((g) => [g.heading, g.items.length])).toEqual([['Mon, Sep 28', 1], ['Tue, Sep 29', 2]])
  })

  it('the week range follows the week start preference', () => {
    const sunday = done(at(2026, 9, 27, 12), { title: 'sun' })
    expect(titles(buildSummary(opts({ range: 'thisWeek' }), cx([sunday], { weekStart: 1 })))).toEqual([]) // belongs to last week
    expect(titles(buildSummary(opts({ range: 'thisWeek' }), cx([sunday], { weekStart: 0 })))).toEqual(['sun'])
  })

  it('groups by list in list order and drops the list name from each item', () => {
    const t = [
      done(at(2026, 9, 29, 9), { title: 'h', listId: 'home' }),
      done(at(2026, 9, 29, 10), { title: 'w', listId: 'work' }),
      done(at(2026, 9, 29, 11), { title: 'i', listId: 'inbox' }),
    ]
    const doc = buildSummary(opts({ groupBy: 'list', showList: true }), cx(t))
    const groups = doc.sections[0]!.groups
    expect(groups.map((g) => g.heading)).toEqual(['Inbox', 'Work', 'Home'])
    expect(groups.every((g) => g.items.every((i) => i.meta.length === 0))).toBe(true)
  })

  it('shows only the facts asked for, in a fixed order', () => {
    const t = task({ title: 'x', listId: 'home', dueMs: d, priority: 5, tags: ['a', 'b'] })
    const meta = (o: Partial<SummaryOptions>): string[] => buildSummary(opts(o), cx([t])).sections[0]!.groups[0]!.items[0]!.meta
    expect(meta({ showList: false, showDue: false })).toEqual([])
    expect(meta({ showList: true, showDue: true, showPriority: true, showTags: true })).toEqual(['Home', 'Due Sep 29 17:30', 'High priority', '#a #b'])
    expect(buildSummary(opts({ showList: false, showDue: true }), { ...cx([t]), hour12: true }).sections[0]!.groups[0]!.items[0]!.meta).toEqual(['Due Sep 29 5:30 PM'])
  })

  it('leaves the time off an all-day due date and shows completion time when asked', () => {
    const t = done(at(2026, 9, 29, 9, 5), { title: 'x', dueMs: at(2026, 9, 29), isAllDay: true })
    const item = buildSummary(opts({ showList: false, showCompletedTime: true }), cx([t])).sections[0]!.groups[0]!.items[0]!
    expect(item.meta).toEqual(['Due Sep 29', 'Done Sep 29 09:05'])
  })

  it('names an unknown list Inbox rather than crashing', () => {
    const doc = buildSummary(opts(), cx([done(at(2026, 9, 29, 9), { listId: 'gone' })]))
    expect(doc.sections[0]!.groups[0]!.items[0]!.meta).toEqual(['Inbox'])
  })
})
