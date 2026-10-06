/** Subscribing to a calendar feed and keeping its tasks up to date: the effects around `planSync`. */
import type { ApiClient } from '@/api/client'
import type { Task } from '@/api/types'
import type { DataActions } from '@/store/data'
import type { DocItem } from '@/lib/docStore'
import { parseIcs } from '@/lib/ics'
import { planSync, sourceFields, type SubscriptionBody } from './logic'
import { useSubscriptions } from './store'

type Deps = { api: Pick<ApiClient, 'fetchIcs'>; actions: Pick<DataActions, 'createTask' | 'updateTask' | 'trashTask'>; tasks: Record<string, Task> }

export interface SyncResult {
  created: number
  updated: number
  trashed: number
}

/** Fetch the feed and bring the subscription's tasks in line with it. Throws if the fetch fails; nothing changes then. */
export async function syncSubscription(sub: DocItem<SubscriptionBody>, { api, actions, tasks }: Deps): Promise<SyncResult> {
  const feed = parseIcs(await api.fetchIcs(sub.url)).tasks
  const plan = planSync(feed, sub.items, tasks)
  const items = { ...plan.items }
  for (const event of plan.create) items[event.uid] = (await actions.createTask({ listId: sub.listId, ...sourceFields(event) })).id
  for (const { taskId, patch } of plan.update) await actions.updateTask(taskId, patch)
  for (const taskId of plan.trash) await actions.trashTask(taskId)
  const { id: _id, ...body } = sub
  useSubscriptions.getState().put(sub.id, { ...body, items, syncedMs: Date.now() })
  return { created: plan.create.length, updated: plan.update.length, trashed: plan.trash.length }
}

/** Create the list and the subscription, then sync once. If that first sync fails, both are undone. */
export async function subscribe(name: string, url: string, deps: Deps & { actions: Pick<DataActions, 'createList' | 'deleteList'> }): Promise<SyncResult> {
  const list = await deps.actions.createList({ name })
  const store = useSubscriptions.getState()
  const id = store.add({ name, url, listId: list.id, items: {}, syncedMs: 0 })
  try {
    return await syncSubscription({ id, name, url, listId: list.id, items: {}, syncedMs: 0 }, deps)
  } catch (error) {
    useSubscriptions.getState().remove(id)
    await deps.actions.deleteList(list.id)
    throw error
  }
}

/** Stop following a feed: its list, and so its tasks, go too. */
export async function unsubscribe(sub: DocItem<SubscriptionBody>, actions: Pick<DataActions, 'deleteList'>): Promise<void> {
  useSubscriptions.getState().remove(sub.id)
  await actions.deleteList(sub.listId)
}
