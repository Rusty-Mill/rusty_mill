import { describe, expect, it } from 'vitest'
import { dayKey } from '@/lib/date'
import { isUuid } from '@/lib/id'
import {
  asHabitBody, checkinId, computeStreak, formatGoal, frequencyLabel, isDue, parseGoal, weekDays, weekdayOrder,
  type HabitBody,
} from './logic'

// Sep 29 2026 is a Tuesday.
const at = (m: number, d: number, y = 2026): number => new Date(y, m - 1, d, 12).getTime()
const keys = (...days: [number, number][]): Set<string> => new Set(days.map(([m, d]) => dayKey(at(m, d))))
const habit = (frequency: HabitBody['frequency'], createdMs = at(1, 1)): HabitBody => ({ name: 'Read', color: '#4772fa', goal: '1 time per day', frequency, reminder: null, createdMs, archived: false })
const daily = habit({ kind: 'daily' })

describe('checkinId', () => {
  it('is a stable UUID-shaped value', () => {
    const a = checkinId('0190a000-0000-7000-8000-000000000001', '2026-09-29')
    expect(a).toBe(checkinId('0190a000-0000-7000-8000-000000000001', '2026-09-29'))
    expect(isUuid(a)).toBe(true)
    expect(a[14]).toBe('5')
    expect('89ab').toContain(a[19]!)
  })
  it('differs by habit and by day, and ignores id case', () => {
    const id = '0190a000-0000-7000-8000-000000000001'
    const ids = new Set([checkinId(id, '2026-09-29'), checkinId(id, '2026-09-30'), checkinId('0190a000-0000-7000-8000-000000000002', '2026-09-29')])
    expect(ids.size).toBe(3)
    expect(checkinId(id.toUpperCase(), '2026-09-29')).toBe(checkinId(id, '2026-09-29'))
  })
  it('does not collide across a year of days', () => {
    const seen = new Set<string>()
    for (let i = 0; i < 365; i++) seen.add(checkinId('h', dayKey(at(1, 1) + i * 86_400_000)))
    expect(seen.size).toBe(365)
  })
})

describe('weeks', () => {
  it('returns the seven days of the week containing the date', () => {
    const mon = weekDays(at(9, 29), 1).map((t) => dayKey(t))
    expect(mon).toEqual(['2026-09-28', '2026-09-29', '2026-09-30', '2026-10-01', '2026-10-02', '2026-10-03', '2026-10-04'])
    expect(weekDays(at(9, 29), 0)[0]).toBe(new Date(2026, 8, 27).getTime())
    expect(dayKey(weekDays(at(9, 29), 6)[0]!)).toBe('2026-09-26')
  })
  it('is right across the autumn DST change (Nov 1 2026)', () => {
    expect(weekDays(at(11, 3), 0).map((t) => dayKey(t))).toEqual(['2026-11-01', '2026-11-02', '2026-11-03', '2026-11-04', '2026-11-05', '2026-11-06', '2026-11-07'])
  })
  it('orders weekday chips from the week start', () => {
    expect(weekdayOrder(1)).toEqual([1, 2, 3, 4, 5, 6, 0])
    expect(weekdayOrder(6)).toEqual([6, 0, 1, 2, 3, 4, 5])
  })
})

describe('isDue', () => {
  it('daily and per-week habits are due every day', () => {
    expect(isDue(daily, at(9, 29))).toBe(true)
    expect(isDue(habit({ kind: 'perWeek', times: 3 }), at(9, 27))).toBe(true)
  })
  it('weekday habits are due on their days only', () => {
    const h = habit({ kind: 'weekdays', days: [1, 3] })
    expect(isDue(h, at(9, 28))).toBe(true) // Monday
    expect(isDue(h, at(9, 29))).toBe(false) // Tuesday
    expect(isDue(h, at(9, 30))).toBe(true) // Wednesday
  })
})

describe('computeStreak', () => {
  const today = at(9, 29)
  it('is zero with no check-ins', () => {
    expect(computeStreak(daily, new Set(), today, 1)).toEqual({ current: 0, best: 0, unit: 'day' })
  })
  it('counts consecutive days including today', () => {
    expect(computeStreak(daily, keys([9, 27], [9, 28], [9, 29]), today, 1)).toEqual({ current: 3, best: 3, unit: 'day' })
  })
  it('today not yet done does not break the streak', () => {
    expect(computeStreak(daily, keys([9, 26], [9, 27], [9, 28]), today, 1).current).toBe(3)
  })
  it('a missed earlier day breaks it, and best remembers the longer run', () => {
    const s = computeStreak(daily, keys([9, 10], [9, 11], [9, 12], [9, 13], [9, 27], [9, 29]), today, 1)
    expect(s.current).toBe(1)
    expect(s.best).toBe(4)
  })
  it('yesterday missed with today done starts again at one', () => {
    expect(computeStreak(daily, keys([9, 25], [9, 26], [9, 29]), today, 1).current).toBe(1)
  })
  it('weekday habits skip days that are not due', () => {
    const h = habit({ kind: 'weekdays', days: [1, 3, 5] }) // Mon Wed Fri
    // Fri 25, Mon 28 done; Sat/Sun/Tue not due; Wed 30 not yet.
    expect(computeStreak(h, keys([9, 23], [9, 25], [9, 28]), at(9, 29), 1).current).toBe(3)
    // Missing Monday breaks it.
    expect(computeStreak(h, keys([9, 23], [9, 25]), at(9, 29), 1).current).toBe(0)
  })
  it('a check-in on a day that is not due still counts', () => {
    const h = habit({ kind: 'weekdays', days: [1, 3] })
    expect(computeStreak(h, keys([9, 28], [9, 29]), today, 1).current).toBe(2)
  })
  it('per-week habits count weeks that reached the target', () => {
    const h = habit({ kind: 'perWeek', times: 2 })
    // Weeks (Mon start): Sep 14, Sep 21 reached; current week Sep 28 has one so far.
    const done = keys([9, 14], [9, 16], [9, 21], [9, 24], [9, 28])
    expect(computeStreak(h, done, today, 1)).toEqual({ current: 2, best: 2, unit: 'week' })
    const now = computeStreak(h, keys([9, 14], [9, 16], [9, 21], [9, 24], [9, 28], [9, 29]), today, 1)
    expect(now.current).toBe(3)
  })
  it('per-week: a short week in the past resets, best is kept', () => {
    const h = habit({ kind: 'perWeek', times: 2 })
    const done = keys([8, 3], [8, 4], [8, 10], [8, 11], [8, 17], [8, 18], [8, 31], [9, 1])
    const s = computeStreak(h, done, today, 1)
    expect(s.best).toBe(3)
    expect(s.current).toBe(0) // Sep 7-21 had none, so the week of Aug 31 no longer counts
  })
  it('ignores check-ins in the future', () => {
    expect(computeStreak(daily, keys([9, 30]), today, 1).current).toBe(0)
  })
})

describe('goal and frequency text', () => {
  it('round-trips a goal', () => {
    expect(formatGoal(8, 'glasses')).toBe('8 glasses per day')
    expect(parseGoal('8 glasses per day')).toEqual({ count: 8, unit: 'glasses' })
    expect(parseGoal('whatever')).toEqual({ count: 1, unit: 'time' })
    expect(formatGoal(1, '  ')).toBe('1 time per day')
  })
  it('labels frequencies', () => {
    expect(frequencyLabel({ kind: 'daily' })).toBe('Daily')
    expect(frequencyLabel({ kind: 'perWeek', times: 1 })).toBe('1 time per week')
    expect(frequencyLabel({ kind: 'weekdays', days: [3, 1] })).toBe('Mon, Wed')
  })
  it('sanitizes stored bodies', () => {
    expect(asHabitBody(null)).toBeNull()
    expect(asHabitBody({ name: 'x', frequency: { kind: 'perWeek', times: 99 } })?.frequency).toEqual({ kind: 'perWeek', times: 7 })
    expect(asHabitBody({ name: 'x', frequency: { kind: 'weekdays', days: [1, 9, 'a'] } })?.frequency).toEqual({ kind: 'weekdays', days: [1] })
  })
})
