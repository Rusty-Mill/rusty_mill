import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import type { Checkin, Habit } from '@/features/habits/logic'
import { checkinId } from '@/features/habits/logic'
import { triggerOffsetMs, upcoming, upcomingHabits } from './plan'

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
    const due = upcoming([task({ title: 'b', reminders: ['TRIGGER:PT0S'] }), task({ title: 'a', reminders: ['TRIGGER:-PT1H'] })], 0, 24 * H)
    expect(due.map((d) => [d.title, d.atMs])).toEqual([['a', 9 * H], ['b', 10 * H]])
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

describe('upcomingHabits', () => {
  const day = (d: number, h = 0, m = 0) => new Date(2026, 8, d, h, m).getTime()
  const habit = (over: Partial<Habit> = {}): Habit => ({ id: 'h1', name: 'Read', color: '#000', goal: '', frequency: { kind: 'daily' }, reminder: '20:00', createdMs: 0, archived: false, ...over })

  it('lists the reminder time on each day inside the horizon, soonest first', () => {
    const got = upcomingHabits([habit()], {}, day(29, 9), 48 * H)
    expect(got.map((d) => d.atMs)).toEqual([day(29, 20), day(30, 20)])
    expect(got[0]).toMatchObject({ title: 'Read', body: 'Habit reminder' })
  })
  it('skips a time already passed today, and a day already checked in', () => {
    const done: Record<string, Checkin> = { [checkinId('h1', '2026-09-29')]: { id: 'x', habitId: 'h1', day: '2026-09-29', count: 1 } }
    expect(upcomingHabits([habit()], {}, day(29, 21), 24 * H).map((d) => d.atMs)).toEqual([day(30, 20)])
    expect(upcomingHabits([habit()], done, day(29, 9), 48 * H).map((d) => d.atMs)).toEqual([day(30, 20)])
  })
  it('keeps reminding until the goal is met, not just started', () => {
    const goal = { goal: '3 glasses per day' }
    const at = (count: number): Record<string, Checkin> => ({ [checkinId('h1', '2026-09-29')]: { id: 'x', habitId: 'h1', day: '2026-09-29', count } })
    expect(upcomingHabits([habit(goal)], at(2), day(29, 9), 24 * H).map((d) => d.atMs)).toEqual([day(29, 20)])
    expect(upcomingHabits([habit(goal)], at(3), day(29, 9), 24 * H)).toEqual([])
  })
  it('skips habits with no reminder, archived ones, and days a weekday habit is not due', () => {
    const wed = day(30).valueOf() // 2026-09-30 is a Wednesday
    expect(upcomingHabits([habit({ reminder: null }), habit({ archived: true })], {}, day(29, 9), 48 * H)).toEqual([])
    expect(upcomingHabits([habit({ frequency: { kind: 'weekdays', days: [new Date(wed).getDay()] } })], {}, day(29, 9), 48 * H).map((d) => d.atMs)).toEqual([day(30, 20)])
  })
})
