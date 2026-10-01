import { describe, expect, it } from 'vitest'
import { runApiContract } from './contract'
import { INBOX_ID, MemoryAdapter, STORAGE_KEY } from './memory'

runApiContract('MemoryAdapter', async () => new MemoryAdapter())

describe('MemoryAdapter persistence', () => {
  it('survives a reload through the storage it is given', async () => {
    const first = new MemoryAdapter({ storage: localStorage })
    const list = await first.createList({ name: 'Kept' })
    const task = await first.createTask({ listId: list.id, title: 'still here', tags: ['t'] })

    const second = new MemoryAdapter({ storage: localStorage })
    const snap = await second.snapshot()
    expect(snap.tasks.map((t) => t.id)).toEqual([task.id])
    expect(snap.lists.map((l) => l.name)).toEqual(['Inbox', 'Kept'])
    expect(snap.tags.map((t) => t.name)).toEqual(['t'])
  })

  it('sweeps comments whose task is gone when it loads, and keeps a trashed task\'s', async () => {
    const first = new MemoryAdapter({ storage: localStorage })
    const alive = await first.createTask({ listId: INBOX_ID, title: 'alive' })
    const binned = await first.createTask({ listId: INBOX_ID, title: 'binned' })
    await first.trashTask(binned.id)
    for (const taskId of [alive.id, binned.id, 'purged-long-ago']) await first.putDoc('comment', crypto.randomUUID(), { v: 1, taskId, text: 'x', createdMs: 1 })
    await first.putDoc('habit', crypto.randomUUID(), { name: 'not a comment' })

    const second = new MemoryAdapter({ storage: localStorage })
    expect((await second.listDocs<{ taskId: string }>('comment')).map((d) => d.body.taskId).sort()).toEqual([alive.id, binned.id].sort())
    expect(await second.listDocs('habit')).toHaveLength(1)
  })

  it('treats an unreadable save as empty rather than crashing', async () => {
    localStorage.setItem(STORAGE_KEY, '{not json')
    const api = new MemoryAdapter({ storage: localStorage })
    expect((await api.snapshot()).lists.map((l) => l.name)).toEqual(['Inbox'])
  })

  it('does not hand out its internal state', async () => {
    const api = new MemoryAdapter()
    const list = await api.createList({ name: 'L' })
    const t = await api.createTask({ listId: list.id, title: 'orig' })
    t.title = 'mutated by the caller'
    ;(await api.snapshot()).tasks[0]!.title = 'mutated snapshot'
    expect((await api.snapshot()).tasks[0]?.title).toBe('orig')
  })

  it('uses the clock it is given', async () => {
    let now = 1000
    const api = new MemoryAdapter({ now: () => now })
    const list = await api.createList({ name: 'L' })
    const t = await api.createTask({ listId: list.id, title: 't' })
    expect(t.createdMs).toBe(1000)
    now = 5000
    expect((await api.updateTask(t.id, { status: 'done' })).completedMs).toBe(5000)
  })
})
