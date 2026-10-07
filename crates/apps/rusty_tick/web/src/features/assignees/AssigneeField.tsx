import { UserRound } from 'lucide-react'
import { useEffect, useId, useMemo, useState } from 'react'
import { assigneeMap, knownNames, MAX_NAME } from './logic'
import { setAssignee, useAssignees } from './store'

/** The task's assignee: type a name (earlier names are suggested) and leave the field or press Enter to save; clear it to unassign. */
export function AssigneeField({ taskId, disabled }: { taskId: string; disabled?: boolean }) {
  const items = useAssignees((s) => s.items)
  const current = useMemo(() => assigneeMap(items)[taskId] ?? '', [items, taskId])
  const names = useMemo(() => knownNames(items), [items])
  const [draft, setDraft] = useState(current)
  const listId = useId()
  useEffect(() => setDraft(current), [current])
  const commit = (): void => {
    if (draft.trim() !== current) setAssignee(taskId, draft)
  }
  return (
    <label className="flex items-center gap-2 text-s text-grey">
      <UserRound size={14} aria-hidden />
      <span className="sr-only">Assignee</span>
      <input
        value={draft}
        disabled={disabled}
        maxLength={MAX_NAME}
        list={listId}
        placeholder="Assignee"
        aria-label="Assignee"
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') e.currentTarget.blur()
        }}
        className="h-7 min-w-0 flex-1 rounded-row bg-transparent px-1 text-base text-text outline-hidden placeholder:text-grey hover:bg-hover focus:bg-hover"
      />
      <datalist id={listId}>
        {names.map((n) => (
          <option key={n} value={n} />
        ))}
      </datalist>
    </label>
  )
}
