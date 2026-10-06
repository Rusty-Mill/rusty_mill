import type { EstimateRow } from './logic'

/** Estimated against actual pomos, for every task that has an estimate. */
export function EstimatesPanel({ rows }: { rows: EstimateRow[] }) {
  if (rows.length === 0) return null
  return (
    <section aria-label="Estimates" className="px-5 pt-5">
      <h3 className="mb-2 font-semibold">Estimates</h3>
      <ul>
        {rows.map((r) => (
          <li key={r.taskId} className="flex items-center gap-3 py-1">
            <span className="min-w-0 flex-1 truncate">{r.title}</span>
            <span className={`tabular-nums ${r.actual > r.estimated ? 'text-danger' : 'text-grey'}`}>
              {r.actual} / {r.estimated} pomos
            </span>
          </li>
        ))}
      </ul>
    </section>
  )
}
