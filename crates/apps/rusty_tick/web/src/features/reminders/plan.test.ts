import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { triggerOffsetMs, upcoming } from './plan'

const H = 3_600_000
const task = (over: Partial<Task>): Task => ({ id: 't1', title: 'Pay rent', status: 'open', deletedMs: null, dueMs: 10 * H, reminders: ['TRIGGER:PT0S'], ...over }) as Task

describe('triggerOffsetMs', () => {
  it('reads the reminder options the date popover offers', () => {
    expect(triggerOffsetMs('TRIGGER:PT0S')).toBe(0)
    expect(triggerOffsetMs('TRIGGER:-PT5M')).toBe(-300_000)
    expect(triggerOffsetMs('TRIGGER:-PT1H')).toBe(-H)
    expect(triggerOffsetMs('TRIGGER:-P1D')).toBe(-24 * H)
    expect(triggerOffsetMs('TRIGGER:PT9H')).toBe(9 * H)
    expect(triggerOffsetMs('TRIGGER:-PT39H')).toBe(-39 * H)
  })
  it('refuses what it cannot read', () => {
    expect(triggerOffsetMs('')).toBeNull()
    expect(triggerOffsetMs('soon')).toBeNull()
  })
})

describe('upcoming', () => {
  it('lists reminders inside the horizon, soonest first', () => {
    const due = upcoming([task({ id: 'b', reminders: ['TRIGGER:PT0S'] }), task({ id: 'a', reminders: ['TRIGGER:-PT1H'] })], 0, 24 * H)
    expect(due.map((d) => [d.taskId, d.atMs])).toEqual([['a', 9 * H], ['b', 10 * H]])
  })
  it('leaves out the past, the far future, and anything not open', () => {
    expect(upcoming([task({})], 10 * H, 24 * H)).toEqual([]) // exactly now has already fired
    expect(upcoming([task({})], 0, 5 * H)).toEqual([])
    expect(upcoming([task({ status: 'done' }), task({ deletedMs: 1 }), task({ dueMs: null })], 0, 24 * H)).toEqual([])
  })
  it('skips reminders it cannot read but keeps the rest', () => {
    const due = upcoming([task({ reminders: ['nonsense', 'TRIGGER:PT0S'] })], 0, 24 * H)
    expect(due).toHaveLength(1)
  })
})
