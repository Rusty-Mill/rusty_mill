import { describe, expect, it } from 'vitest'
import {
  breakFor,
  computeStats,
  DEFAULT_SETTINGS,
  elapsedMs,
  formatClock,
  formatDuration,
  groupByDay,
  isFinished,
  remainingSec,
  ringProgress,
  sanitizeSettings,
  asRecordBody,
  type FocusRecord,
  type Session,
} from './logic'

const at = (y: number, mo: number, d: number, h = 12, mi = 0): number => new Date(y, mo - 1, d, h, mi).getTime()
const rec = (id: string, endMs: number, durationSec: number, kind: FocusRecord['kind'] = 'pomo'): FocusRecord => ({
  id, startMs: endMs - durationSec * 1000, endMs, durationSec, kind, taskId: null, taskTitle: null,
})

describe('formatting', () => {
  it('formats the clock', () => {
    expect(formatClock(1500)).toBe('25:00')
    expect(formatClock(0)).toBe('00:00')
    expect(formatClock(3725)).toBe('1:02:05')
    expect(formatClock(-5)).toBe('00:00')
  })
  it('formats durations', () => {
    expect(formatDuration(5100)).toBe('1h 25m')
    expect(formatDuration(3600)).toBe('1h 0m')
    expect(formatDuration(1500)).toBe('25m')
    expect(formatDuration(45)).toBe('45s')
    expect(formatDuration(0)).toBe('0m')
  })
})

describe('computeStats', () => {
  const now = at(2026, 9, 29, 15)
  it('is all zeros with no records', () => {
    expect(computeStats([], now)).toEqual({ todayPomos: 0, todaySec: 0, totalPomos: 0, totalSec: 0 })
  })
  it('splits today from the total and counts only pomos as pomos', () => {
    const records = [
      rec('a', at(2026, 9, 29, 9), 1500),
      rec('b', at(2026, 9, 29, 10), 600, 'stopwatch'),
      rec('c', at(2026, 9, 28, 23, 59), 1500),
    ]
    expect(computeStats(records, now)).toEqual({ todayPomos: 1, todaySec: 2100, totalPomos: 2, totalSec: 3600 })
  })
  it('treats local midnight as the day boundary', () => {
    const r = [rec('a', at(2026, 9, 30, 0, 0), 60)]
    expect(computeStats(r, now).todayPomos).toBe(0)
    expect(computeStats(r, at(2026, 9, 30, 8)).todayPomos).toBe(1)
  })
})

describe('groupByDay', () => {
  it('orders newest first and labels days', () => {
    const now = at(2026, 9, 29, 15)
    const groups = groupByDay([rec('old', at(2026, 9, 20), 60), rec('a', at(2026, 9, 29, 9), 60), rec('b', at(2026, 9, 29, 11), 60), rec('y', at(2026, 9, 28), 60)], now)
    expect(groups.map((g) => g.label)).toEqual(['Today', 'Yesterday', 'Sep 20'])
    expect(groups[0]?.records.map((r) => r.id)).toEqual(['b', 'a'])
  })
  it('adds the year for other years', () => {
    expect(groupByDay([rec('a', at(2025, 1, 5), 60)], at(2026, 9, 29))[0]?.label).toBe('Jan 5, 2025')
  })
})

describe('sessions', () => {
  const base: Session = { mode: 'pomo', phase: 'focus', targetSec: 1500, startedMs: 1_000_000, pausedAt: null, pausedTotalMs: 0, taskId: null, taskTitle: null }
  it('derives remaining time from timestamps', () => {
    expect(remainingSec(base, 1_000_000)).toBe(1500)
    expect(remainingSec(base, 1_000_000 + 90_500)).toBe(1410)
    expect(remainingSec(base, 1_000_000 + 9_999_999)).toBe(0)
    expect(isFinished(base, 1_000_000 + 1_499_999)).toBe(false)
    expect(isFinished(base, 1_000_000 + 1_500_000)).toBe(true)
  })
  it('does not count paused time', () => {
    const paused = { ...base, pausedAt: 1_060_000 }
    expect(elapsedMs(paused, 5_000_000)).toBe(60_000) // frozen while paused
    const resumed = { ...base, pausedTotalMs: 100_000 }
    expect(elapsedMs(resumed, 1_200_000)).toBe(100_000)
  })
  it('has no remaining time for a stopwatch', () => {
    const sw = { ...base, mode: 'stopwatch' as const, targetSec: null }
    expect(remainingSec(sw, 2_000_000)).toBeNull()
    expect(isFinished(sw, 9e12)).toBe(false)
  })
  it('computes ring progress', () => {
    expect(ringProgress(null, 'pomo', 0)).toBe(1)
    expect(ringProgress(base, 'pomo', 1_000_000 + 750_000)).toBeCloseTo(0.5)
  })
})

describe('breaks and settings', () => {
  it('every fourth pomo earns the long break', () => {
    expect(breakFor(1, DEFAULT_SETTINGS)).toEqual({ kind: 'short', sec: 300 })
    expect(breakFor(4, DEFAULT_SETTINGS)).toEqual({ kind: 'long', sec: 900 })
    expect(breakFor(8, DEFAULT_SETTINGS).kind).toBe('long')
  })
  it('sanitizes settings', () => {
    expect(sanitizeSettings(null)).toEqual(DEFAULT_SETTINGS)
    expect(sanitizeSettings({ focusMin: 0, shortMin: 999, longMin: 'x', longEvery: 3.4 })).toEqual({ focusMin: 1, shortMin: 60, longMin: 15, longEvery: 3 })
  })
  it('tells records from the settings doc', () => {
    expect(asRecordBody(DEFAULT_SETTINGS)).toBeNull()
    expect(asRecordBody({ startMs: 1, endMs: 2, durationSec: 1, kind: 'pomo', taskId: null, taskTitle: null })).not.toBeNull()
  })
})
