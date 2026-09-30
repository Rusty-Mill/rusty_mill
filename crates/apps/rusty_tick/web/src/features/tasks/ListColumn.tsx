import { Menu as MenuIcon } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import type { ViewMode } from '@/api/types'
import { taskPath } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { NoTasksArt } from '@/components/Illustrations'
import { useMedia } from '@/lib/hooks'
import { useReorderDrag } from '@/lib/useReorderDrag'
import { NARROW } from '@/app/Shell'
import { useUi } from '@/store/ui'
import { usePrefs } from '../settings/prefs'
import { KanbanBoard } from './KanbanBoard'
import { QuickAdd } from './QuickAdd'
import { TaskRow, type RowActions } from './TaskRow'
import { reorderItems, viewKey, type ViewSpec } from './organize'
import { buildRows, windowRows, ROW_HEIGHT, HEADER_HEIGHT, VIRTUALIZE_ABOVE, type Row } from './rows'
import { useTaskActions } from './useTaskActions'
import { useViewData } from './useViewData'
import { MoreMenu, SortMenu } from './ViewMenus'
import { ChevronDown, ChevronRight } from 'lucide-react'

interface Props {
  spec: ViewSpec
  selectedId: string | null
}

/** The middle column: title, sort and view menus, the quick-add box, and the grouped rows. */
export function ListColumn({ spec, selectedId }: Props) {
  const navigate = useNavigate()
  const actions = useActions()
  const taskActions = useTaskActions()
  const inboxId = useData((s) => s.inboxId)
  const data = useViewData(spec)
  const { options, groups, lists, tags, now } = data
  const setOptions = useUi((s) => s.setOptions)
  const collapsedGroups = useUi((s) => s.collapsedGroups)
  const toggleGroup = useUi((s) => s.toggleGroup)
  const toggleSidebar = useUi((s) => s.toggleSidebar)
  const toggleDrawer = useUi((s) => s.toggleDrawer)
  const narrow = useMedia(NARROW)
  const focusQuickAdd = useUi((s) => s.quickAddFocus)
  const hour12 = usePrefs((s) => s.prefs.hour12)
  void focusQuickAdd

  const key = viewKey(spec)
  const list = spec.kind === 'list' ? lists.find((l) => l.id === spec.id) : undefined
  const viewMode: ViewMode = list ? list.viewMode : options.viewMode
  const setViewMode = (mode: ViewMode): void => {
    setOptions(spec, { viewMode: mode })
    if (list) void actions.updateList(list.id, { viewMode: mode })
  }

  const listById = useMemo(() => Object.fromEntries(lists.map((l) => [l.id, l])), [lists])
  const tagsByName = useMemo(() => Object.fromEntries(tags.map((t) => [t.name, t])), [tags])
  const rows = useMemo(() => buildRows(groups, collapsedGroups, key), [groups, collapsedGroups, key])

  const rowActions: RowActions = useMemo(
    () => ({
      open: (id) => navigate(taskPath(spec, id)),
      toggle: taskActions.toggle,
      setPriority: taskActions.setPriority,
      moveTo: taskActions.moveTo,
      duplicate: (t) => void taskActions.duplicate(t),
      remove: taskActions.remove,
    }),
    [navigate, spec, taskActions],
  )

  // Dragging reorders only when the order is the user's own and rows are not split into groups.
  const canDrag = options.sortBy === 'manual' && options.groupBy === 'none' && !options.showCompleted
  const drag = useReorderDrag('application/x-tick-task', (moved, target, after) => {
    const r = reorderItems(data.tasks, moved, target, after)
    if (!r) return
    if (r.kind === 'set') void actions.reorderTask(r.id, r.sortOrder)
    else for (const [id, sortOrder] of Object.entries(r.orders)) void actions.reorderTask(id, sortOrder)
  })

  const total = groups.reduce((n, g) => n + g.tasks.length, 0)
  const showList = spec.kind !== 'list' && spec.kind !== 'inbox'

  return (
    <section aria-label={data.title} className="flex min-w-0 flex-1 flex-col">
      <header className="flex h-14 shrink-0 items-center gap-1 px-4">
        <button type="button" aria-label="Toggle sidebar" onClick={narrow ? toggleDrawer : toggleSidebar} className="flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover">
          <MenuIcon size={20} />
        </button>
        <h1 className="ml-1 min-w-0 flex-1 truncate text-title font-semibold">{data.title}</h1>
        <SortMenu options={options} onChange={(patch) => setOptions(spec, patch)} />
        <MoreMenu
          options={options}
          onChange={(patch) => setOptions(spec, patch)}
          viewMode={viewMode}
          onViewMode={setViewMode}
          onViewOptions={() => document.querySelector<HTMLButtonElement>('button[aria-label="Sort"]')?.click()}
        />
      </header>

      <QuickAdd view={spec} lists={lists} inboxId={inboxId} tasks={data.tasks} now={now} />

      {viewMode === 'kanban' ? (
        <KanbanBoard spec={spec} data={data} selectedId={selectedId} />
      ) : viewMode === 'timeline' ? (
        <div className="flex flex-1 items-center justify-center text-grey">The timeline view is not available yet.</div>
      ) : total === 0 ? (
        <button type="button" onClick={() => useUi.getState().focusQuickAdd()} className="flex flex-1 flex-col items-center justify-center gap-2 pb-24 text-grey outline-none">
          <NoTasksArt />
          <span className="text-base font-semibold text-text">No tasks</span>
          <span className="text-s">Click the input box to add</span>
        </button>
      ) : (
        <RowList
          rows={rows}
          renderTask={(r) => {
            const dropIndicator = drag.indicator?.id === r.task.id ? (drag.indicator.after ? 'after' : 'before') : null
            return (
              <TaskRow
                key={r.task.id}
                task={r.task}
                selected={r.task.id === selectedId}
                now={now}
                hour12={hour12}
                showDetails={options.showDetails}
                showList={showList}
                list={listById[r.task.listId]}
                tagsByName={tagsByName}
                lists={lists}
                actions={rowActions}
                dragProps={canDrag && r.groupKey !== 'done' ? drag.bind(r.task.id) : undefined}
                dropIndicator={dropIndicator}
              />
            )
          }}
          onToggleGroup={(g) => toggleGroup(`${key}:${g}`)}
          selectedId={selectedId}
        />
      )}
    </section>
  )
}

interface RowListProps {
  rows: Row[]
  renderTask: (r: Extract<Row, { type: 'task' }>) => React.ReactNode
  onToggleGroup: (groupKey: string) => void
  selectedId: string | null
}

/** Draws every row, or only the visible window once there are more than a couple of hundred. */
function RowList({ rows, renderTask, onToggleGroup, selectedId }: RowListProps) {
  const scroller = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewport, setViewport] = useState(800)
  const virtual = rows.length > VIRTUALIZE_ABOVE

  useEffect(() => {
    const el = scroller.current
    if (!el || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(() => setViewport(el.clientHeight))
    ro.observe(el)
    setViewport(el.clientHeight)
    return () => ro.disconnect()
  }, [])

  // Keep the selected task in view when it changes (J/K, or opening one).
  useEffect(() => {
    if (!selectedId || !scroller.current) return
    const el = scroller.current.querySelector<HTMLElement>(`[data-task-id="${selectedId}"]`)
    el?.scrollIntoView?.({ block: 'nearest' })
  }, [selectedId])

  const onScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => setScrollTop(e.currentTarget.scrollTop), [])
  const win = virtual ? windowRows(rows, scrollTop, viewport) : { start: 0, end: rows.length, offset: 0, total: 0 }
  const shown = rows.slice(win.start, win.end)

  const body = (
    <ul role="list" aria-label="Tasks" className="px-2" style={virtual ? { position: 'absolute', top: win.offset, left: 0, right: 0 } : undefined}>
      {shown.map((r) =>
        r.type === 'header' ? (
          <li key={`h:${r.key}`} style={{ height: HEADER_HEIGHT }} className="flex items-center">
            <button type="button" aria-expanded={!r.collapsed} onClick={() => onToggleGroup(r.key)} className="flex items-center gap-1 rounded px-1 text-base font-semibold hover:text-primary">
              {r.collapsed ? <ChevronRight size={16} className="text-grey" /> : <ChevronDown size={16} className="text-grey" />}
              <span className={r.tone === 'overdue' ? 'text-danger' : ''}>{r.label}</span>
              <span className="ml-1 font-normal text-grey">{r.count}</span>
            </button>
          </li>
        ) : (
          renderTask(r)
        ),
      )}
    </ul>
  )

  return (
    <div ref={scroller} onScroll={onScroll} className="scroll-thin relative min-h-0 flex-1 overflow-y-auto pb-6" style={{ ['--row' as string]: `${ROW_HEIGHT}px` }}>
      {virtual ? <div style={{ height: win.total, position: 'relative' }}>{body}</div> : body}
    </div>
  )
}
