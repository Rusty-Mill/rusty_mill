import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { restoreBackup, wipeData } from './backup'

async function source(): Promise<MemoryAdapter> {
  const api = new MemoryAdapter()
  const inbox = (await api.snapshot()).inboxId
  const list = await api.createList({ name: 'Work', color: '#ff0000' })
  await api.createTag('urgent', '#00ff00')
  const parent = await api.createTask({ listId: list.id, title: 'parent', tags: ['urgent'] })
  await api.createTask({ listId: list.id, title: 'child', parentId: parent.id })
  const done = await api.createTask({ listId: inbox, title: 'done one' })
  await api.updateTask(done.id, { status: 'done' })
  const gone = await api.createTask({ listId: inbox, title: 'trashed' })
  await api.trashTask(gone.id)
  return api
}

describe('restoreBackup', () => {
  it('rebuilds lists, tags and live tasks (subtasks, completion) in an empty account', async () => {
    const text = JSON.stringify(await (await source()).snapshot())
    const fresh = new MemoryAdapter()
    const n = await restoreBackup(fresh, text)
    expect(n).toEqual({ lists: 1, tags: 1, tasks: 3 })
    const snap = await fresh.snapshot()
    expect(snap.lists.map((l) => l.name).sort()).toEqual(['Inbox', 'Work'])
    expect(snap.tasks.map((t) => t.title).sort()).toEqual(['child', 'done one', 'parent'])
    expect(snap.tasks.find((t) => t.title === 'done one')).toMatchObject({ status: 'done', listId: snap.inboxId })
    expect(snap.tasks.find((t) => t.title === 'child')?.parentId).toBe(snap.tasks.find((t) => t.title === 'parent')?.id)
  })

  it('is idempotent: a second import adds nothing', async () => {
    const text = JSON.stringify(await (await source()).snapshot())
    const fresh = new MemoryAdapter()
    await restoreBackup(fresh, text)
    expect(await restoreBackup(fresh, text)).toEqual({ lists: 0, tags: 0, tasks: 0 })
  })

  it.each(['not json', '{"lists":[]}', 'null'])('refuses %s', async (text) => {
    await expect(restoreBackup(new MemoryAdapter(), text)).rejects.toThrow('not a Tick Local backup')
  })
})

describe('wipeData', () => {
  it('leaves an empty Inbox and no documents', async () => {
    const api = await source()
    await api.putDoc('comment', crypto.randomUUID(), { v: 1 })
    await wipeData(api)
    const snap = await api.snapshot()
    expect(snap.tasks).toEqual([])
    expect(snap.tags).toEqual([])
    expect(snap.lists.map((l) => l.name)).toEqual(['Inbox'])
    expect(await api.listDocs('comment')).toEqual([])
  })
})
