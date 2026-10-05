import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import type { Group } from './organize'
import { buildRows, HEADER_HEIGHT, ROW_HEIGHT, viewTitle, windowRows, type Row } from './rows'

const task = (id: string): Task => ({ id, title: id }) as Task
const group = (key: string, label: string, ids: string[], tone?: 'overdue'): Group => ({ key, label, tasks: ids.map(task), ...(tone ? { tone } : {}) })

describe('buildRows', () => {
  it('puts a header before each labelled group', () => {
    const rows = buildRows([group('today', 'Today', ['a', 'b']), group('later', 'Later', ['c'])], {}, 'all')
    expect(rows.map((r) => (r.type === 'header' ? `# ${r.label} ${r.count}` : r.task.id))).toEqual(['# Today 2', 'a', 'b', '# Later 1', 'c'])
  })

  it('draws no header for the single unlabelled group', () => {
    expect(buildRows([group('all', '', ['a', 'b'])], {}, 'inbox')).toHaveLength(2)
  })

  it('hides the tasks of a collapsed group but keeps its header and count', () => {
    const rows = buildRows([group('today', 'Today', ['a', 'b'])], { 'all:today': true }, 'all')
    expect(rows).toEqual([{ type: 'header', key: 'today', label: 'Today', count: 2, collapsed: true }])
  })

  it('collapse state is per view', () => {
    const rows = buildRows([group('today', 'Today', ['a'])], { 'inbox:today': true }, 'all')
    expect(rows).toHaveLength(2)
  })

  it('carries the overdue tone', () => {
    const [h] = buildRows([group('overdue', 'Overdue', ['a'], 'overdue')], {}, 'all')
    expect(h).toMatchObject({ tone: 'overdue' })
  })
})

describe('windowRows', () => {
  const many = (n: number): Row[] => Array.from({ length: n }, (_, i) => ({ type: 'task', task: task(`t${i}`), groupKey: 'g' }))

  it('draws the top of a long list, with overscan', () => {
    const w = windowRows(many(1000), 0, 400, 5)
    expect(w.start).toBe(0)
    expect(w.end).toBe(Math.ceil(400 / ROW_HEIGHT) + 1 + 5) // 10 visible + the one straddling the edge + overscan
    expect(w.offset).toBe(0)
    expect(w.total).toBe(1000 * ROW_HEIGHT)
  })

  it('follows the scroll position', () => {
    const w = windowRows(many(1000), 4000, 400, 5)
    expect(w.start).toBe(100 - 5)
    expect(w.offset).toBe(w.start * ROW_HEIGHT)
    expect(w.end).toBeGreaterThan(110)
  })

  it('never runs past the end', () => {
    const w = windowRows(many(30), 100_000, 400, 5)
    expect(w.end).toBe(30)
    expect(w.start).toBeLessThan(30)
  })

  it('accounts for the shorter header rows', () => {
    const rows: Row[] = [{ type: 'header', key: 'a', label: 'A', count: 1, collapsed: false }, ...many(5)]
    expect(windowRows(rows, 0, 1000, 0).total).toBe(HEADER_HEIGHT + 5 * ROW_HEIGHT)
    expect(windowRows(rows, HEADER_HEIGHT + ROW_HEIGHT, 40, 0).start).toBe(2)
  })

  it('handles an empty list', () => {
    expect(windowRows([], 0, 400)).toEqual({ start: 0, end: 0, offset: 0, total: 0 })
  })
})

describe('viewTitle', () => {
  const lists = [{ id: 'i', name: 'Inbox' }, { id: 'w', name: 'Work' }] as never
  const tags = [{ name: 'q4', label: 'Q4' }] as never
  const filters = [{ id: 'f', name: 'Soon', rule: { lists: [], tags: [], priorities: [], dates: [], assignees: [] } }]
  it('names each kind of view', () => {
    expect(viewTitle({ kind: 'all' }, lists, tags, filters, 'i')).toBe('All')
    expect(viewTitle({ kind: 'week' }, lists, tags, filters, 'i')).toBe('Next 7 Days')
    expect(viewTitle({ kind: 'inbox' }, lists, tags, filters, 'i')).toBe('Inbox')
    expect(viewTitle({ kind: 'list', id: 'w' }, lists, tags, filters, 'i')).toBe('Work')
    expect(viewTitle({ kind: 'tag', name: 'q4' }, lists, tags, filters, 'i')).toBe('Q4')
    expect(viewTitle({ kind: 'filter', id: 'f' }, lists, tags, filters, 'i')).toBe('Soon')
    expect(viewTitle({ kind: 'filter', id: 'x' }, lists, tags, filters, 'i')).toBe('Filter')
    expect(viewTitle({ kind: 'tag', name: 'gone' }, lists, tags, filters, 'i')).toBe('gone')
  })
})
