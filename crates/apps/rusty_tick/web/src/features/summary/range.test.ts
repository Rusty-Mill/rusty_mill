import { describe, expect, it } from 'vitest'
import { computeRange, formatRange, inRange } from './range'
import { NOW, at } from './testkit'

describe('computeRange', () => {
  it('today and yesterday are one calendar day each', () => {
    expect(computeRange('today', NOW, 1)).toEqual({ startMs: at(2026, 9, 29), endMs: at(2026, 9, 30) })
    expect(computeRange('yesterday', NOW, 1)).toEqual({ startMs: at(2026, 9, 28), endMs: at(2026, 9, 29) })
  })

  it.each([
    [0, at(2026, 9, 27)], // Sunday
    [1, at(2026, 9, 28)], // Monday
    [6, at(2026, 9, 26)], // Saturday
  ] as const)('this week honours week start %i', (weekStart, start) => {
    const r = computeRange('thisWeek', NOW, weekStart)!
    expect(r.startMs).toBe(start)
    expect(r.endMs).toBe(at(2026, 9, new Date(start).getDate() + 7))
    expect(inRange(NOW, r)).toBe(true)
  })

  it('last week is the seven days before this week', () => {
    expect(computeRange('lastWeek', NOW, 1)).toEqual({ startMs: at(2026, 9, 21), endMs: at(2026, 9, 28) })
    expect(computeRange('lastWeek', NOW, 0)).toEqual({ startMs: at(2026, 9, 20), endMs: at(2026, 9, 27) })
  })

  it('a week containing the end of daylight saving time is 169 hours, still ending at a midnight', () => {
    // DST ended Sun 1 Nov 2026 in Chicago: that week has 169 hours.
    const r = computeRange('thisWeek', at(2026, 11, 3, 12), 0)!
    expect(r.startMs).toBe(at(2026, 11, 1))
    expect(r.endMs).toBe(at(2026, 11, 8))
    expect((r.endMs - r.startMs) / 3_600_000).toBe(169)
  })

  it('months run from the 1st to the 1st', () => {
    expect(computeRange('thisMonth', NOW, 1)).toEqual({ startMs: at(2026, 9, 1), endMs: at(2026, 10, 1) })
    expect(computeRange('lastMonth', NOW, 1)).toEqual({ startMs: at(2026, 8, 1), endMs: at(2026, 9, 1) })
  })

  it('last month crosses the year boundary and handles a short February', () => {
    expect(computeRange('lastMonth', at(2026, 1, 15), 1)).toEqual({ startMs: at(2025, 12, 1), endMs: at(2026, 1, 1) })
    expect(computeRange('lastMonth', at(2026, 3, 31), 1)).toEqual({ startMs: at(2026, 2, 1), endMs: at(2026, 3, 1) })
    expect(computeRange('thisMonth', at(2026, 12, 31, 23, 59), 1)).toEqual({ startMs: at(2026, 12, 1), endMs: at(2027, 1, 1) })
  })

  it('a custom range is inclusive of its last day', () => {
    expect(computeRange('custom', NOW, 1, { from: '2026-09-01', to: '2026-09-03' })).toEqual({ startMs: at(2026, 9, 1), endMs: at(2026, 9, 4) })
  })

  it('a reversed custom range is put right; a bad or missing one is null', () => {
    expect(computeRange('custom', NOW, 1, { from: '2026-09-03', to: '2026-09-01' })).toEqual({ startMs: at(2026, 9, 1), endMs: at(2026, 9, 4) })
    expect(computeRange('custom', NOW, 1, { from: '', to: '2026-09-01' })).toBeNull()
    expect(computeRange('custom', NOW, 1, { from: '2026-02-30', to: '2026-03-01' })).toBeNull()
    expect(computeRange('custom', NOW, 1)).toBeNull()
  })
})

describe('inRange', () => {
  const r = { startMs: 100, endMs: 200 }
  it('includes the start, excludes the end, and rejects null', () => {
    expect(inRange(100, r)).toBe(true)
    expect(inRange(199, r)).toBe(true)
    expect(inRange(200, r)).toBe(false)
    expect(inRange(99, r)).toBe(false)
    expect(inRange(null, r)).toBe(false)
  })
})

describe('formatRange', () => {
  it('names one day, or first and last day', () => {
    expect(formatRange(computeRange('today', NOW, 1)!, NOW)).toBe('Sep 29')
    expect(formatRange(computeRange('thisWeek', NOW, 1)!, NOW)).toBe('Sep 28 – Oct 4')
  })
  it('adds the year when it is not the current one', () => {
    expect(formatRange(computeRange('lastMonth', at(2026, 1, 15), 1)!, at(2026, 1, 15))).toBe('Dec 1, 2025 – Dec 31, 2025')
  })
})
