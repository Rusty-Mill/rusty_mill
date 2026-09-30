import { diffDays, formatTime, startOfDay } from '@/lib/date'
import type { CalendarCtx } from './context'
import { agendaGroups, overdueEvents, shortDate, type CalEvent, type Range } from './layout'

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

function Row({ e, day, ctx, listName }: { e: CalEvent; day: number; ctx: CalendarCtx; listName: (listId: string) => string }) {
  return (
    <li>
      <button
        type="button"
        onClick={(ev) => ctx.openTask(e.task, ev.currentTarget)}
        className="flex w-full items-center gap-3 px-4 py-2 text-left outline-none hover:bg-hover focus-visible:bg-hover focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
      >
        <span aria-hidden className="h-4 w-1 shrink-0 rounded-sm" style={{ backgroundColor: ctx.colorOf(e.task) }} />
        <span className="w-16 shrink-0 text-s text-grey">{day < 0 ? shortDate(e.endDay) : timeText(e, day, ctx.hour12)}</span>
        <span className={`min-w-0 flex-1 truncate ${e.done ? 'text-grey line-through' : ''}`}>{e.task.title}</span>
        <span className="max-w-[30%] shrink-0 truncate text-s text-grey">{listName(e.task.listId)}</span>
      </button>
    </li>
  )
}

/** Upcoming tasks grouped by day, days with none left out; overdue ones first. */
export function AgendaView({ range, events, ctx, listName }: Props) {
  const groups = agendaGroups(events, range)
  const overdue = overdueEvents(events, startOfDay(ctx.now))
  if (groups.length === 0 && overdue.length === 0) return <div className="flex flex-1 items-center justify-center text-grey">No tasks in this period</div>
  return (
    <div className="min-h-0 flex-1 overflow-y-auto" role="list" aria-label="Agenda">
      {overdue.length > 0 && (
        <section role="listitem" aria-label="Overdue">
          <h2 className="sticky top-0 z-10 border-b border-line bg-surface px-4 py-2 font-semibold text-danger">Overdue</h2>
          <ul>
            {overdue.map((e) => (
              <Row key={e.task.id} e={e} day={-1} ctx={ctx} listName={listName} />
            ))}
          </ul>
        </section>
      )}
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
                <Row key={e.task.id} e={e} day={day} ctx={ctx} listName={listName} />
              ))}
            </ul>
          </section>
        )
      })}
    </div>
  )
}
