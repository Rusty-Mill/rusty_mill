import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { mergeTargets } from './merge'

const task = (id: string, title: string, deletedMs: number | null = null): Task => ({ id, title, deletedMs }) as Task

describe('mergeTargets', () => {
  const tasks = [task('a', 'Standup'), task('b', 'standup '), task('c', 'Standup notes'), task('d', 'Standup', 1), task('e', 'Lunch')]

  it('offers the live tasks with the same title, not the duplicate itself', () => {
    expect(mergeTargets(tasks, tasks[0]!, '').map((t) => t.id)).toEqual(['b'])
  })

  it('searches every live task by title, same title first, up to the limit', () => {
    expect(mergeTargets(tasks, tasks[0]!, 'STAND').map((t) => t.id)).toEqual(['b', 'c'])
    expect(mergeTargets(tasks, tasks[4]!, 'stand').map((t) => t.id)).toEqual(['a', 'b', 'c'])
    expect(mergeTargets(tasks, tasks[4]!, 'stand', 2)).toHaveLength(2)
  })
})
