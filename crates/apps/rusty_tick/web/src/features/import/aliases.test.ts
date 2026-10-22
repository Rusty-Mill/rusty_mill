import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { readAliases, recordMerge } from './aliases'

const A = '00000000-0000-4000-8000-00000000000a'
const B = '00000000-0000-4000-8000-00000000000b'
const C = '00000000-0000-4000-8000-00000000000c'

describe('recordMerge', () => {
  it('records the duplicate as the task kept', async () => {
    const api = new MemoryAdapter()
    await recordMerge(api, A, B)
    expect(await readAliases(api)).toEqual(new Map([[A, B]]))
  })

  it('moves aliases of a task merged on, so they never chain', async () => {
    const api = new MemoryAdapter()
    await recordMerge(api, A, B)
    await recordMerge(api, B, C)
    expect(await readAliases(api)).toEqual(new Map([[A, C], [B, C]]))
  })

  it('drops the alias of a task merged away and back, so they never loop', async () => {
    const api = new MemoryAdapter()
    await recordMerge(api, A, B)
    await recordMerge(api, B, A)
    expect(await readAliases(api)).toEqual(new Map([[B, A]]))
  })
})
