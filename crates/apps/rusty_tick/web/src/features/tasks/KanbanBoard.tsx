import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { taskPath } from '@/app/paths'
import { useActions } from '@/app/services'
import { TaskCheck } from '@/components/TaskCheck'
import { formatDue } from '@/lib/date'
import { usePrefs } from '../settings/prefs'
import { dropPatch, kanbanGroupBy } from './kanban'
import { groupTasks, type ViewSpec } from './organize'
import { useTaskActions } from './useTaskActions'
import type { ViewData } from './useViewData'

const TONE = { overdue: 'text-danger', today: 'text-primary', future: 'text-grey' } as const

/** Columns are the view's groups; dragging a card to another column edits the field they group by. */
export function KanbanBoard({ spec, data, selectedId }: { spec: ViewSpec; data: ViewData; selectedId: string | null }) {
  const navigate = useNavigate()
  const actions = useActions()
  const taskActions = useTaskActions()
  const hour12 = usePrefs((s) => s.prefs.hour12)
  const by = kanbanGroupBy(data.options.groupBy)
  const columns = groupTasks(data.tasks, by, { now: data.now, lists: data.lists, tags: data.tags })
  const [over, setOver] = useState<string | null>(null)

  const onDrop = (toKey: string, e: React.DragEvent): void => {
    e.preventDefault()
    setOver(null)
    const [id, fromKey] = e.dataTransfer.getData('application/x-tick-card').split('|')
    const task = data.tasks.find((t) => t.id === id)
    if (!task || fromKey === undefined) return
    const patch = dropPatch(task, by, fromKey, toKey, data.now)
    if (patch) void actions.updateTask(task.id, patch).catch((err: Error) => actions.notify('error', err.message))
  }

  if (columns.length === 0) return <div className="flex flex-1 items-center justify-center text-grey">No tasks</div>
  return (
    <div className="scroll-thin flex min-h-0 flex-1 gap-3 overflow-x-auto px-4 pb-4" role="list" aria-label="Board">
      {columns.map((col) => (
        <section
          key={col.key}
          role="listitem"
          aria-label={col.label}
          onDragOver={(e) => {
            if (e.dataTransfer.types.includes('application/x-tick-card')) {
              e.preventDefault()
              setOver(col.key)
            }
          }}
          onDragLeave={() => setOver((o) => (o === col.key ? null : o))}
          onDrop={(e) => onDrop(col.key, e)}
          className={`flex w-64 shrink-0 flex-col rounded-menu bg-black/[.03] ${over === col.key ? 'ring-2 ring-primary/40' : ''}`}
        >
          <h2 className={`px-3 py-2 text-base font-semibold ${col.tone === 'overdue' ? 'text-danger' : ''}`}>
            {col.label} <span className="font-normal text-grey">{col.tasks.length}</span>
          </h2>
          <ul className="scroll-thin flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
            {col.tasks.map((t) => {
              const due = formatDue(t, data.now, hour12)
              return (
                <li
                  key={t.id}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData('application/x-tick-card', `${t.id}|${col.key}`)
                    e.dataTransfer.effectAllowed = 'move'
                  }}
                  className={`cursor-grab rounded-row border border-line bg-surface p-2.5 ${t.id === selectedId ? 'ring-2 ring-primary/40' : ''}`}
                >
                  <div className="flex items-start gap-2">
                    <TaskCheck checked={t.status === 'done'} priority={t.priority} label={t.title} onChange={() => taskActions.toggle(t.id)} />
                    <button type="button" onClick={() => navigate(taskPath(spec, t.id))} className="min-w-0 flex-1 text-left text-base">
                      <span className="line-clamp-2">{t.title}</span>
                    </button>
                  </div>
                  {due && <p className={`mt-1 pl-6 text-s ${TONE[due.tone]}`}>{due.text}</p>}
                </li>
              )
            })}
          </ul>
        </section>
      ))}
    </div>
  )
}
