import { useMemo } from 'react'
import { useData } from '@/app/services'
import { useNow } from '@/lib/hooks'
import { useUi } from '@/store/ui'
import { defaultOptions, groupTasks, sortTasks, tasksForView, viewKey, type Group, type ViewOptions, type ViewSpec } from './organize'
import { viewTitle } from './rows'
import type { List, Tag, Task } from '@/api/types'

export interface ViewData {
  title: string
  options: ViewOptions
  /** Open tasks in view order, before grouping. */
  tasks: Task[]
  groups: Group[]
  lists: List[]
  tags: Tag[]
  now: number
}

const NONE: ViewSpec = { kind: 'all' }

/** Everything a task view needs: its tasks sorted and grouped by the view's own options. */
export function useViewData(spec: ViewSpec | null): ViewData {
  const now = useNow(30_000)
  const inboxId = useData((s) => s.inboxId)
  const listMap = useData((s) => s.lists)
  const tagMap = useData((s) => s.tags)
  const taskMap = useData((s) => s.tasks)
  const stored = useUi((s) => (spec ? s.options[viewKey(spec)] : undefined))
  const view = spec ?? NONE

  const lists = useMemo(() => Object.values(listMap).sort((a, b) => a.sortOrder - b.sortOrder), [listMap])
  const tags = useMemo(() => Object.values(tagMap).sort((a, b) => a.sortOrder - b.sortOrder), [tagMap])
  const options = useMemo(() => ({ ...defaultOptions(view), ...stored }), [view, stored])

  return useMemo(() => {
    const entities = { tasks: Object.values(taskMap), lists, tags, inboxId }
    const open = sortTasks(tasksForView(view, entities, now), options.sortBy, options.order)
    const groups = groupTasks(open, options.groupBy, { now, lists, tags })
    if (options.showCompleted) {
      const done = tasksForView(view, entities, now, true).filter((t) => t.status === 'done').sort((a, b) => (b.completedMs ?? 0) - (a.completedMs ?? 0))
      if (done.length) groups.push({ key: 'done', label: 'Completed', tasks: done })
    }
    return { title: viewTitle(view, lists, tags, inboxId), options, tasks: open, groups, lists, tags, now }
  }, [view, taskMap, lists, tags, inboxId, now, options])
}
