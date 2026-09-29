import { Plus, X } from 'lucide-react'
import { useEffect, useMemo, useRef } from 'react'
import type { ChecklistItem } from '@/api/types'
import { TaskCheck } from '@/components/TaskCheck'
import { newId } from '@/lib/id'
import { useDraft } from '@/lib/useDraft'
import { useReorderDrag } from '@/lib/useReorderDrag'
import { ORDER_STEP, reorderItems } from '../organize'

const same = (a: ChecklistItem[], b: ChecklistItem[]): boolean => JSON.stringify(a) === JSON.stringify(b)

interface Props {
  taskId: string
  items: ChecklistItem[]
  disabled?: boolean
  onChange: (items: ChecklistItem[]) => void
}

/** A task's checklist: tick, edit, add (Enter), remove (Backspace on empty), drag to reorder. */
export function Checklist({ taskId, items, disabled = false, onChange }: Props) {
  const draft = useDraft(taskId, items, onChange, 400, same)
  const list = useMemo(() => [...draft.value].sort((a, b) => a.sortOrder - b.sortOrder), [draft.value])
  const focusId = useRef<string | null>(null)
  const root = useRef<HTMLUListElement>(null)

  // Focus a row that was just added (after it renders).
  useEffect(() => {
    if (!focusId.current) return
    root.current?.querySelector<HTMLInputElement>(`[data-item="${focusId.current}"] input[type="text"]`)?.focus()
    focusId.current = null
  })

  const drag = useReorderDrag('application/x-tick-item', (moved, target, after) => {
    const r = reorderItems(list, moved, target, after)
    if (!r) return
    const orders = r.kind === 'set' ? { [r.id]: r.sortOrder } : r.orders
    draft.set(draft.value.map((i) => (orders[i.id] !== undefined ? { ...i, sortOrder: orders[i.id]! } : i)))
    draft.flush()
  })

  const patch = (id: string, change: Partial<ChecklistItem>, save = false): void => {
    draft.set(draft.value.map((i) => (i.id === id ? { ...i, ...change } : i)))
    if (save) draft.flush()
  }

  /** Insert a blank item after `afterId` (or at the end). */
  const add = (afterId: string | null, title = ''): void => {
    const at = afterId ? list.findIndex((i) => i.id === afterId) : list.length - 1
    const prev = list[at]?.sortOrder ?? null
    const next = list[at + 1]?.sortOrder ?? null
    const order = prev === null ? 0 : next === null ? prev + ORDER_STEP : prev + Math.max(1, Math.floor((next - prev) / 2))
    const item: ChecklistItem = { id: newId(), title, done: false, sortOrder: order }
    focusId.current = item.id
    draft.set([...draft.value, item])
  }

  const remove = (id: string): void => {
    const idx = list.findIndex((i) => i.id === id)
    focusId.current = list[idx - 1]?.id ?? null
    draft.set(draft.value.filter((i) => i.id !== id))
    draft.flush()
  }

  const done = list.filter((i) => i.done).length
  return (
    <div>
      {list.length > 0 && <p className="mb-1 text-s text-grey">{done}/{list.length}</p>}
      <ul ref={root} role="list" aria-label="Checklist">
        {list.map((item) => {
          const marker = drag.indicator?.id === item.id ? (drag.indicator.after ? 'after' : 'before') : null
          return (
            <li
              key={item.id}
              data-item={item.id}
              {...(disabled ? {} : drag.bind(item.id))}
              className={`group relative flex h-8 items-center gap-2 rounded-row px-1 hover:bg-hover ${marker === 'before' ? 'before:absolute before:-top-px before:inset-x-1 before:h-0.5 before:bg-primary' : ''} ${marker === 'after' ? 'after:absolute after:-bottom-px after:inset-x-1 after:h-0.5 after:bg-primary' : ''}`}
            >
              <TaskCheck size={14} checked={item.done} label={item.title || 'Checklist item'} onChange={() => !disabled && patch(item.id, { done: !item.done }, true)} />
              <input
                type="text"
                aria-label="Checklist item"
                value={item.title}
                disabled={disabled}
                maxLength={500}
                onChange={(e) => patch(item.id, { title: e.target.value })}
                onBlur={() => {
                  // An item left empty is not an item; a blur also saves what was typed.
                  if (item.title.trim() === '') remove(item.id)
                  else draft.flush()
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                    e.preventDefault()
                    if (item.title.trim() !== '') add(item.id)
                  } else if (e.key === 'Backspace' && item.title === '') {
                    e.preventDefault()
                    remove(item.id)
                  }
                }}
                className={`h-full min-w-0 flex-1 bg-transparent outline-none ${item.done ? 'text-grey line-through' : ''}`}
              />
              {!disabled && (
                <button type="button" aria-label="Remove item" onClick={() => remove(item.id)} className="hidden h-6 w-6 items-center justify-center rounded text-grey hover:bg-black/5 group-hover:flex">
                  <X size={14} />
                </button>
              )}
            </li>
          )
        })}
      </ul>
      {!disabled && (
        <button type="button" onClick={() => add(null)} className="mt-1 flex h-8 items-center gap-2 rounded-row px-1 text-grey hover:text-primary">
          <Plus size={16} /> Add item
        </button>
      )}
    </div>
  )
}
