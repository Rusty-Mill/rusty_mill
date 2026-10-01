import { describe, expect, it } from 'vitest'
import { dayKey } from './date'
import { advanceDates, describeRule, formatRule, nextOccurrence, parseRule, laterOccurrences } from './recurrence'

const at = (y: number, m: number, d: number, h = 9) => new Date(y, m - 1, d, h).getTime()
const next = (rule: string, from: number) => {
  const n = nextOccurrence(rule, from)
  return n === null ? null : dayKey(n)
}

describe('parseRule', () => {
  it('reads the supported rules and rejects the rest', () => {
    expect(parseRule('RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=WE,MO')).toMatchObject({
      freq: 'WEEKLY',
      interval: 2,
      byDay: [1, 3],
    })
    expect(parseRule('FREQ=DAILY')?.interval).toBe(1)
    expect(parseRule('')).toBeNull()
    expect(parseRule('RRULE:FREQ=HOURLY')).toBeNull()
    expect(parseRule('RRULE:FREQ=DAILY;INTERVAL=0')).toBeNull()
    expect(parseRule('nonsense')).toBeNull()
  })

  it('round-trips through formatRule', () => {
    for (const text of ['RRULE:FREQ=DAILY', 'RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE', 'RRULE:FREQ=MONTHLY;BYMONTHDAY=15']) {
      expect(formatRule(parseRule(text)!)).toBe(text)
    }
  })
})

describe('nextOccurrence', () => {
  it('advances daily rules by their interval', () => {
    expect(next('RRULE:FREQ=DAILY', at(2026, 9, 29))).toBe('2026-09-30')
    expect(next('RRULE:FREQ=DAILY;INTERVAL=3', at(2026, 9, 29))).toBe('2026-10-02')
  })

  it('keeps the time of day', () => {
    const n = nextOccurrence('RRULE:FREQ=DAILY', at(2026, 9, 29, 17))!
    expect(new Date(n).getHours()).toBe(17)
  })

  it('weekly with days picks the next listed weekday, wrapping to the next interval', () => {
    // Tue 2026-09-29: the next of Mon/Wed is Wed the 30th; after Wed comes Mon the 5th.
    expect(next('RRULE:FREQ=WEEKLY;BYDAY=MO,WE', at(2026, 9, 29))).toBe('2026-09-30')
    expect(next('RRULE:FREQ=WEEKLY;BYDAY=MO,WE', at(2026, 9, 30))).toBe('2026-10-05')
    expect(next('RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO', at(2026, 9, 28))).toBe('2026-10-12')
  })

  it('weekly without days repeats on the same weekday', () => {
    expect(next('RRULE:FREQ=WEEKLY', at(2026, 9, 29))).toBe('2026-10-06')
  })

  it('monthly clamps to short months and honours BYMONTHDAY', () => {
    expect(next('RRULE:FREQ=MONTHLY', at(2026, 1, 31))).toBe('2026-02-28')
    expect(next('RRULE:FREQ=MONTHLY;BYMONTHDAY=31', at(2026, 1, 31))).toBe('2026-02-28')
    expect(next('RRULE:FREQ=MONTHLY;BYMONTHDAY=15', at(2026, 9, 15))).toBe('2026-10-15')
  })

  it('yearly handles Feb 29', () => {
    expect(next('RRULE:FREQ=YEARLY', at(2028, 2, 29))).toBe('2029-02-28')
  })

  it('stops after UNTIL and for non-repeating tasks', () => {
    expect(next('RRULE:FREQ=DAILY;UNTIL=20260930T000000Z', at(2026, 9, 29))).toBe('2026-09-30')
    expect(next('RRULE:FREQ=DAILY;UNTIL=20260930T000000Z', at(2026, 9, 30))).toBeNull()
    expect(next('', at(2026, 9, 29))).toBeNull()
  })
})

describe('describeRule', () => {
  it('reads like the repeat row', () => {
    expect(describeRule('')).toBe('Does not repeat')
    expect(describeRule('RRULE:FREQ=DAILY')).toBe('Every day')
    expect(describeRule('RRULE:FREQ=DAILY;INTERVAL=3')).toBe('Every 3 days')
    expect(describeRule('RRULE:FREQ=WEEKLY;BYDAY=MO,WE')).toBe('Every week on Mon, Wed')
    expect(describeRule('RRULE:FREQ=MONTHLY;BYMONTHDAY=1')).toBe('Every month on the 1st')
    expect(describeRule('RRULE:FREQ=MONTHLY;BYMONTHDAY=22')).toBe('Every month on the 22nd')
    expect(describeRule('RRULE:FREQ=MONTHLY;BYMONTHDAY=13')).toBe('Every month on the 13th')
  })
})

describe('advanceDates', () => {
  it('moves the start date along with the due date', () => {
    const r = advanceDates({ repeatFlag: 'RRULE:FREQ=DAILY', dueMs: at(2026, 9, 29, 17), startMs: at(2026, 9, 29, 16) })!
    expect(dayKey(r.dueMs)).toBe('2026-09-30')
    expect(new Date(r.startMs!).getHours()).toBe(16)
    expect(dayKey(r.startMs!)).toBe('2026-09-30')
  })

  it('skips excluded dates', () => {
    const r = advanceDates({ repeatFlag: 'RRULE:FREQ=DAILY', dueMs: at(2026, 9, 29), startMs: null }, [at(2026, 9, 30, 0)])!
    expect(dayKey(r.dueMs)).toBe('2026-10-01')
    expect(r.startMs).toBeNull()
  })

  it('has nothing to advance without a due date or rule', () => {
    expect(advanceDates({ repeatFlag: 'RRULE:FREQ=DAILY', dueMs: null, startMs: null })).toBeNull()
    expect(advanceDates({ repeatFlag: '', dueMs: at(2026, 9, 29), startMs: null })).toBeNull()
  })
})

describe('laterOccurrences', () => {
  const day = (d: number, h = 9) => new Date(2026, 8, d, h).getTime()
  it('lists the occurrences after the task itself, before the end', () => {
    const got = laterOccurrences({ repeatFlag: 'RRULE:FREQ=DAILY;INTERVAL=2', dueMs: day(1), startMs: null }, day(8))
    expect(got.map((o) => o.dueMs)).toEqual([day(3), day(5), day(7)])
  })
  it('moves a start date with the due date and honours excluded days and UNTIL', () => {
    const got = laterOccurrences({ repeatFlag: 'RRULE:FREQ=DAILY;UNTIL=20260905', dueMs: day(1), startMs: day(1, 8) }, day(30), [day(3)])
    expect(got.map((o) => [o.dueMs, o.startMs])).toEqual([[day(2), day(2, 8)], [day(4), day(4, 8)], [day(5), day(5, 8)]])
  })
  it('gives nothing for a one-off task, an undated task, or a rule it cannot read', () => {
    expect(laterOccurrences({ repeatFlag: '', dueMs: day(1), startMs: null }, day(30))).toEqual([])
    expect(laterOccurrences({ repeatFlag: 'RRULE:FREQ=DAILY', dueMs: null, startMs: null }, day(30))).toEqual([])
    expect(laterOccurrences({ repeatFlag: 'RRULE:FREQ=HOURLY', dueMs: day(1), startMs: null }, day(30))).toEqual([])
  })
})
