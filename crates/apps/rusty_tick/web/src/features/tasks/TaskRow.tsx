import { CalendarClock, ChevronRight, Copy, Flag, FolderInput, GripVertical, ListChecks, MoreHorizontal, Trash2 } from 'lucide-react'
import { memo, useRef, useState } from 'react'
import type { List, Priority, Tag, Task } from '@/api/types'
import { Menu, type MenuEntry } from '@/components/Menu'
import { PRIORITY_COLOR, PRIORITY_LABEL, TaskCheck } from '@/components/TaskCheck'
import { formatDue } from '@/lib/date'
import { checklistProgress } from './organize'

const TONE_CLASS = { overdue: 'text-danger', today: 'text-primary', future: 'text-grey' } as const

export interface RowActions {
  open: (id: string) => void
  toggle: (id: string) => void
  setPriority: (id: string, p: Priority) => void
  moveTo: (id: string, listId: string) => void
  duplicate: (t: Task) => void
  remove: (id: string) => void
}

interface Props {
  task: Task
  selected: boolean
  now: number
  hour12: boolean
  showDetails: boolean
  /** Hide the list name when the whole view is one list. */
  showList: boolean
  list: List | undefined
  tagsByName: Record<string, Tag>
  lists: List[]
  actions: RowActions
  dragProps?: React.HTMLAttributes<HTMLElement>
  dropIndicator?: 'before' | 'after' | null
}

/** One 40px task row. Memoised: a list of hundreds re-renders only the rows that changed. */
export const TaskRow = memo(function TaskRow({ task, selected, now, hour12, showDetails, showList, list, tagsByName, lists, actions, dragProps, dropIndicator }: Props) {
  const [menuOpen, setMenuOpen] = useState(false)
  const [at, setAt] = useState<{ x: number; y: number } | null>(null)
  const moreRef = useRef<HTMLButtonElement>(null)
  const pointRef = useRef<HTMLSpanElement>(null)
  const done = task.status === 'done'
  const due = formatDue(task, now, hour12)
  const progress = checklistProgress(task)

  const menu: MenuEntry[] = [
    { id: 'open', label: 'Open', icon: <ChevronRight size={16} />, onSelect: () => actions.open(task.id) },
    { id: 'dup', label: 'Duplicate', icon: <Copy size={16} />, onSelect: () => actions.duplicate(task) },
    {
      id: 'priority',
      label: 'Priority',
      icon: <Flag size={16} />,
      submenu: ([5, 3, 1, 0] as Priority[]).map((p) => ({
        id: `p${p}`,
        label: PRIORITY_LABEL[p],
        icon: <Flag size={16} fill={p ? PRIORITY_COLOR[p] : 'none'} color={PRIORITY_COLOR[p]} />,
        checked: task.priority === p,
        onSelect: () => actions.setPriority(task.id, p),
      })),
    },
    {
      id: 'move',
      label: 'Move to',
      icon: <FolderInput size={16} />,
      submenu: lists.filter((l) => !l.archived).map((l) => ({ id: l.id, label: l.name, checked: l.id === task.listId, onSelect: () => actions.moveTo(task.id, l.id) })),
    },
    'separator',
    { id: 'delete', label: 'Delete', icon: <Trash2 size={16} />, danger: true, onSelect: () => actions.remove(task.id) },
  ]

  const tagChips = showDetails ? task.tags.map((n) => tagsByName[n]).filter((t): t is Tag => !!t).slice(0, 2) : []

  return (
    <li
      {...dragProps}
      data-task-id={task.id}
      aria-current={selected ? 'true' : undefined}
      onContextMenu={(e) => {
        e.preventDefault()
        setAt({ x: e.clientX, y: e.clientY })
      }}
      className={`group relative flex h-10 items-center rounded-row border-b border-line/50 pr-2 transition-colors duration-150 ${selected ? 'bg-selected' : 'hover:bg-hover'} ${dropIndicator === 'before' ? 'before:absolute before:-top-px before:left-2 before:right-2 before:z-10 before:h-0.5 before:rounded before:bg-primary' : ''} ${dropIndicator === 'after' ? 'after:absolute after:-bottom-px after:left-2 after:right-2 after:z-10 after:h-0.5 after:rounded after:bg-primary' : ''}`}
    >
      {dragProps && (
        <span aria-hidden className="absolute -left-0.5 hidden cursor-grab text-grey group-hover:block">
          <GripVertical size={14} />
        </span>
      )}
      <div className="flex w-10 shrink-0 items-center justify-center">
        <TaskCheck checked={done} priority={task.priority} label={task.title} onChange={() => actions.toggle(task.id)} />
      </div>
      <button
        type="button"
        onClick={() => actions.open(task.id)}
        className="flex h-full min-w-0 flex-1 items-center gap-2 text-left"
      >
        <span className={`min-w-0 flex-1 truncate ${done ? 'text-grey line-through' : ''}`}>{task.title}</span>
        {progress && showDetails && (
          <span className="flex shrink-0 items-center gap-1 text-s text-grey" title="Checklist progress">
            <ListChecks size={13} aria-hidden />
            {progress}
          </span>
        )}
        {tagChips.map((t) => (
          <span key={t.name} className="shrink-0 rounded-full px-1.5 text-s" style={{ color: t.color ?? 'rgb(var(--grey))', backgroundColor: `${t.color ?? '#a8a8a8'}1a` }}>
            {t.label}
          </span>
        ))}
        {showDetails && showList && list && <span className="max-w-[110px] shrink-0 truncate text-s text-grey">{list.name}</span>}
        {due && (
          <span className={`flex shrink-0 items-center gap-1 text-s ${done ? 'text-grey' : TONE_CLASS[due.tone]}`}>
            {task.repeatFlag && <CalendarClock size={12} aria-label="Repeats" />}
            {due.text}
          </span>
        )}
      </button>
      <button
        ref={moreRef}
        type="button"
        aria-label={`Options for ${task.title}`}
        aria-haspopup="menu"
        onClick={() => setMenuOpen(true)}
        className="ml-1 hidden h-6 w-6 shrink-0 items-center justify-center rounded text-grey hover:bg-black/5 group-focus-within:flex group-hover:flex"
      >
        <MoreHorizontal size={16} />
      </button>
      <Menu anchor={moreRef.current} open={menuOpen} onClose={() => setMenuOpen(false)} items={menu} label="Task options" placement="bottom-end" />
      <span ref={pointRef} aria-hidden style={{ position: 'fixed', left: at?.x ?? 0, top: at?.y ?? 0, width: 0, height: 0 }} />
      <Menu anchor={at ? pointRef.current : null} open={!!at} onClose={() => setAt(null)} items={menu} label="Task options" />
    </li>
  )
})
