import { useState } from 'react'
import { useActions } from '@/app/services'
import { QUADRANTS, matrixDropPatch, splitQuadrants, type Quadrant } from './matrix'
import type { ViewSpec } from './organize'
import { TaskCard } from './TaskCard'
import type { ViewData } from './useViewData'

/** Eisenhower matrix: four quadrants; dragging a card to one edits its priority and/or due date. */
export function MatrixBoard({ spec, data, selectedId }: { spec: ViewSpec; data: ViewData; selectedId: string | null }) {
  const actions = useActions()
  const quadrants = splitQuadrants(data.tasks, data.now)
  const [over, setOver] = useState<Quadrant | null>(null)

  const onDrop = (to: Quadrant, e: React.DragEvent): void => {
    e.preventDefault()
    setOver(null)
    const task = data.tasks.find((t) => t.id === e.dataTransfer.getData('application/x-tick-card').split('|')[0])
    const patch = task && matrixDropPatch(task, to, data.now)
    if (task && patch) void actions.updateTask(task.id, patch).catch((err: Error) => actions.notify('error', err.message))
  }

  return (
    <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 overflow-y-auto px-4 pb-4 md:grid-cols-2 md:grid-rows-2" role="list" aria-label="Eisenhower Matrix">
      {QUADRANTS.map((q) => (
        <section
          key={q.key}
          role="listitem"
          aria-label={q.label}
          onDragOver={(e) => {
            if (e.dataTransfer.types.includes('application/x-tick-card')) {
              e.preventDefault()
              setOver(q.key)
            }
          }}
          onDragLeave={() => setOver((o) => (o === q.key ? null : o))}
          onDrop={(e) => onDrop(q.key, e)}
          className={`flex min-h-40 flex-col rounded-menu bg-black/[.03] ${over === q.key ? 'ring-2 ring-primary/40' : ''}`}
        >
          <h2 className="px-3 py-2 text-base font-semibold">
            {q.label} <span className="font-normal text-grey">{quadrants[q.key].length}</span>
            <span className="block text-s font-normal text-grey">{q.hint}</span>
          </h2>
          <ul className="scroll-thin flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
            {quadrants[q.key].map((t) => (
              <TaskCard key={t.id} task={t} group={q.key} spec={spec} now={data.now} selected={t.id === selectedId} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  )
}
