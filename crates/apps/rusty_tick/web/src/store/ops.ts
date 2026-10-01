/**
 * Mutations as data. A pending operation is a plain object, so the queue can
 * be saved, replayed over server state to show an optimistic view, and sent
 * to the server, all from one description.
 */
import type { ApiClient } from '@/api/client'
import type { ListPatch, NewList, NewTask, TagPatch, TaskPatch } from '@/api/types'

export type Op =
  | { kind: 'createList'; input: NewList & { id: string } }
  | { kind: 'updateList'; id: string; patch: ListPatch }
  | { kind: 'deleteList'; id: string }
  | { kind: 'createTask'; input: NewTask & { id: string } }
  | { kind: 'updateTask'; id: string; patch: TaskPatch }
  | { kind: 'trashTask'; id: string }
  | { kind: 'restoreTask'; id: string }
  | { kind: 'purgeTask'; id: string }
  | { kind: 'emptyTrash' }
  | { kind: 'createTag'; label: string; color: string | null }
  | { kind: 'updateTag'; name: string; patch: TagPatch }
  | { kind: 'renameTag'; name: string; label: string }
  | { kind: 'deleteTag'; name: string }

export interface QueuedOp {
  opId: string
  op: Op
  /** Send attempts that ended stale (412); one retry is allowed. */
  stale: number
  /** Send attempts that failed in a way that might pass (a 5xx). */
  attempts: number
}

/** The etag to send with an op, if the record is known. */
export type EtagOf = (op: Op) => string | undefined

/** Run `op` against `api`. Resolves with the server's record where there is one. */
export async function runOp(api: ApiClient, op: Op, etagOf: EtagOf = () => undefined): Promise<unknown> {
  const etag = etagOf(op)
  switch (op.kind) {
    case 'createList':
      return api.createList(op.input)
    case 'updateList':
      return api.updateList(op.id, op.patch, etag)
    case 'deleteList':
      return api.deleteList(op.id)
    case 'createTask':
      return api.createTask(op.input)
    case 'updateTask':
      return api.updateTask(op.id, op.patch, etag)
    case 'trashTask':
      return api.trashTask(op.id, etag)
    case 'restoreTask':
      return api.restoreTask(op.id)
    case 'purgeTask':
      return api.purgeTask(op.id)
    case 'emptyTrash':
      return api.emptyTrash()
    case 'createTag':
      return api.createTag(op.label, op.color)
    case 'updateTag':
      return api.updateTag(op.name, op.patch, etag)
    case 'renameTag':
      return api.renameTag(op.name, op.label)
    case 'deleteTag':
      return api.deleteTag(op.name)
  }
}

/** Creates that a retry may find already done: the server has it, we lost the reply. */
export const isCreate = (op: Op): boolean => op.kind === 'createList' || op.kind === 'createTask' || op.kind === 'createTag'
