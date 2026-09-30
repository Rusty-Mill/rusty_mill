import { useState } from 'react'
import { useActions, useData } from '@/app/services'
import { Popover } from '@/components/Popover'
import { formatTime } from '@/lib/date'
import { usePrefs } from '../settings/prefs'
import { buildQuickAdd } from '../tasks/quickAddParse'
import type { AddTarget } from './context'
import { shortDate } from './layout'

interface Props {
  anchor: HTMLElement | null
  target: AddTarget
  onClose: () => void
}

/** A one-line add box for a day or slot. Natural language works: a date in the text overrides the slot. */
export function AddPopover({ anchor, target, onClose }: Props) {
  const actions = useActions()
  const inboxId = useData((s) => s.inboxId)
  const lists = useData((s) => s.lists)
  const tasks = useData((s) => s.tasks)
  const { hour12, defaultReminder } = usePrefs((s) => s.prefs)
  const [text, setText] = useState('')

  const submit = (e: React.FormEvent): void => {
    e.preventDefault()
    const built = buildQuickAdd(text, {
      now: Date.now(),
      lists: Object.values(lists),
      inboxId,
      view: { kind: 'inbox' },
      siblings: (listId) => Object.values(tasks).filter((t) => t.listId === listId && t.deletedMs === null),
      defaultReminder,
    })
    if (!built.ok) return
    const input = built.input
    if (input.dueMs === undefined) {
      input.dueMs = target.ms
      input.isAllDay = target.allDay
      if (!target.allDay && defaultReminder) input.reminders = [defaultReminder]
    }
    void actions.createTask(input)
    onClose()
  }

  return (
    <Popover anchor={anchor} open onClose={onClose} role="dialog" ariaLabel="Add task" placement="bottom-start" className="w-[300px] p-3">
      <form onSubmit={submit}>
        <div className="mb-2 text-s text-grey">{target.allDay ? shortDate(target.ms) : `${shortDate(target.ms)}, ${formatTime(target.ms, hour12)}`}</div>
        <div className="flex gap-2">
          <input
            autoFocus
            value={text}
            onChange={(e) => setText(e.target.value)}
            aria-label="Task title"
            placeholder="Add task"
            className="h-8 min-w-0 flex-1 rounded-row border border-line bg-surface px-2.5 outline-none focus:border-primary"
          />
          <button type="submit" className="h-8 rounded-row bg-primary px-3 text-white outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 disabled:opacity-50" disabled={!text.trim()}>
            Add
          </button>
        </div>
      </form>
    </Popover>
  )
}
