import { useMemo } from 'react'
import { useParams } from 'react-router-dom'
import { useData } from './services'
import { parseView } from './paths'
import type { ViewSpec } from '@/features/tasks/organize'

/** The view the current route names, and the open task, if any. */
export function useView(): { spec: ViewSpec | null; taskId: string | null } {
  const params = useParams()
  const inboxId = useData((s) => s.inboxId)
  return useMemo(
    () => ({ spec: parseView(params, inboxId), taskId: params.taskId ?? null }),
    [params.smart, params.listId, params.tag, params.filterId, params.taskId, inboxId], // eslint-disable-line react-hooks/exhaustive-deps
  )
}
