import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { applyImport, readImport } from '.'
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

describe('importing over a merge', () => {
  it('does not close the task kept, untouched, when the duplicate merged into it was done', async () => {
    const api = new MemoryAdapter()
    const inbox = (await api.snapshot()).inboxId
    const keep = await api.createTask({ listId: inbox, title: 'Pay rent' })
    const file = 'Title,Completed\nPay rent,yes\n'
    await applyImport(api, readImport(file, 'r.csv'))
    const duplicate = (await api.snapshot()).tasks.find((t) => t.id !== keep.id)!
    await recordMerge(api, duplicate.id, keep.id)
    await api.purgeTask(duplicate.id)
    expect(await applyImport(api, readImport(file, 'r.csv'))).toMatchObject({ tasks: 0, existing: 1, resumed: 0 })
    expect((await api.snapshot()).tasks.find((t) => t.id === keep.id)).toMatchObject({ status: 'open', etag: '1' })
  })
})
