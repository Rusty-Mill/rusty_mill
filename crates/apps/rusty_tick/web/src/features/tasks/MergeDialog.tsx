import { useMemo, useState } from 'react'
import type { Task } from '@/api/types'
import { useActions, useData, useServices } from '@/app/services'
import { Dialog } from '@/components/Dialog'
import { formatDue } from '@/lib/date'
import { recordMerge } from '../import/aliases'
import { mergeTargets } from './merge'

interface Props {
  /** The duplicate to merge away; `null` closes the dialog. */
  task: Task | null
  onClose: () => void
}

/**
 * Merge a duplicate into the task to keep: the kept task stays as it is, the
 * duplicate goes to the Trash (so it can be restored), and an import that
 * would add the duplicate again updates the kept task instead.
 */
export function MergeDialog({ task, onClose }: Props) {
  return (
    <Dialog open={task !== null} onClose={onClose} title="Merge into…" width={480}>
      {task && <MergeBody task={task} onDone={onClose} />}
    </Dialog>
  )
}

function MergeBody({ task, onDone }: { task: Task; onDone: () => void }) {
  const { api } = useServices()
  const actions = useActions()
  const taskMap = useData((s) => s.tasks)
  const listMap = useData((s) => s.lists)
  const [query, setQuery] = useState('')
  const now = Date.now()
  const targets = useMemo(() => mergeTargets(Object.values(taskMap), task, query), [taskMap, task, query])

  const merge = async (keep: Task): Promise<void> => {
    onDone()
    try {
      await recordMerge(api, task.id, keep.id)
      await actions.trashTask(task.id)
      actions.notify('info', `Merged into “${keep.title}”; the duplicate is in the Trash`)
    } catch (e) {
      actions.notify('error', `Could not merge: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <p className="text-s text-grey">
        Pick the task to keep. It stays as it is; “{task.title}” goes to the Trash, and an import that would add it again updates the kept task instead.
      </p>
      <input
        data-autofocus
        aria-label="Find the task to keep"
        autoComplete="off"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search tasks"
        className="h-9 rounded-row border border-line bg-transparent px-3 outline-hidden focus:border-primary"
      />
      {targets.length === 0 ? (
        <p className="py-4 text-center text-s text-grey">{query ? 'No task matches.' : 'No other task has this title. Search for the one to keep.'}</p>
      ) : (
        <ul aria-label="Tasks to keep" className="max-h-72 overflow-y-auto">
          {targets.map((t) => (
            <li key={t.id}>
              <button type="button" onClick={() => void merge(t)} className="flex h-10 w-full items-center gap-2 rounded-row px-2 text-left hover:bg-hover">
                <span className="min-w-0 flex-1 truncate">{t.title}</span>
                <span className="shrink-0 text-s text-grey">{[listMap[t.listId]?.name, formatDue(t, now)?.text].filter(Boolean).join(' · ')}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
