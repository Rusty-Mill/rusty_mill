import { describe, expect, it } from 'vitest'
import { isUuid, newId } from './id'

describe('newId', () => {
  it('makes v7 UUIDs the backend accepts', () => {
    const id = newId()
    expect(isUuid(id)).toBe(true)
    expect(id[14]).toBe('7')
    expect('89ab').toContain(id[19])
  })

  it('embeds the timestamp so ids sort by creation time', () => {
    const a = newId(1_700_000_000_000)
    const b = newId(1_700_000_001_000)
    expect(a < b).toBe(true)
  })

  it('keeps ids from one millisecond ordered and unique', () => {
    const ids = Array.from({ length: 50 }, () => newId(1_800_000_000_000))
    expect(new Set(ids).size).toBe(50)
    expect([...ids].sort()).toEqual(ids)
  })

  it('does not go backwards if the clock does', () => {
    const later = newId(1_900_000_005_000)
    const earlier = newId(1_900_000_001_000)
    expect(earlier > later).toBe(true)
  })
})

describe('isUuid', () => {
  it('rejects the prompt-style 24-hex ids and junk', () => {
    expect(isUuid('5f1d7c9a8b3e4d2a1c0b9e8f')).toBe(false)
    expect(isUuid('')).toBe(false)
  })
})

describe('derivedId', () => {
  it('is stable, UUID-shaped, and different for different texts', async () => {
    const { derivedId } = await import('./id')
    expect(derivedId('estimate|t1')).toBe(derivedId('estimate|t1'))
    expect(derivedId('estimate|t1')).not.toBe(derivedId('assignee|t1'))
    expect(derivedId('estimate|t1')).not.toBe(derivedId('estimate|t2'))
    expect(isUuid(derivedId('estimate|t1'))).toBe(true)
  })
})
