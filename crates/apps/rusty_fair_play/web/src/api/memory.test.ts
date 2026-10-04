import { describe, expect, it } from 'vitest'
import { runApiContract } from './contract'
import { MemoryAdapter } from './memory'

runApiContract('MemoryAdapter', async () => new MemoryAdapter())

describe('MemoryAdapter persistence', () => {
  it('saves to the given storage and loads it back', async () => {
    const a = new MemoryAdapter({ storage: localStorage })
    await a.seed()
    await a.createPerson('Ada')
    const b = new MemoryAdapter({ storage: localStorage })
    const snap = await b.snapshot()
    expect(snap.people.map((p) => p.name)).toEqual(['Ada'])
    expect(snap.cards).toHaveLength(12)
  })
})
