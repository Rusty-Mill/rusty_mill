import { diffDays, formatTime } from '@/lib/date'
import type { CalendarCtx } from './context'
import { agendaGroups, shortDate, type CalEvent, type Range } from './layout'

interface Props {
  range: Range
  events: CalEvent[]
  ctx: CalendarCtx
  listName: (listId: string) => string
}

/** When in the day a row happens: the start time, or `All day`. */
function timeText(e: CalEvent, day: number, hour12: boolean): string {
  if (e.kind === 'timed') return formatTime(e.startMs, hour12)
  if (e.startDay === e.endDay) return 'All day'
  return e.startDay === day ? 'Starts' : e.endDay === day ? 'Ends' : 'All day'
}

/** Upcoming tasks grouped by day, days with none left out. */
export function AgendaView({ range, events, ctx, listName }: Props) {
  const groups = agendaGroups(events, range)
  if (groups.length === 0) return <div className="flex flex-1 items-center justify-center text-grey">No tasks in this period</div>
  return (
    <div className="min-h-0 flex-1 overflow-y-auto" role="list" aria-label="Agenda">
      {groups.map(({ day, events: list }) => {
        const rel = diffDays(ctx.now, day)
        return (
          <section key={day} role="listitem" aria-label={shortDate(day)}>
            <h2 className="sticky top-0 z-10 flex items-baseline gap-2 border-b border-line bg-surface px-4 py-2 font-semibold">
              <span className={rel === 0 ? 'text-primary' : ''}>{rel === 0 ? 'Today' : rel === 1 ? 'Tomorrow' : shortDate(day)}</span>
              {(rel === 0 || rel === 1) && <span className="text-s font-normal text-grey">{shortDate(day)}</span>}
            </h2>
            <ul>
              {list.map((e) => (
                <li key={e.task.id}>
                  <button
                    type="button"
                    onClick={(ev) => ctx.openTask(e.task, ev.currentTarget)}
                    className="flex w-full items-center gap-3 px-4 py-2 text-left outline-none hover:bg-hover focus-visible:bg-hover focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
                  >
                    <span aria-hidden className="h-4 w-1 shrink-0 rounded-sm" style={{ backgroundColor: ctx.colorOf(e.task) }} />
                    <span className="w-16 shrink-0 text-s text-grey">{timeText(e, day, ctx.hour12)}</span>
                    <span className={`min-w-0 flex-1 truncate ${e.done ? 'text-grey line-through' : ''}`}>{e.task.title}</span>
                    <span className="max-w-[30%] shrink-0 truncate text-s text-grey">{listName(e.task.listId)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )
      })}
    </div>
  )
}
