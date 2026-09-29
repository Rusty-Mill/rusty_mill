import { describe, expect, it } from 'vitest'
import type { List, Task } from '@/api/types'
import { highlight, searchLists, searchTasks, snippet } from './search'

let n = 0
const task = (o: Partial<Task>): Task => ({ id: `t${++n}`, title: `task ${n}`, notes: '', status: 'open', deletedMs: null, updatedMs: n, ...o }) as Task
const list = (id: string, name: string, sortOrder = 0): List => ({ id, name, sortOrder }) as List

describe('highlight', () => {
  it('splits around matches, ignoring case', () => {
    expect(highlight('Buy Milk and milk', 'milk')).toEqual([
      { text: 'Buy ', hit: false }, { text: 'Milk', hit: true }, { text: ' and ', hit: false }, { text: 'milk', hit: true },
    ])
  })
  it('returns the text whole when there is nothing to find', () => {
    expect(highlight('abc', '')).toEqual([{ text: 'abc', hit: false }])
    expect(highlight('abc', '  ')).toEqual([{ text: 'abc', hit: false }])
    expect(highlight('abc', 'zzz')).toEqual([{ text: 'abc', hit: false }])
  })
  it('does not treat the query as a pattern', () => {
    expect(highlight('a.b (c)', '.')).toEqual([{ text: 'a', hit: false }, { text: '.', hit: true }, { text: 'b (c)', hit: false }])
    expect(highlight('a(b', '(')).toEqual([{ text: 'a', hit: false }, { text: '(', hit: true }, { text: 'b', hit: false }])
  })
  it('handles a match at either end', () => {
    expect(highlight('abc', 'abc')).toEqual([{ text: 'abc', hit: true }])
    expect(highlight('abab', 'ab')).toEqual([{ text: 'ab', hit: true }, { text: 'ab', hit: true }])
  })
})

describe('snippet', () => {
  it('windows around the match with ellipses', () => {
    const text = `${'x'.repeat(100)} needle ${'y'.repeat(100)}`
    const s = snippet(text, 'needle', 10)
    expect(s.startsWith('…') && s.endsWith('…') && s.includes('needle')).toBe(true)
    expect(s.length).toBeLessThan(40)
  })
  it('collapses whitespace', () => expect(snippet('a\n\n  needle', 'needle')).toBe('a needle'))
})

describe('searchTasks', () => {
  it('matches substrings, so a prefix finds the word (the server search cannot)', () => {
    const t = task({ title: 'Buy groceries' })
    expect(searchTasks('grocer', [t, task({ title: 'Other' })]).map((h) => h.task.id)).toEqual([t.id])
  })
  it('needs every word, in any order, across title and notes', () => {
    const t = task({ title: 'Report', notes: 'quarterly numbers' })
    expect(searchTasks('quarterly report', [t])).toHaveLength(1)
    expect(searchTasks('quarterly nothing', [t])).toHaveLength(0)
  })
  it('ranks title matches, then open, then newest', () => {
    const notesOnly = task({ title: 'A', notes: 'milk', updatedMs: 999 })
    const doneTitle = task({ title: 'milk', status: 'done', updatedMs: 500 })
    const openOld = task({ title: 'milk run', updatedMs: 1 })
    const openNew = task({ title: 'milk shake', updatedMs: 50 })
    expect(searchTasks('milk', [notesOnly, doneTitle, openOld, openNew]).map((h) => h.task.id)).toEqual([openNew.id, openOld.id, doneTitle.id, notesOnly.id])
  })
  it('gives a notes snippet only when the title did not match', () => {
    const hits = searchTasks('milk', [task({ title: 'A', notes: 'remember the milk' }), task({ title: 'milk', notes: 'milk again' })])
    expect(hits.find((h) => h.task.title === 'A')?.snippet).toContain('milk')
    expect(hits.find((h) => h.task.title === 'milk')?.snippet).toBeNull()
  })
  it('never returns trashed tasks', () => {
    expect(searchTasks('x', [task({ title: 'x', deletedMs: 5 })])).toEqual([])
  })
  it('an empty query shows the most recently changed', () => {
    const tasks = Array.from({ length: 12 }, (_, i) => task({ updatedMs: 1000 + i }))
    const hits = searchTasks('', tasks)
    expect(hits).toHaveLength(8)
    expect(hits[0]!.task.updatedMs).toBe(1011)
  })
  it('caps the results', () => {
    const tasks = Array.from({ length: 80 }, () => task({ title: 'match' }))
    expect(searchTasks('match', tasks)).toHaveLength(50)
    expect(searchTasks('match', tasks, 5)).toHaveLength(5)
  })
})

describe('searchLists', () => {
  const lists = [list('a', 'Work', 2), list('b', 'Home Reno', 1), list('c', 'Workouts', 3)]
  it('matches names, in sidebar order', () => {
    expect(searchLists('work', lists).map((l) => l.id)).toEqual(['a', 'c'])
    expect(searchLists('', lists).map((l) => l.id)).toEqual(['b', 'a', 'c'])
    expect(searchLists('zzz', lists)).toEqual([])
  })
})
