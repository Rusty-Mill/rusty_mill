/**
 * Behaviour every `ApiClient` must have. Run against `MemoryAdapter` in the
 * unit tests and against `HttpAdapter` + the real `rusty_tick` binary in
 * `npm run test:integration`, so the in-browser fallback cannot drift from the
 * server it stands in for.
 */
import { describe, expect, it } from 'vitest'
import type { ApiClient } from './client'
import { ConflictError, InvalidError, NotFoundError, StaleError } from './errors'
import { newId } from '@/lib/id'

export const INBOX = '00000000-0000-7000-8000-000000000001'

export function runApiContract(label: string, make: () => Promise<ApiClient>): void {
  describe(`ApiClient contract: ${label}`, () => {
    const setup = async () => {
      const api = await make()
      const list = await api.createList({ name: 'Work' })
      return { api, list }
    }
    const openTasks = async (api: ApiClient) => (await api.snapshot()).tasks.filter((t) => t.deletedMs === null)

    it('starts with an Inbox that cannot be deleted or archived', async () => {
      const api = await make()
      const snap = await api.snapshot()
      expect(snap.inboxId).toBe(INBOX)
      expect(snap.lists.find((l) => l.id === INBOX)?.name).toBe('Inbox')
      await expect(api.deleteList(INBOX)).rejects.toBeInstanceOf(InvalidError)
      await expect(api.updateList(INBOX, { archived: true })).rejects.toBeInstanceOf(InvalidError)
    })

    it('creates, edits and deletes lists', async () => {
      const { api, list } = await setup()
      const edited = await api.updateList(list.id, { name: ' Home ', color: '#4772FA', viewMode: 'kanban', sortType: 'priority' })
      expect(edited).toMatchObject({ name: 'Home', color: '#4772fa', viewMode: 'kanban', sortType: 'priority' })
      await expect(api.updateList(list.id, { color: 'blue' })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.createList({ name: '   ' })).rejects.toBeInstanceOf(InvalidError)
      expect((await api.updateList(list.id, { color: null })).color).toBeNull()
      await api.deleteList(list.id)
      expect((await api.snapshot()).lists.some((l) => l.id === list.id)).toBe(false)
      await expect(api.deleteList(list.id)).rejects.toBeInstanceOf(NotFoundError)
    })

    it('keeps the Inbox first when lists are added', async () => {
      const { api } = await setup()
      await api.createList({ name: 'Second' })
      const names = (await api.snapshot()).lists.map((l) => l.name)
      expect(names[0]).toBe('Inbox')
      expect(names).toEqual(['Inbox', 'Work', 'Second'])
    })

    it('makes creates idempotent through client-chosen ids', async () => {
      const { api, list } = await setup()
      const id = newId()
      const input = { id, listId: list.id, title: 'once' }
      expect((await api.createTask(input)).id).toBe(id)
      await expect(api.createTask(input)).rejects.toBeInstanceOf(ConflictError)
      const listId = newId()
      await api.createList({ id: listId, name: 'Mine' })
      await expect(api.createList({ id: listId, name: 'Mine' })).rejects.toBeInstanceOf(ConflictError)
    })

    it('validates tasks at the boundary', async () => {
      const { api, list } = await setup()
      await expect(api.createTask({ listId: list.id, title: '  ' })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.createTask({ listId: list.id, title: 'x'.repeat(501) })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.createTask({ listId: list.id, title: 'x', priority: 2 as never })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.createTask({ listId: newId(), title: 'x' })).rejects.toBeInstanceOf(NotFoundError)
      await expect(api.createTask({ listId: list.id, title: 'x', tags: Array.from({ length: 21 }, (_, i) => `t${i}`) })).rejects.toBeInstanceOf(InvalidError)
    })

    it('appends by default and honours an explicit position', async () => {
      const { api, list } = await setup()
      await api.createTask({ listId: list.id, title: 'appended' })
      await api.createTask({ listId: list.id, title: 'on top', sortOrder: -5000 })
      const order = (await openTasks(api)).filter((t) => t.listId === list.id).sort((a, b) => a.sortOrder - b.sortOrder).map((t) => t.title)
      expect(order).toEqual(['on top', 'appended'])
    })

    it('round-trips the rich fields', async () => {
      const { api, list } = await setup()
      const item = { id: newId(), title: 'step', done: false, sortOrder: 1 }
      const t = await api.createTask({
        listId: list.id,
        title: 'rich',
        kind: 'checklist',
        startMs: 1000,
        dueMs: 2000,
        isAllDay: true,
        timeZone: 'America/Chicago',
        reminders: ['TRIGGER:PT0S'],
        repeatFlag: 'RRULE:FREQ=DAILY',
        items: [item],
      })
      expect(t).toMatchObject({ kind: 'checklist', startMs: 1000, dueMs: 2000, isAllDay: true, reminders: ['TRIGGER:PT0S'], repeatFlag: 'RRULE:FREQ=DAILY', items: [item] })
      const patched = await api.updateTask(t.id, { items: [{ ...item, done: true }], dueMs: null, exDates: [5] })
      expect(patched.items[0]?.done).toBe(true)
      expect(patched.dueMs).toBeNull()
      expect(patched.exDates).toEqual([5])
    })

    it('stamps completion and clears it on reopen', async () => {
      const { api, list } = await setup()
      const t = await api.createTask({ listId: list.id, title: 't' })
      const done = await api.updateTask(t.id, { status: 'done' })
      expect(done.status).toBe('done')
      expect(done.completedMs).toEqual(expect.any(Number))
      const again = await api.updateTask(t.id, { status: 'done' })
      expect(again.completedMs).toBe(done.completedMs)
      expect((await api.updateTask(t.id, { status: 'open' })).completedMs).toBeNull()
    })

    it('refuses stale writes with the current record, and accepts fresh ones', async () => {
      const { api, list } = await setup()
      const t = await api.createTask({ listId: list.id, title: 'v1' })
      const v2 = await api.updateTask(t.id, { title: 'v2' }, t.etag)
      expect(v2.etag).not.toBe(t.etag)
      const stale = await api.updateTask(t.id, { title: 'lost' }, t.etag).catch((e: unknown) => e)
      expect(stale).toBeInstanceOf(StaleError)
      expect((stale as StaleError<{ title: string }>).current.title).toBe('v2')
      expect((await openTasks(api)).find((x) => x.id === t.id)?.title).toBe('v2')
      expect((await api.updateTask(t.id, { title: 'no header' })).title).toBe('no header')
    })

    it('trashes, restores and purges, subtasks with their parent', async () => {
      const { api, list } = await setup()
      const parent = await api.createTask({ listId: list.id, title: 'p' })
      const child = await api.createTask({ listId: list.id, title: 'c', parentId: parent.id })
      await api.trashTask(parent.id)
      let snap = await api.snapshot()
      expect(snap.tasks.find((t) => t.id === parent.id)?.deletedMs).toEqual(expect.any(Number))
      expect(snap.tasks.find((t) => t.id === child.id)?.deletedMs).toEqual(expect.any(Number))
      const back = await api.restoreTask(parent.id)
      expect(back.deletedMs).toBeNull()
      snap = await api.snapshot()
      expect(snap.tasks.find((t) => t.id === child.id)?.deletedMs).toBeNull()

      await api.purgeTask(parent.id)
      snap = await api.snapshot()
      expect(snap.tasks.some((t) => t.id === parent.id || t.id === child.id)).toBe(false)

      const lone = await api.createTask({ listId: list.id, title: 'lone' })
      await api.trashTask(lone.id)
      expect(await api.emptyTrash()).toBe(1)
      expect(await api.emptyTrash()).toBe(0)
    })

    it('sends a deleted list\'s tasks to the trash, and restores them to the Inbox', async () => {
      const { api, list } = await setup()
      const t = await api.createTask({ listId: list.id, title: 'orphan' })
      await api.deleteList(list.id)
      expect((await api.snapshot()).tasks.find((x) => x.id === t.id)?.deletedMs).toEqual(expect.any(Number))
      expect((await api.restoreTask(t.id)).listId).toBe(INBOX)
    })

    it('moves a task with its subtasks, and refuses to move a subtask alone', async () => {
      const { api, list } = await setup()
      const other = await api.createList({ name: 'Other' })
      const parent = await api.createTask({ listId: list.id, title: 'p' })
      const child = await api.createTask({ listId: list.id, title: 'c', parentId: parent.id })
      expect((await api.updateTask(parent.id, { listId: other.id })).listId).toBe(other.id)
      expect((await openTasks(api)).find((t) => t.id === child.id)?.listId).toBe(other.id)
      await expect(api.updateTask(child.id, { listId: list.id })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.createTask({ listId: list.id, title: 'x', parentId: parent.id })).rejects.toBeInstanceOf(InvalidError)
    })

    it('treats tags as entities that outlive their tasks', async () => {
      const { api, list } = await setup()
      const t = await api.createTask({ listId: list.id, title: 't', tags: ['Home Office', 'home office'] })
      expect(t.tags).toEqual(['home office'])
      const tags = (await api.snapshot()).tags
      expect(tags).toHaveLength(1)
      expect(tags[0]).toMatchObject({ name: 'home office', label: 'Home Office', color: null })

      expect((await api.updateTag('home office', { color: '#ED70A5' })).color).toBe('#ed70a5')
      await expect(api.updateTag('home office', { color: 'pink' })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.createTag('HOME OFFICE')).rejects.toBeInstanceOf(ConflictError)

      await api.createTag('Parent')
      expect((await api.updateTag('home office', { parent: 'parent' })).parent).toBe('parent')
      await expect(api.updateTag('parent', { parent: 'parent' })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.updateTag('parent', { parent: 'ghost' })).rejects.toBeInstanceOf(InvalidError)

      const renamed = await api.renameTag('home office', 'Work')
      expect(renamed).toMatchObject({ name: 'work', label: 'Work', parent: 'parent' })
      expect((await openTasks(api)).find((x) => x.id === t.id)?.tags).toEqual(['work'])

      await api.deleteTag('work')
      expect((await openTasks(api)).find((x) => x.id === t.id)?.tags).toEqual([])
      await expect(api.deleteTag('work')).rejects.toBeInstanceOf(NotFoundError)
    })

    it('stores client documents, validating kind, size and ownership', async () => {
      const api = await make()
      const id = newId()
      const put = await api.putDoc('habit', id, { name: 'Read', goal: 30 })
      expect(put.body).toEqual({ name: 'Read', goal: 30 })
      await api.putDoc('habit', id, { name: 'Read more' })
      const docs = await api.listDocs<{ name: string }>('habit')
      expect(docs.map((d) => d.body.name)).toEqual(['Read more'])
      await expect(api.putDoc('nonsense' as never, id, {})).rejects.toBeInstanceOf(InvalidError)
      await expect(api.putDoc('focus', id, {})).rejects.toBeInstanceOf(ConflictError)
      await expect(api.putDoc('habit', newId(), { x: 'a'.repeat(70_000) })).rejects.toBeInstanceOf(InvalidError)
      await api.deleteDoc('habit', id)
      expect(await api.listDocs('habit')).toEqual([])
      await expect(api.deleteDoc('habit', id)).rejects.toBeInstanceOf(NotFoundError)
    })

    it('snapshots everything, trash and completed tasks included', async () => {
      const { api, list } = await setup()
      const a = await api.createTask({ listId: list.id, title: 'a', tags: ['t'] })
      const b = await api.createTask({ listId: list.id, title: 'b' })
      await api.updateTask(b.id, { status: 'done' })
      await api.trashTask(a.id)
      const snap = await api.snapshot()
      expect(snap.tasks).toHaveLength(2)
      expect(snap.lists).toHaveLength(2)
      expect(snap.tags[0]?.name).toBe('t')
      expect(snap.serverTimeMs).toEqual(expect.any(Number))
    })
  })
}
