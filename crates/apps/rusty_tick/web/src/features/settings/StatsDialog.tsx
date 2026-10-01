import { useMemo } from 'react'
import { useData } from '@/app/services'
import { Dialog } from '@/components/Dialog'
import { computeStats } from './stats'
import { usePrefs } from './prefs'

function Figure({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-row border border-line px-3 py-2.5">
      <dd className="text-h1 font-semibold">{value}</dd>
      <dt className="text-s text-grey">{label}</dt>
    </div>
  )
}

/** Real numbers from the tasks in memory: nothing is sent anywhere. */
export function StatsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const tasks = useData((s) => s.tasks)
  const lists = useData((s) => s.lists)
  const weekStart = usePrefs((s) => s.prefs.weekStart)
  // Recomputed each time the dialog opens, so "today" is right after midnight too.
  const stats = useMemo(() => (open ? computeStats(Object.values(tasks), Object.values(lists), Date.now(), weekStart) : null), [open, tasks, lists, weekStart])
  return (
    <Dialog open={open} onClose={onClose} title="Statistics" width={480}>
      {stats && (
        <div className="max-h-[70vh] overflow-y-auto px-6 pb-6">
          <dl className="grid grid-cols-3 gap-3">
            <Figure label="Completed today" value={stats.completedToday} />
            <Figure label="Completed this week" value={stats.completedThisWeek} />
            <Figure label="Completed all time" value={stats.completedAllTime} />
            <Figure label="Open tasks" value={stats.open} />
            <Figure label="Overdue" value={stats.overdue} />
            <Figure label="Day streak" value={stats.streak} />
          </dl>
          <h3 className="mb-1 mt-5 text-base font-semibold">Top lists by completed tasks</h3>
          {stats.topLists.length === 0 ? (
            <p className="text-base text-grey">No completed tasks yet.</p>
          ) : (
            <ol className="flex flex-col">
              {stats.topLists.map((l) => (
                <li key={l.listId} className="flex items-center justify-between border-b border-line py-2 last:border-b-0">
                  <span className="truncate">{l.name}</span>
                  <span className="text-grey">{l.count}</span>
                </li>
              ))}
            </ol>
          )}
        </div>
      )}
    </Dialog>
  )
}
