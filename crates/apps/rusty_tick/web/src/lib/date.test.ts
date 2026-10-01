import { describe, expect, it } from 'vitest'
import {
  addDays,
  addMonths,
  atTime,
  dayKey,
  diffDays,
  formatDue,
  formatDueLong,
  monthGrid,
  parseDayKey,
  startOfDay,
  startOfWeek,
} from './date'

const at = (y: number, m: number, d: number, h = 0, min = 0) => new Date(y, m - 1, d, h, min).getTime()
// Tue 2026-09-29 12:00 local
const NOW = at(2026, 9, 29, 12)

describe('day arithmetic', () => {
  it('addDays keeps the wall clock across a DST change', () => {
    // US DST ended 2026-11-01: that day is 25 hours long.
    const before = at(2026, 10, 31, 9)
    const after = addDays(before, 1)
    expect(new Date(after).getHours()).toBe(9)
    expect(after - before).toBe(25 * 3_600_000)
  })

  it('diffDays counts calendar days, not 24h blocks', () => {
    expect(diffDays(at(2026, 10, 31, 23), at(2026, 11, 2, 1))).toBe(2)
    expect(diffDays(NOW, NOW)).toBe(0)
    expect(diffDays(at(2026, 9, 30), at(2026, 9, 29))).toBe(-1)
  })

  it('addMonths clamps to the end of a short month', () => {
    expect(dayKey(addMonths(at(2026, 1, 31), 1))).toBe('2026-02-28')
    expect(dayKey(addMonths(at(2028, 1, 31), 1))).toBe('2028-02-29')
    expect(dayKey(addMonths(at(2026, 12, 15), 1))).toBe('2027-01-15')
    expect(dayKey(addMonths(at(2026, 3, 15), -3))).toBe('2025-12-15')
  })

  it('startOfWeek honours the week-start preference', () => {
    expect(dayKey(startOfWeek(NOW, 1))).toBe('2026-09-28') // Monday
    expect(dayKey(startOfWeek(NOW, 0))).toBe('2026-09-27') // Sunday
    expect(dayKey(startOfWeek(at(2026, 9, 27), 1))).toBe('2026-09-21') // a Sunday belongs to the previous Monday week
  })

  it('monthGrid is six weeks that contain the whole month', () => {
    const grid = monthGrid(NOW, 1)
    expect(grid).toHaveLength(42)
    expect(dayKey(grid[0]!)).toBe('2026-08-31') // Sep 1 2026 is a Tuesday; the Monday-start grid begins the day before
    expect(grid).toContain(startOfDay(at(2026, 9, 30)))
    expect(new Date(grid[0]!).getDay()).toBe(1)
  })
})

describe('day keys', () => {
  it('round-trips and rejects impossible dates', () => {
    expect(parseDayKey('2026-09-30')).toBe(at(2026, 9, 30))
    expect(dayKey(at(2026, 1, 5))).toBe('2026-01-05')
    expect(parseDayKey('2026-02-30')).toBeNull()
    expect(parseDayKey('nope')).toBeNull()
  })
})

describe('formatDue', () => {
  const timed = (ms: number) => ({ dueMs: ms, isAllDay: false })
  const allDay = (ms: number) => ({ dueMs: startOfDay(ms), isAllDay: true })

  it('has nothing to say without a date', () => {
    expect(formatDue({ dueMs: null, isAllDay: false }, NOW)).toBeNull()
  })

  it('names near days and turns a passed time red', () => {
    expect(formatDue(allDay(NOW), NOW)).toEqual({ text: 'Today', tone: 'today' })
    expect(formatDue(allDay(addDays(NOW, 1)), NOW)).toEqual({ text: 'Tomorrow', tone: 'future' })
    expect(formatDue(allDay(addDays(NOW, -1)), NOW)).toEqual({ text: 'Yesterday', tone: 'overdue' })
    expect(formatDue(timed(atTime(NOW, 9)), NOW)).toEqual({ text: 'Today 09:00', tone: 'overdue' })
    expect(formatDue(timed(atTime(NOW, 17)), NOW)).toEqual({ text: 'Today 17:00', tone: 'today' })
  })

  it('an all-day task is not overdue until its day is over', () => {
    expect(formatDue(allDay(NOW), NOW)?.tone).toBe('today')
    expect(formatDue(allDay(addDays(NOW, -2)), NOW)?.tone).toBe('overdue')
  })

  it('uses a weekday inside the week and a date beyond it', () => {
    expect(formatDue(allDay(addDays(NOW, 3)), NOW)?.text).toBe('Fri')
    expect(formatDue(allDay(addDays(NOW, 10)), NOW)?.text).toBe('Oct 9')
    expect(formatDue(allDay(at(2027, 1, 15)), NOW)?.text).toBe('Jan 15, 2027')
  })

  it('formats 12-hour times when asked', () => {
    expect(formatDue(timed(atTime(NOW, 17, 30)), NOW, true)?.text).toBe('Today 5:30 PM')
  })
})

describe('formatDueLong', () => {
  it('matches the detail chip', () => {
    expect(formatDueLong({ dueMs: atTime(addDays(NOW, 1), 17), isAllDay: false }, NOW)).toBe('Tomorrow, Sep 30, 17:00')
    expect(formatDueLong({ dueMs: startOfDay(addDays(NOW, 5)), isAllDay: true }, NOW)).toBe('Oct 4')
    expect(formatDueLong({ dueMs: null, isAllDay: false }, NOW)).toBeNull()
  })
})
