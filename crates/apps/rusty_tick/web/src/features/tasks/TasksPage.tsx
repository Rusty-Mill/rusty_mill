import { useEffect, useMemo } from 'react'
import { Navigate, useNavigate } from 'react-router-dom'
import { HOME, taskPath, viewPath } from '@/app/paths'
import { useActions } from '@/app/services'
import { useView } from '@/app/useView'
import { useUi } from '@/store/ui'
import { DetailPane } from './DetailPane'
import { ListColumn } from './ListColumn'
import { shortcutFor } from './shortcuts'
import { useTaskActions } from './useTaskActions'
import { useViewData } from './useViewData'
import type { ViewSpec } from './organize'

/** The list column and the detail pane for the view in the URL. */
export function TasksPage() {
  const { spec, taskId } = useView()
  if (!spec) return <Navigate to={HOME} replace />
  return (
    <>
      <ListColumn spec={spec} selectedId={taskId} />
      <DetailPane spec={spec} taskId={taskId} />
      <TaskShortcuts spec={spec} selectedId={taskId} />
    </>
  )
}

/** Keyboard control of the selected task. Renders nothing. */
function TaskShortcuts({ spec, selectedId }: { spec: ViewSpec; selectedId: string | null }) {
  const navigate = useNavigate()
  const actions = useActions()
  const taskActions = useTaskActions()
  const { groups } = useViewData(spec)
  const ids = useMemo(() => groups.flatMap((g) => g.tasks.map((t) => t.id)), [groups])
  const collapsed = useUi((s) => s.collapsedGroups)
  void collapsed

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const action = shortcutFor({ key: e.key, ctrlKey: e.ctrlKey, metaKey: e.metaKey, altKey: e.altKey, shiftKey: e.shiftKey, target: e.target as HTMLElement | null })
      if (!action) return
      // Something modal or floating is open: it owns Escape and the keys.
      const overlay = document.querySelector('[data-popover], [role="dialog"]')
      if (overlay && action.type !== 'search') return
      const at = selectedId ? ids.indexOf(selectedId) : -1
      switch (action.type) {
        case 'newTask':
          e.preventDefault()
          return useUi.getState().focusQuickAdd()
        case 'search':
          e.preventDefault()
          return useUi.getState().setSearchOpen(true)
        case 'next':
        case 'previous': {
          const to = ids[action.type === 'next' ? Math.min(at + 1, ids.length - 1) : Math.max(at - 1, 0)]
          if (to) navigate(taskPath(spec, to))
          return
        }
        case 'close':
          if (selectedId) navigate(viewPath(spec))
          return
      }
      if (!selectedId) return
      switch (action.type) {
        case 'toggle':
          e.preventDefault()
          return taskActions.toggle(selectedId)
        case 'delete':
          e.preventDefault()
          taskActions.remove(selectedId)
          // Land on a neighbour so J/K and Delete can be chained.
          return navigate(ids[at + 1] || ids[at - 1] ? taskPath(spec, (ids[at + 1] ?? ids[at - 1])!) : viewPath(spec))
        case 'priority':
          return taskActions.setPriority(selectedId, action.value)
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [ids, selectedId, spec, navigate, taskActions, actions])

  return null
}
