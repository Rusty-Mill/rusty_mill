import { Trash2 } from 'lucide-react'
import { FocusArt } from '@/components/Illustrations'
import { formatTime } from '@/lib/date'
import { formatDuration, groupByDay, type FocusRecord } from './logic'

interface Props {
  records: readonly FocusRecord[]
  now: number
  hour12: boolean
  onDelete: (id: string) => void
}

export function FocusRecords({ records, now, hour12, onDelete }: Props) {
  if (records.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 pb-10 text-grey">
        <FocusArt />
        <p>No focus record yet</p>
      </div>
    )
  }
  return (
    <ul className="flex-1 overflow-y-auto px-5 pb-4" aria-label="Focus records">
      {groupByDay(records, now).map((g) => (
        <li key={g.key} className="mt-3">
          <h4 className="mb-1 text-s font-semibold text-grey">{g.label}</h4>
          <ul>
            {g.records.map((r) => (
              <li key={r.id} className="group flex items-center gap-3 rounded-row py-2 pl-2 pr-1 hover:bg-hover">
                <span aria-hidden className={`h-2 w-2 shrink-0 rounded-full ${r.kind === 'pomo' ? 'bg-primary' : 'bg-green'}`} />
                <div className="min-w-0 flex-1">
                  <div className="truncate">{r.taskTitle ?? (r.kind === 'pomo' ? 'Focus' : 'Stopwatch')}</div>
                  <div className="text-s text-grey">
                    {formatTime(r.startMs, hour12)} – {formatTime(r.endMs, hour12)}
                  </div>
                </div>
                <span className="tabular-nums text-grey">{formatDuration(r.durationSec)}</span>
                <button
                  type="button"
                  aria-label={`Delete record at ${formatTime(r.startMs, hour12)}`}
                  onClick={() => onDelete(r.id)}
                  className="rounded-row p-1.5 text-grey opacity-0 hover:bg-selected focus-visible:opacity-100 group-hover:opacity-100"
                >
                  <Trash2 size={16} strokeWidth={1.5} />
                </button>
              </li>
            ))}
          </ul>
        </li>
      ))}
    </ul>
  )
}
