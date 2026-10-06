import { useNavigate } from 'react-router-dom'
import { taskPath } from '@/app/paths'
import type { Task } from '@/api/types'
import { TaskCheck } from '@/components/TaskCheck'
import { formatDue } from '@/lib/date'
import { usePrefs } from '../settings/prefs'
import type { ViewSpec } from './organize'
import { useTaskActions } from './useTaskActions'

const TONE = { overdue: 'text-danger', today: 'text-primary', future: 'text-grey' } as const

/** A draggable board card. `group` is the column it sits in; it rides along in the drag payload. */
export function TaskCard({ task, group, spec, now, selected }: { task: Task; group: string; spec: ViewSpec; now: number; selected: boolean }) {
  const navigate = useNavigate()
  const taskActions = useTaskActions()
  const hour12 = usePrefs((s) => s.prefs.hour12)
  const due = formatDue(task, now, hour12)
  return (
    <li
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData('application/x-tick-card', `${task.id}|${group}`)
        e.dataTransfer.effectAllowed = 'move'
      }}
      className={`cursor-grab rounded-row border border-line bg-surface p-2.5 ${selected ? 'ring-2 ring-primary/40' : ''}`}
    >
      <div className="flex items-start gap-2">
        <TaskCheck checked={task.status !== 'open'} priority={task.priority} label={task.title} onChange={() => taskActions.toggle(task.id)} />
        <button type="button" onClick={() => navigate(taskPath(spec, task.id))} className="min-w-0 flex-1 text-left text-base">
          <span className="line-clamp-2">{task.title}</span>
        </button>
      </div>
      {due && <p className={`mt-1 pl-6 text-s ${TONE[due.tone]}`}>{due.text}</p>}
    </li>
  )
}
