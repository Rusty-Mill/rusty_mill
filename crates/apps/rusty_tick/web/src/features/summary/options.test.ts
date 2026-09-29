import { describe, expect, it } from 'vitest'
import { defaultOptions, sanitizeOptions } from './options'
import { NOW } from './testkit'

describe('sanitizeOptions', () => {
  it('falls back to defaults for junk', () => {
    expect(sanitizeOptions(null, NOW)).toEqual(defaultOptions(NOW))
    expect(sanitizeOptions('x', NOW)).toEqual(defaultOptions(NOW))
    expect(sanitizeOptions({ template: 'bogus', range: 3, status: 'nope', listIds: 'a', priorities: [9], custom: { from: 'x' }, showTags: 'yes' }, NOW)).toEqual(defaultOptions(NOW))
  })

  it('keeps well-formed values and drops malformed entries', () => {
    const o = sanitizeOptions({ template: 'daily', range: 'lastMonth', status: 'open', listIds: ['a', 1, 'b'], priorities: [5, 2, 0], tags: ['x'], groupBy: 'list', showTags: true, showList: false, custom: { from: '2026-01-01', to: '2026-01-31' } }, NOW)
    expect(o).toMatchObject({ template: 'daily', range: 'lastMonth', status: 'open', listIds: ['a', 'b'], priorities: [5, 0], tags: ['x'], groupBy: 'list', showTags: true, showList: false, custom: { from: '2026-01-01', to: '2026-01-31' } })
  })

  it('defaults to this week for the week template, with today as the custom range', () => {
    expect(defaultOptions(NOW)).toMatchObject({ range: 'thisWeek', custom: { from: '2026-09-29', to: '2026-09-29' } })
  })
})
