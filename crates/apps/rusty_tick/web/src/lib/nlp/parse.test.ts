import { describe, expect, it } from 'vitest'
import { dayKey } from '../date'
import { parseQuickAdd } from './parse'

const at = (y: number, m: number, d: number, h = 0, min = 0) => new Date(y, m - 1, d, h, min).getTime()
// Tuesday 2026-09-29, 12:00 local
const NOW = at(2026, 9, 29, 12)
const parse = (text: string) => parseQuickAdd(text, NOW)
const when = (text: string) => {
  const p = parse(text)
  return p.dueMs === null ? null : `${dayKey(p.dueMs)} ${String(new Date(p.dueMs).getHours()).padStart(2, '0')}:${String(new Date(p.dueMs).getMinutes()).padStart(2, '0')}${p.isAllDay ? ' (all day)' : ''}`
}

describe('plain text', () => {
  it('is left alone', () => {
    const p = parse('Buy milk and eggs')
    expect(p).toMatchObject({ title: 'Buy milk and eggs', dueMs: null, priority: null, tags: [], listName: null, tokens: [] })
  })

  it('collapses the whitespace tokens leave behind', () => {
    expect(parse('  Call   mum   tomorrow  ').title).toBe('Call mum')
  })
})

describe('dates', () => {
  it('today and tomorrow are all-day', () => {
    expect(when('pay rent today')).toBe('2026-09-29 00:00 (all day)')
    expect(when('pay rent Tomorrow')).toBe('2026-09-30 00:00 (all day)')
    expect(parse('pay rent tomorrow').title).toBe('pay rent')
  })

  it('next <weekday> is always in the future, even on that weekday', () => {
    expect(when('gym next mon')).toBe('2026-10-05 00:00 (all day)') // today is Tuesday
    expect(when('gym next monday')).toBe('2026-10-05 00:00 (all day)')
    expect(when('gym next tue')).toBe('2026-10-06 00:00 (all day)') // today is Tuesday: a week ahead, not today
  })

  it('a bare full weekday name is the coming one (today counts)', () => {
    expect(when('gym friday')).toBe('2026-10-02 00:00 (all day)')
    expect(when('gym tuesday')).toBe('2026-09-29 00:00 (all day)')
  })

  it('bare abbreviations stay in the title: "sat" and "mon" are also words', () => {
    expect(parse('meet the mon of the pack').dueMs).toBeNull()
  })

  it('in N days/weeks', () => {
    expect(when('renew in 3 days')).toBe('2026-10-02 00:00 (all day)')
    expect(when('renew in 2 weeks')).toBe('2026-10-13 00:00 (all day)')
    expect(when('renew in 1 day')).toBe('2026-09-30 00:00 (all day)')
  })

  it('in N hours/minutes is a moment, not a day', () => {
    expect(when('check oven in 90 minutes')).toBe('2026-09-29 13:30')
    expect(when('check oven in 2 hours')).toBe('2026-09-29 14:00')
  })

  it('ISO dates, and rejects impossible ones', () => {
    expect(when('trip 2026-12-25')).toBe('2026-12-25 00:00 (all day)')
    expect(parse('trip 2026-02-30').dueMs).toBeNull()
    expect(parse('trip 2026-02-30').title).toBe('trip 2026-02-30')
  })

  it('does not mistake "in" in ordinary text for a duration', () => {
    expect(parse('put the milk in the fridge').dueMs).toBeNull()
    expect(parse('lives in 3 places').dueMs).toBeNull()
  })
})

describe('times', () => {
  it('attaches to the date', () => {
    expect(when('call tomorrow 5pm')).toBe('2026-09-30 17:00')
    expect(when('call tomorrow at 17:30')).toBe('2026-09-30 17:30')
    expect(when('call tomorrow at 5:30pm')).toBe('2026-09-30 17:30')
    expect(when('call today 9am')).toBe('2026-09-29 09:00')
  })

  it('handles the 12 o\'clock edges', () => {
    expect(when('lunch tomorrow 12pm')).toBe('2026-09-30 12:00')
    expect(when('sleep tomorrow 12am')).toBe('2026-09-30 00:00')
  })

  it('a time alone means today, or tomorrow once it has passed', () => {
    expect(when('stand-up at 5pm')).toBe('2026-09-29 17:00')
    expect(when('stand-up 9am')).toBe('2026-09-30 09:00') // it is already noon
    expect(when('stand-up 13:00')).toBe('2026-09-29 13:00')
  })

  it('rejects impossible clock values', () => {
    expect(parse('meeting 13pm').dueMs).toBeNull()
    expect(parse('meeting 25:00').dueMs).toBeNull()
    expect(parse('meeting 13pm').title).toBe('meeting 13pm')
  })
})

describe('priority', () => {
  it.each([
    ['!high', 5], ['!medium', 3], ['!med', 3], ['!low', 1], ['!none', 0],
    ['!1', 5], ['!2', 3], ['!3', 1], ['!0', 0], ['!HIGH', 5],
  ] as const)('%s', (token, expected) => {
    const p = parse(`file taxes ${token}`)
    expect(p.priority).toBe(expected)
    expect(p.title).toBe('file taxes')
  })

  it('needs a word start and a whole word', () => {
    expect(parse('wow!high').priority).toBeNull()
    expect(parse('!highway').priority).toBeNull()
    expect(parse('what!?').priority).toBeNull()
  })

  it('the last one wins', () => {
    expect(parse('x !low !high').priority).toBe(5)
  })
})

describe('tags and lists', () => {
  it('collects tags in order, once each', () => {
    const p = parse('plan trip #travel #Fun #travel')
    expect(p.tags).toEqual(['travel', 'Fun'])
    expect(p.title).toBe('plan trip')
  })

  it('supports unicode and dashes in tags', () => {
    expect(parse('x #café #deep-work #a_b').tags).toEqual(['café', 'deep-work', 'a_b'])
  })

  it('a # in the middle of a word is not a tag', () => {
    expect(parse('issue#42 fix').tags).toEqual([])
    expect(parse('C# notes').tags).toEqual([])
  })

  it('reads a list, quoted or not', () => {
    expect(parse('buy paint ~home').listName).toBe('home')
    expect(parse('buy paint ~"Home Reno"').listName).toBe('Home Reno')
    expect(parse('buy paint ~home').title).toBe('buy paint')
  })

  it('sigil bodies are never read as dates', () => {
    const p = parse('#today ~tomorrow report')
    expect(p.tags).toEqual(['today'])
    expect(p.listName).toBe('tomorrow')
    expect(p.dueMs).toBeNull()
    expect(p.title).toBe('report')
  })
})

describe('tokens', () => {
  it('report offsets into the original text, for highlighting', () => {
    const text = 'call mum tomorrow 5pm !high #family'
    const p = parse(text)
    for (const t of p.tokens) expect(text.slice(t.start, t.end)).toBe(t.text)
    expect(p.tokens.map((t) => [t.kind, t.text])).toEqual([
      ['date', 'tomorrow'],
      ['time', '5pm'],
      ['priority', '!high'],
      ['tag', '#family'],
    ])
  })

  it('a token span never overlaps another', () => {
    const p = parse('x today at 5pm in 2 days next fri !1 #a ~b')
    const spans = p.tokens.map((t) => [t.start, t.end] as const)
    for (let i = 1; i < spans.length; i++) expect(spans[i]![0]).toBeGreaterThanOrEqual(spans[i - 1]![1])
  })
})

describe('everything at once', () => {
  it('parses a full line', () => {
    const p = parse('Submit report next fri at 5pm !1 #work ~"Q4 plan"')
    expect(p.title).toBe('Submit report')
    expect(when('Submit report next fri at 5pm')).toBe('2026-10-02 17:00')
    expect(p).toMatchObject({ priority: 5, tags: ['work'], listName: 'Q4 plan', isAllDay: false })
  })

  it('leaves an empty title when the line is all tokens', () => {
    expect(parse('tomorrow #a').title).toBe('')
  })
})
