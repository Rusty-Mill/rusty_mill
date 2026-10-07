import { Popover } from '@/components/Popover'
import { barTime, eventsOnDay, longDate, type CalEvent } from './layout'

interface Props {
  anchor: HTMLElement | null
  day: number
  events: CalEvent[]
  colorOf: (e: CalEvent) => string
  hour12: boolean
  onPick: (e: CalEvent, anchor: HTMLElement | null) => void
  onClose: () => void
}

/** The "+N more" list: everything on one day. */
export function DayPopover({ anchor, day, events, colorOf, hour12, onPick, onClose }: Props) {
  const list = eventsOnDay(events, day)
  return (
    <Popover anchor={anchor} open onClose={onClose} role="dialog" ariaLabel={longDate(day)} className="w-[260px] py-2">
      <h2 className="px-3 pb-1.5 font-semibold">{longDate(day)}</h2>
      <ul className="max-h-[300px] overflow-y-auto px-1.5">
        {list.map((e) => (
          <li key={e.task.id}>
            <button type="button" onClick={() => onPick(e, anchor)} className="flex w-full items-center gap-2 rounded-row px-1.5 py-1 text-left outline-hidden hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary">
              <span aria-hidden className="h-4 w-1 shrink-0 rounded-xs" style={{ backgroundColor: colorOf(e) }} />
              <span className={`min-w-0 flex-1 truncate ${e.done ? 'text-grey line-through' : ''}`}>{e.task.title}</span>
              <span className="shrink-0 text-s text-grey">{barTime(e, hour12)}</span>
            </button>
          </li>
        ))}
      </ul>
    </Popover>
  )
}
