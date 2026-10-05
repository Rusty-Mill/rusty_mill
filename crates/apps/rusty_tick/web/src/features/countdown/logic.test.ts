import { describe, expect, it } from 'vitest'
import { addDays, startOfDay } from '@/lib/date'
import { asCountdownBody, byUpcoming, daysLeft, describe as say, fromForm } from './logic'

const NOW = new Date(2026, 8, 29, 15).getTime()
const TODAY = startOfDay(NOW)

describe('daysLeft and describe', () => {
  it('counts whole calendar days, whatever the time of day', () => {
    expect(daysLeft(addDays(TODAY, 3), NOW)).toBe(3)
    expect(daysLeft(TODAY, NOW)).toBe(0)
    expect(daysLeft(addDays(TODAY, -2), NOW)).toBe(-2)
  })
  it('words them', () => {
    expect([say(0), say(1), say(12), say(-1), say(-5)]).toEqual(['Today', '1 day left', '12 days left', '1 day ago', '5 days ago'])
  })
})

describe('byUpcoming', () => {
  it('soonest first, then the past with the most recent first', () => {
    const at = (name: string, d: number) => ({ name, dateMs: addDays(TODAY, d) })
    const names = byUpcoming([at('far', 30), at('old', -10), at('soon', 2), at('recent', -1), at('today', 0)], NOW).map((c) => c.name)
    expect(names).toEqual(['today', 'soon', 'far', 'recent', 'old'])
  })
})

describe('bodies', () => {
  it('fromForm needs a name and a real date', () => {
    expect(fromForm(' Exam ', '2026-12-01')).toEqual({ name: 'Exam', dateMs: new Date(2026, 11, 1).getTime() })
    expect(fromForm('', '2026-12-01')).toBeNull()
    expect(fromForm('x', 'nope')).toBeNull()
  })
  it('asCountdownBody rejects other documents', () => {
    expect(asCountdownBody({ name: 'a', dateMs: 5 })).toEqual({ name: 'a', dateMs: 5 })
    expect(asCountdownBody({ name: '', dateMs: 5 })).toBeNull()
    expect(asCountdownBody({ name: 'a' })).toBeNull()
    expect(asCountdownBody(null)).toBeNull()
  })
})
