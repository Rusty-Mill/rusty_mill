import { describe, expect, it } from 'vitest'
import { addDays, atTime, dayKey, startOfDay } from '@/lib/date'
import { CLEARED, presetOf, quickDates, repeatRule, selectionOf, toFields } from './dateSelection'

const at = (y: number, m: number, d: number, h = 0, min = 0) => new Date(y, m - 1, d, h, min).getTime()
const NOW = at(2026, 9, 29, 12) // Tuesday

describe('quickDates', () => {
  it('today, tomorrow, next week (from the week start), and this evening', () => {
    const [today, tomorrow, nextWeek, evening] = quickDates(NOW, 1)
    expect(dayKey(today!.day)).toBe('2026-09-29')
    expect(dayKey(tomorrow!.day)).toBe('2026-09-30')
    expect(dayKey(nextWeek!.day)).toBe('2026-10-05') // the Monday after this Monday-start week
    expect(evening).toMatchObject({ time: { h: 18, m: 0 } })
    expect(dayKey(evening!.day)).toBe('2026-09-29')
  })

  it('next week follows the week-start preference', () => {
    expect(dayKey(quickDates(NOW, 0)[2]!.day)).toBe('2026-10-04') // Sunday start
  })

  it('"later this evening" is an hour from now once 18:00 has passed', () => {
    const late = at(2026, 9, 29, 19, 10)
    const evening = quickDates(late, 1)[3]!
    expect(evening.time).toEqual({ h: 20, m: 0 })
    expect(dayKey(evening.day)).toBe('2026-09-29')
  })

  it('rolls to tomorrow when an hour from now is past midnight', () => {
    const evening = quickDates(at(2026, 9, 29, 23, 30), 1)[3]!
    expect(dayKey(evening.day)).toBe('2026-09-30')
    expect(evening.time?.h).toBe(0)
  })
})

describe('toFields', () => {
  const base = { day: startOfDay(NOW), time: null, startDay: null, startTime: null, reminder: '', repeat: '' }

  it('an all-day due date sits at local midnight', () => {
    expect(toFields(base)).toEqual({ dueMs: startOfDay(NOW), startMs: null, isAllDay: true, reminders: [], repeatFlag: '' })
  })

  it('a time makes it a timed task', () => {
    const f = toFields({ ...base, time: { h: 17, m: 30 }, reminder: 'TRIGGER:PT0S', repeat: 'RRULE:FREQ=DAILY' })
    expect(f).toEqual({ dueMs: atTime(NOW, 17, 30), startMs: null, isAllDay: false, reminders: ['TRIGGER:PT0S'], repeatFlag: 'RRULE:FREQ=DAILY' })
  })

  it('no day clears everything, including a time, reminder or repeat', () => {
    expect(toFields({ ...base, day: null, time: { h: 9, m: 0 }, reminder: 'TRIGGER:PT0S', repeat: 'RRULE:FREQ=DAILY' })).toEqual(CLEARED)
  })

  it('a duration keeps its start, and never starts after it is due', () => {
    const f = toFields({ ...base, day: addDays(startOfDay(NOW), 3), startDay: startOfDay(NOW) })
    expect(f.startMs).toBe(startOfDay(NOW))
    const inverted = toFields({ ...base, startDay: addDays(startOfDay(NOW), 9) })
    expect(inverted.startMs).toBe(inverted.dueMs)
  })
})

describe('selectionOf', () => {
  it('reads a timed task back', () => {
    const sel = selectionOf({ dueMs: atTime(NOW, 17, 30), startMs: null, isAllDay: false, reminders: ['TRIGGER:-PT5M'], repeatFlag: 'RRULE:FREQ=WEEKLY' })
    expect(sel).toMatchObject({ day: startOfDay(NOW), time: { h: 17, m: 30 }, reminder: 'TRIGGER:-PT5M', repeat: 'RRULE:FREQ=WEEKLY' })
  })
  it('reads an all-day task and an undated one', () => {
    expect(selectionOf({ dueMs: startOfDay(NOW), startMs: null, isAllDay: true, reminders: [], repeatFlag: '' }).time).toBeNull()
    expect(selectionOf({ dueMs: null, startMs: null, isAllDay: false, reminders: [], repeatFlag: '' }).day).toBeNull()
  })
  it('round-trips through toFields', () => {
    const f = { dueMs: atTime(NOW, 8, 15), startMs: atTime(NOW, 7, 0), isAllDay: false, reminders: ['TRIGGER:PT0S'], repeatFlag: 'RRULE:FREQ=DAILY' }
    expect(toFields(selectionOf(f))).toEqual(f)
  })
})

describe('repeat presets', () => {
  const due = at(2026, 9, 29, 9) // a Tuesday, the 29th
  it('anchors weekly on the weekday and monthly on the date', () => {
    expect(repeatRule('weekly', due)).toBe('RRULE:FREQ=WEEKLY;BYDAY=TU')
    expect(repeatRule('monthly', due)).toBe('RRULE:FREQ=MONTHLY;BYMONTHDAY=29')
    expect(repeatRule('daily', due)).toBe('RRULE:FREQ=DAILY')
    expect(repeatRule('yearly', due)).toBe('RRULE:FREQ=YEARLY')
    expect(repeatRule('none', due)).toBe('')
  })
  it('builds a custom interval', () => {
    expect(repeatRule('custom', due, { freq: 'DAILY', interval: 3 })).toBe('RRULE:FREQ=DAILY;INTERVAL=3')
    expect(repeatRule('custom', due, { freq: 'WEEKLY', interval: 2 })).toBe('RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=TU')
    expect(repeatRule('custom', due, { freq: 'DAILY', interval: 0 })).toBe('RRULE:FREQ=DAILY')
  })
  it('recognises a stored rule', () => {
    expect(presetOf('')).toBe('none')
    expect(presetOf('RRULE:FREQ=DAILY')).toBe('daily')
    expect(presetOf('RRULE:FREQ=WEEKLY;BYDAY=TU')).toBe('weekly')
    expect(presetOf('RRULE:FREQ=WEEKLY;BYDAY=MO,WE')).toBe('custom')
    expect(presetOf('RRULE:FREQ=DAILY;INTERVAL=3')).toBe('custom')
    expect(presetOf('RRULE:FREQ=MONTHLY;BYMONTHDAY=29')).toBe('monthly')
    expect(presetOf('garbage')).toBe('none')
  })
})
