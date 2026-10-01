import { ChevronDown, Menu as MenuIcon, RotateCcw, Trash2 } from 'lucide-react'
import { useMemo, useRef, useState, type ReactNode } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import type { Task } from '@/api/types'
import { PATHS } from '@/app/paths'
import { NARROW } from '@/app/Shell'
import { useActions, useData } from '@/app/services'
import { Confirm } from '@/components/Confirm'
import { NoTasksArt } from '@/components/Illustrations'
import { Menu, type MenuEntry } from '@/components/Menu'
import { TaskCheck } from '@/components/TaskCheck'
import { formatTime } from '@/lib/date'
import { useMedia, useNow } from '@/lib/hooks'
import { useUi } from '@/store/ui'
import { usePrefs } from '../settings/prefs'
import { DetailPane } from './detail/DetailPane'
import { completedGroups, RANGE_LABEL, trashGroups, type CompletedFilter, type DateRange, type DayGroup } from './histories'
import { useTaskActions } from './useTaskActions'

function useTasksAndLists() {
  const taskMap = useData((s) => s.tasks)
  const listMap = useData((s) => s.lists)
  const tasks = useMemo(() => Object.values(taskMap), [taskMap])
  const lists = useMemo(() => Object.values(listMap).sort((a, b) => a.sortOrder - b.sortOrder), [listMap])
  return { tasks, lists, listMap }
}

function Shell({ title, base, taskId, toolbar, children }: { title: string; base: string; taskId: string | undefined; toolbar?: ReactNode; children: ReactNode }) {
  const narrow = useMedia(NARROW)
  const toggleSidebar = useUi((s) => s.toggleSidebar)
  const toggleDrawer = useUi((s) => s.toggleDrawer)
  const paths = useMemo(() => ({ list: base, task: (id: string) => `${base}/${id}` }), [base])
  return (
    <>
      <section aria-label={title} className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center gap-2 px-4">
          <button type="button" aria-label="Toggle sidebar" onClick={narrow ? toggleDrawer : toggleSidebar} className="flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover">
            <MenuIcon size={20} />
          </button>
          <h1 className="ml-1 min-w-0 flex-1 truncate text-title font-semibold">{title}</h1>
          {toolbar}
        </header>
        {children}
      </section>
      <DetailPane paths={paths} taskId={taskId ?? null} />
    </>
  )
}

function Groups({ groups, empty, row }: { groups: DayGroup[]; empty: string; row: (t: Task) => ReactNode }) {
  if (groups.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 pb-24 text-grey">
        <NoTasksArt />
        <span className="text-base font-semibold text-text">{empty}</span>
      </div>
    )
  }
  return (
    <div className="scroll-thin min-h-0 flex-1 overflow-y-auto px-2 pb-6">
      {groups.map((g) => (
        <section key={g.key} aria-label={g.label}>
          <h2 className="flex h-9 items-center gap-2 px-2 text-base font-semibold">
            {g.label}
            <span className="font-normal text-grey">{g.tasks.length}</span>
          </h2>
          <ul role="list">{g.tasks.map(row)}</ul>
        </section>
      ))}
    </div>
  )
}

function Dropdown({ label, value, items }: { label: string; value: string; items: MenuEntry[] }) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLButtonElement>(null)
  return (
    <>
      <button ref={ref} type="button" aria-label={label} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)} className="flex h-8 items-center gap-1 rounded-row px-2 text-base hover:bg-hover">
        {value}
        <ChevronDown size={14} className="text-grey" />
      </button>
      <Menu anchor={ref.current} open={open} onClose={() => setOpen(false)} items={items} label={label} placement="bottom-end" />
    </>
  )
}

/** Completed: filters by date and list, grouped by the day each task was finished. */
export function CompletedPage() {
  const { taskId } = useParams()
  const navigate = useNavigate()
  const now = useNow(60_000)
  const weekStart = usePrefs((s) => s.prefs.weekStart)
  const hour12 = usePrefs((s) => s.prefs.hour12)
  const taskActions = useTaskActions()
  const { tasks, lists, listMap } = useTasksAndLists()
  const [filter, setFilter] = useState<CompletedFilter>({ range: 'all', listId: null })
  const groups = useMemo(() => completedGroups(tasks, filter, now, weekStart), [tasks, filter, now, weekStart])

  const rangeItems: MenuEntry[] = (Object.keys(RANGE_LABEL) as DateRange[]).map((r) => ({ id: r, label: RANGE_LABEL[r], checked: filter.range === r, onSelect: () => setFilter((f) => ({ ...f, range: r })) }))
  const listItems: MenuEntry[] = [
    { id: 'all', label: 'All Lists', checked: filter.listId === null, onSelect: () => setFilter((f) => ({ ...f, listId: null })) },
    ...lists.map((l): MenuEntry => ({ id: l.id, label: l.name, checked: filter.listId === l.id, onSelect: () => setFilter((f) => ({ ...f, listId: l.id })) })),
  ]

  return (
    <Shell
      title="Completed"
      base={PATHS.completed}
      taskId={taskId}
      toolbar={
        <>
          <Dropdown label="Date range" value={RANGE_LABEL[filter.range]} items={rangeItems} />
          <Dropdown label="List" value={filter.listId === null ? 'All Lists' : (listMap[filter.listId]?.name ?? 'All Lists')} items={listItems} />
        </>
      }
    >
      <Groups
        groups={groups}
        empty="No completed tasks"
        row={(t) => (
          <li key={t.id} data-task-id={t.id} aria-current={t.id === taskId ? 'true' : undefined} className={`group flex h-10 items-center rounded-row pr-3 ${t.id === taskId ? 'bg-selected' : 'hover:bg-hover'}`}>
            <div className="flex w-10 shrink-0 items-center justify-center">
              <TaskCheck checked priority={t.priority} label={`Reopen: ${t.title}`} onChange={() => taskActions.toggle(t.id)} />
            </div>
            <button type="button" onClick={() => navigate(`${PATHS.completed}/${t.id}`)} className="flex h-full min-w-0 flex-1 items-center gap-2 text-left text-grey">
              <span className="min-w-0 flex-1 truncate line-through">{t.title}</span>
              <span className="shrink-0 text-s">{listMap[t.listId]?.name}</span>
              <span className="shrink-0 text-s">{t.completedMs !== null && formatTime(t.completedMs, hour12)}</span>
            </button>
          </li>
        )}
      />
    </Shell>
  )
}

/** Trash: deleted tasks, each restorable or deletable for good. */
export function TrashPage() {
  const { taskId } = useParams()
  const navigate = useNavigate()
  const actions = useActions()
  const taskActions = useTaskActions()
  const now = useNow(60_000)
  const { tasks, listMap } = useTasksAndLists()
  const groups = useMemo(() => trashGroups(tasks, now), [tasks, now])
  const total = groups.reduce((n, g) => n + g.tasks.length, 0)
  const [confirmEmpty, setConfirmEmpty] = useState(false)
  const [confirmOne, setConfirmOne] = useState<Task | null>(null)

  return (
    <Shell
      title="Trash"
      base={PATHS.trash}
      taskId={taskId}
      toolbar={
        <button type="button" disabled={total === 0} onClick={() => setConfirmEmpty(true)} className="flex h-8 items-center gap-1.5 rounded-row px-2 text-base text-danger hover:bg-hover disabled:text-grey disabled:opacity-50">
          <Trash2 size={16} /> Empty Trash
        </button>
      }
    >
      <Groups
        groups={groups}
        empty="Trash is empty"
        row={(t) => (
          <li key={t.id} data-task-id={t.id} aria-current={t.id === taskId ? 'true' : undefined} className={`group flex h-10 items-center rounded-row pl-4 pr-2 ${t.id === taskId ? 'bg-selected' : 'hover:bg-hover'}`}>
            <button type="button" onClick={() => navigate(`${PATHS.trash}/${t.id}`)} className="flex h-full min-w-0 flex-1 items-center gap-2 text-left text-grey">
              <span className="min-w-0 flex-1 truncate">{t.title}</span>
              <span className="shrink-0 text-s">{listMap[t.listId]?.name ?? 'Deleted list'}</span>
            </button>
            <button type="button" aria-label={`Restore ${t.title}`} title="Restore" onClick={() => taskActions.restore(t.id)} className="ml-1 hidden h-7 w-7 items-center justify-center rounded text-grey hover:bg-black/5 hover:text-primary group-focus-within:flex group-hover:flex">
              <RotateCcw size={15} />
            </button>
            <button type="button" aria-label={`Delete ${t.title} forever`} title="Delete forever" onClick={() => setConfirmOne(t)} className="hidden h-7 w-7 items-center justify-center rounded text-grey hover:bg-black/5 hover:text-danger group-focus-within:flex group-hover:flex">
              <Trash2 size={15} />
            </button>
          </li>
        )}
      />
      <Confirm
        open={confirmEmpty}
        title="Empty Trash?"
        message={`${total} task${total === 1 ? '' : 's'} will be deleted permanently. This cannot be undone.`}
        confirmLabel="Empty Trash"
        danger
        onCancel={() => setConfirmEmpty(false)}
        onConfirm={() => {
          setConfirmEmpty(false)
          navigate(PATHS.trash)
          void actions.emptyTrash().catch((e: Error) => actions.notify('error', e.message))
        }}
      />
      <Confirm
        open={!!confirmOne}
        title="Delete forever?"
        message={`"${confirmOne?.title}" will be deleted permanently. This cannot be undone.`}
        confirmLabel="Delete"
        danger
        onCancel={() => setConfirmOne(null)}
        onConfirm={() => {
          const t = confirmOne
          setConfirmOne(null)
          if (!t) return
          if (t.id === taskId) navigate(PATHS.trash)
          taskActions.purge(t.id)
        }}
      />
    </Shell>
  )
}
