import { useState } from 'react'
import { useActions } from '@/app/services'
import { dropPatch, kanbanGroupBy } from './kanban'
import { groupTasks, type ViewSpec } from './organize'
import { TaskCard } from './TaskCard'
import type { ViewData } from './useViewData'

/** Columns are the view's groups; dragging a card to another column edits the field they group by. */
export function KanbanBoard({ spec, data, selectedId }: { spec: ViewSpec; data: ViewData; selectedId: string | null }) {
  const actions = useActions()
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
            {col.tasks.map((t) => (
              <TaskCard key={t.id} task={t} group={col.key} spec={spec} now={data.now} selected={t.id === selectedId} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  )
}
