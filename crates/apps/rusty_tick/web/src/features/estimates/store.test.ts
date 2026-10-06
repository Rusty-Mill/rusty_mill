import { beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { assigneeDocId } from '../assignees/logic'
import { setAssignee, useAssignees, resetAssigneesStore } from '../assignees/store'
import { estimateDocId } from './logic'
import { setEstimate, useEstimates, resetEstimatesStore } from './store'

const TASK = '0190a000-0000-7000-8000-0000000000aa'
const OTHER = '0190a000-0000-7000-8000-0000000000bb'
const notify = vi.fn()

async function setup(): Promise<MemoryAdapter> {
  const api = new MemoryAdapter()
  await useAssignees.getState().load(api, notify)
  await useEstimates.getState().load(api, notify)
  return api
}

describe('per-task documents', () => {
  beforeEach(() => {
    resetAssigneesStore()
    resetEstimatesStore()
    notify.mockReset()
  })

  it('a task can have an assignee and an estimate at once: the server keeps one document per id, so each kind gets its own', async () => {
    const api = await setup()
    setAssignee(TASK, 'Ada')
    setEstimate(TASK, 3)
    await vi.waitFor(async () => {
      expect((await api.listDocs('assignee')).map((d) => d.id)).toEqual([assigneeDocId(TASK)])
      expect((await api.listDocs('estimate')).map((d) => d.id)).toEqual([estimateDocId(TASK)])
    })
    expect(notify).not.toHaveBeenCalled() // no save was refused
  })

  it('changing and clearing them touches only their own document', async () => {
    const api = await setup()
    setAssignee(TASK, 'Ada')
    setEstimate(TASK, 3)
    setEstimate(TASK, 5)
    setAssignee(TASK, '')
    await vi.waitFor(async () => {
      expect(await api.listDocs('assignee')).toHaveLength(0)
      expect((await api.listDocs('estimate')).map((d) => d.body)).toEqual([{ taskId: TASK, pomos: 5 }])
    })
  })

  it('replaces a document an earlier version saved under the bare task id', async () => {
    const api = new MemoryAdapter()
    await api.putDoc('estimate', TASK, { taskId: TASK, pomos: 2 })
    await api.putDoc('assignee', OTHER, { taskId: OTHER, name: 'Old' })
    await useAssignees.getState().load(api, notify)
    await useEstimates.getState().load(api, notify)
    setEstimate(TASK, 4)
    setAssignee(OTHER, 'New')
    await vi.waitFor(async () => {
      expect((await api.listDocs('estimate')).map((d) => [d.id, (d.body as { pomos: number }).pomos])).toEqual([[estimateDocId(TASK), 4]])
      expect((await api.listDocs('assignee')).map((d) => [d.id, (d.body as { name: string }).name])).toEqual([[assigneeDocId(OTHER), 'New']])
    })
  })
})
