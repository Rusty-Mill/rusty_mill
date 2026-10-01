import { diffDays, formatTime, startOfDay } from '@/lib/date'
import type { CalendarCtx } from './context'
import { wash } from './EventBar'
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

function Row({ e, day, ctx, listName, first }: { e: CalEvent; day: number; ctx: CalendarCtx; listName: (listId: string) => string; first?: boolean }) {
  const color = ctx.colorOf(e.task)
  const when = day < 0 ? shortDate(e.endDay) : timeText(e, day, ctx.hour12)
  return (
    <li className="grid grid-cols-[84px_28px_1fr] items-stretch gap-x-1 py-1.5">
      <span className="pt-2 text-right text-s text-grey">{when}</span>
      <span aria-hidden className="relative flex justify-center">
        <span className="absolute inset-y-[-6px] w-px bg-line" style={first ? { top: 12 } : undefined} />
        <span className="relative mt-3 h-2.5 w-2.5 rounded-full border-[1.5px] bg-surface" style={{ borderColor: color }} />
      </span>
      <button
        type="button"
        onClick={(ev) => ctx.openTask(e.task, ev.currentTarget)}
        className="flex min-w-0 flex-col rounded-row border-l-[3px] px-3 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
        style={{ backgroundColor: wash(color, 10), borderLeftColor: color }}
      >
        <span className="text-s text-primary">{e.kind === 'timed' ? `${formatTime(e.startMs, ctx.hour12)} - ${formatTime(e.endMs, ctx.hour12)}` : e.startDay === e.endDay ? 'All day' : `${shortDate(e.startDay)} - ${shortDate(e.endDay)}`}</span>
        <span className={`truncate ${e.done ? 'text-grey line-through' : 'font-medium'}`}>{e.task.title}</span>
        <span className="sr-only">{listName(e.task.listId)}</span>
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
        const d = new Date(day)
        return (
          <section key={day} role="listitem" aria-label={shortDate(day)} className="grid grid-cols-[92px_1fr] px-6 py-2">
            <h2 className="flex items-baseline gap-1.5 pt-3 font-semibold">
              <span className={`text-title ${rel === 0 ? 'text-primary' : ''}`}>{d.getDate()}</span>
              <span className={`text-s font-normal ${rel === 0 ? 'text-primary' : 'text-grey'}`}>{rel === 0 ? 'Today' : rel === 1 ? 'Tomorrow' : d.toLocaleDateString('en-US', { weekday: 'short' })}</span>
            </h2>
            <ul>
              {list.map((e, i) => (
                <Row key={e.task.id} e={e} day={day} ctx={ctx} listName={listName} first={i === 0} />
              ))}
            </ul>
          </section>
        )
      })}
    </div>
  )
}
