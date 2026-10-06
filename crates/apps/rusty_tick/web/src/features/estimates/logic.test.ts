import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import type { FocusRecordBody } from '../focus/logic'
import { actualPomos, asEstimateBody, estimateRows } from './logic'

const rec = (o: Partial<FocusRecordBody>): FocusRecordBody => ({ startMs: 0, endMs: 1, durationSec: 1, kind: 'pomo', taskId: 'a', taskTitle: 'A', interruptions: 0, ...o })
const task = (id: string, title: string, deletedMs: number | null = null) => ({ id, title, deletedMs }) as Task

describe('actualPomos', () => {
  it('counts finished pomos for the task only', () => {
    const records = [rec({}), rec({}), rec({ kind: 'stopwatch' }), rec({ taskId: 'b' }), rec({ taskId: null })]
    expect(actualPomos(records, 'a')).toBe(2)
  })
})

describe('estimateRows', () => {
  it('skips missing and trashed tasks and lists over-runs first', () => {
    const tasks = { a: task('a', 'Alpha'), b: task('b', 'Beta'), c: task('c', 'Gone', 5) }
    const records = [rec({ taskId: 'b' }), rec({ taskId: 'b' })]
    const rows = estimateRows(
      [{ taskId: 'a', pomos: 3 }, { taskId: 'b', pomos: 1 }, { taskId: 'c', pomos: 2 }, { taskId: 'zz', pomos: 2 }],
      records,
      tasks,
    )
    expect(rows).toEqual([
      { taskId: 'b', title: 'Beta', estimated: 1, actual: 2 },
      { taskId: 'a', title: 'Alpha', estimated: 3, actual: 0 },
    ])
  })
})

describe('asEstimateBody', () => {
  it('keeps positive whole estimates, capped, and drops the rest', () => {
    expect(asEstimateBody({ taskId: 't', pomos: 2.4 })).toEqual({ taskId: 't', pomos: 2 })
    expect(asEstimateBody({ taskId: 't', pomos: 500 })).toEqual({ taskId: 't', pomos: 99 })
    expect(asEstimateBody({ taskId: 't', pomos: 0 })).toBeNull()
    expect(asEstimateBody({ pomos: 1 })).toBeNull()
  })
})
