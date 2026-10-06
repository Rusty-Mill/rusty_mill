import { ArrowUpRight, Clock } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { taskPath } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { Popover } from '@/components/Popover'
import { TaskCheck } from '@/components/TaskCheck'
import { eventWhen, toEvent } from './layout'

interface Props {
  taskId: string
  anchor: HTMLElement | null
  color: string
  hour12: boolean
  onClose: () => void
}

/** The small card a click on a task opens: what it is, when, where, done, and a way into the detail pane. */
export function TaskPopover({ taskId, anchor, color, hour12, onClose }: Props) {
  const navigate = useNavigate()
  const actions = useActions()
  const task = useData((s) => s.tasks[taskId])
  const list = useData((s) => (task ? s.lists[task.listId] : undefined))
  const event = task ? toEvent(task, true) : null
  if (!task) return null

  return (
    <Popover anchor={anchor} open onClose={onClose} role="dialog" ariaLabel={task.title} className="w-[280px] p-3">
      <div className="flex items-start gap-2.5">
        <span className="mt-[3px]">
          <TaskCheck
            checked={task.status !== 'open'}
            priority={task.priority}
            label={`Complete ${task.title}`}
            onChange={() => {
              void actions.toggleDone(task.id)
              onClose() // the bar it hangs off may vanish when the task leaves the calendar
            }}
          />
        </span>
        <h2 className={`min-w-0 flex-1 break-words font-semibold ${task.status !== 'open' ? 'text-grey line-through' : ''}`}>{task.title}</h2>
      </div>
      <dl className="mt-2 space-y-1 pl-[26px] text-s text-grey">
        {event && (
          <div className="flex items-center gap-1.5">
            <dt className="sr-only">Due</dt>
            <Clock size={14} strokeWidth={1.5} aria-hidden />
            <dd>{eventWhen(event, hour12)}</dd>
          </div>
        )}
        <div className="flex items-center gap-1.5">
          <dt className="sr-only">List</dt>
          <span aria-hidden className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: color }} />
          <dd className="truncate">{list?.name ?? 'Inbox'}</dd>
        </div>
      </dl>
      <div className="mt-3 flex justify-end">
        <button
          type="button"
          onClick={() => {
            onClose()
            navigate(taskPath({ kind: 'list', id: task.listId }, task.id))
          }}
          className="flex h-8 items-center gap-1 rounded-row border border-line px-3 outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary"
        >
          Open
          <ArrowUpRight size={16} strokeWidth={1.5} aria-hidden />
        </button>
      </div>
    </Popover>
  )
}
