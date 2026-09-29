import { formatDuration, type FocusStats } from './logic'

/** The four stat tiles. */
export function Overview({ stats }: { stats: FocusStats }) {
  const tiles: [string, string][] = [
    ["Today's Pomos", String(stats.todayPomos)],
    ["Today's Focus", formatDuration(stats.todaySec)],
    ['Total Pomos', String(stats.totalPomos)],
    ['Total Focus', formatDuration(stats.totalSec)],
  ]
  return (
    <section aria-label="Overview" className="px-5 pt-5">
      <h3 className="mb-3 font-semibold">Overview</h3>
      <dl className="grid grid-cols-2 gap-2">
        {tiles.map(([label, value]) => (
          <div key={label} className="rounded-row bg-side px-3 py-2.5">
            <dt className="text-s text-grey">{label}</dt>
            <dd className="mt-0.5 text-title font-semibold tabular-nums">{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  )
}
