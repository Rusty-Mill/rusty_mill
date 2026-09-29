import { useEffect, useRef, useState } from 'react'
import { addDays, diffDays, formatTime, atTime } from '@/lib/date'
import type { CalendarCtx } from './context'
import { beginDrag, drag, endDrag, rectAnchor } from './dnd'
import { EventBar, wash } from './EventBar'
import { cellLabel, eventWhen, layoutTimed, minutesOfDay, moveToAllDay, moveToSlot, slotTime, weekdayShort, weekSegments, MINUTES_PER_DAY, type CalEvent, type Range } from './layout'

const HOUR_PX = 48
const GUTTER = 56

interface Props {
  range: Range
  events: CalEvent[]
  ctx: CalendarCtx
}

const hourLabel = (h: number, hour12: boolean): string => (hour12 ? `${h % 12 || 12} ${h < 12 ? 'AM' : 'PM'}` : `${String(h).padStart(2, '0')}:00`)

/** Week and Day: an all-day strip above a 24-hour grid, both sharing the same day columns. */
export function TimeGridView({ range, events, ctx }: Props) {
  const { days } = range
  const cols = days.length
  const scroller = useRef<HTMLDivElement>(null)
  const [over, setOver] = useState<number | null>(null)

  // Open near the working day, or near now when it is earlier than that.
  useEffect(() => {
    const el = scroller.current
    if (!el) return
    const showsToday = days.some((d) => diffDays(d, ctx.now) === 0)
    const minute = showsToday ? Math.min(minutesOfDay(ctx.now) - 60, 7 * 60) : 7 * 60
    el.scrollTop = Math.max(0, (minute / 60) * HOUR_PX - 12) // a little of the hour above, so its label clears the sticky header
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const { segments, lanes } = weekSegments(
    events.filter((e) => e.kind === 'span'),
    days,
  )
  const gridCols = { gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }

  const dropAllDay = (e: React.DragEvent, day: number): void => {
    e.preventDefault()
    setOver(null)
    const info = drag.current
    endDrag()
    if (!info) return
    ctx.move(info.task, moveToAllDay(info.task, addDays(info.startDay, diffDays(info.grabDay, day))))
  }

  const dropSlot = (e: React.DragEvent<HTMLElement>, day: number): void => {
    e.preventDefault()
    setOver(null)
    const info = drag.current
    endDrag()
    if (!info) return
    const rect = e.currentTarget.getBoundingClientRect()
    const minutes = ((e.clientY - rect.top) / HOUR_PX) * 60 - info.grabMinutes
    ctx.move(info.task, moveToSlot(info.task, slotTime(day, minutes)))
  }

  const openSlot = (e: React.MouseEvent<HTMLElement>, day: number): void => {
    const rect = e.currentTarget.getBoundingClientRect()
    const ms = slotTime(day, ((e.clientY - rect.top) / HOUR_PX) * 60, 30)
    const top = rect.top + (minutesOfDay(ms) / 60) * HOUR_PX
    ctx.openAdd({ ms, allDay: false }, rectAnchor(new DOMRect(rect.left, top, rect.width, HOUR_PX / 2)))
  }

  return (
    <div ref={scroller} role="grid" aria-label={cols === 1 ? 'Day' : 'Week'} className="min-h-0 flex-1 overflow-y-auto [scrollbar-gutter:stable]">
      <div className="sticky top-0 z-30 border-b border-line bg-surface">
        <div role="row" className="flex">
          <div style={{ width: GUTTER }} className="shrink-0" />
          <div className="grid flex-1" style={gridCols}>
            {days.map((day) => {
              const isToday = diffDays(day, ctx.now) === 0
              return (
                <div key={day} role="columnheader" aria-label={cellLabel(day, events.filter((e) => e.startDay <= day && day <= e.endDay).length)} className="flex flex-col items-center py-1.5">
                  <span className={`text-s ${isToday ? 'text-primary' : 'text-grey'}`}>{weekdayShort(new Date(day).getDay())}</span>
                  <span className={`flex h-7 min-w-7 items-center justify-center rounded-full px-1 text-title ${isToday ? 'bg-primary font-semibold text-white' : ''}`}>{new Date(day).getDate()}</span>
                </div>
              )
            })}
          </div>
        </div>
        <div role="row" className="flex border-t border-line">
          <div style={{ width: GUTTER }} className="flex shrink-0 items-start justify-end pr-2 pt-1.5 text-s text-grey">
            all-day
          </div>
          <div className="relative grid min-h-[28px] flex-1 py-0.5" style={{ ...gridCols, gridTemplateRows: `repeat(${Math.max(lanes, 1)}, 22px)` }}>
            {days.map((day, i) => (
              <div
                key={day}
                role="gridcell"
                aria-label={`All-day, ${cellLabel(day, events.filter((e) => e.kind === 'span' && e.startDay <= day && day <= e.endDay).length)}`}
                tabIndex={0}
                style={{ gridColumn: i + 1, gridRow: '1 / -1' }}
                onClick={(e) => ctx.openAdd({ ms: day, allDay: true }, e.currentTarget)}
                onKeyDown={(e) => {
                  if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
                    e.preventDefault()
                    ctx.openAdd({ ms: day, allDay: true }, e.currentTarget)
                  }
                }}
                onDragOver={(e) => {
                  if (!drag.current) return
                  e.preventDefault()
                  setOver(-day)
                }}
                onDragLeave={() => setOver((o) => (o === -day ? null : o))}
                onDrop={(e) => dropAllDay(e, day)}
                className={`border-l border-line outline-none first:border-l-0 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary ${over === -day ? 'bg-selected' : ''}`}
              />
            ))}
            {segments.map((s) => (
              <div key={`${s.event.task.id}-${s.startCol}`} className="pointer-events-none z-10 flex items-center px-0.5" style={{ gridColumn: `${s.startCol + 1} / ${s.endCol + 2}`, gridRow: s.lane + 1 }}>
                <EventBar
                  event={s.event}
                  color={ctx.colorOf(s.event.task)}
                  hour12={ctx.hour12}
                  cutStart={s.cutStart}
                  cutEnd={s.cutEnd}
                  onOpen={(el) => ctx.openTask(s.event.task, el)}
                  onDragStart={(e) => {
                    const strip = e.currentTarget.parentElement!.parentElement!.getBoundingClientRect()
                    const col = Math.max(0, Math.min(cols - 1, Math.floor(((e.clientX - strip.left) / strip.width) * cols)))
                    beginDrag(e, { task: s.event.task, grabDay: days[col]!, startDay: s.event.startDay, grabMinutes: 0 })
                  }}
                  style={s.event.done ? { opacity: 0.7 } : undefined}
                />
              </div>
            ))}
          </div>
        </div>
      </div>

      <div role="row" className="flex" style={{ height: 24 * HOUR_PX }}>
        <div style={{ width: GUTTER }} className="relative shrink-0" aria-hidden>
          {Array.from({ length: 23 }, (_, i) => (
            <span key={i} style={{ top: (i + 1) * HOUR_PX - 9 }} className="absolute right-2 text-s text-grey">
              {hourLabel(i + 1, ctx.hour12)}
            </span>
          ))}
        </div>
        <div className="grid flex-1" style={gridCols}>
          {days.map((day) => {
            const isToday = diffDays(day, ctx.now) === 0
            const blocks = layoutTimed(events, day)
            const slot = ctx.addSlot && !ctx.addSlot.allDay && diffDays(day, ctx.addSlot.ms) === 0 ? ctx.addSlot.ms : null
            return (
              <div
                key={day}
                role="gridcell"
                aria-label={cellLabel(day, blocks.length)}
                tabIndex={0}
                onClick={(e) => openSlot(e, day)}
                onKeyDown={(e) => {
                  if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
                    e.preventDefault()
                    const rect = e.currentTarget.getBoundingClientRect()
                    const ms = isToday ? slotTime(day, (Math.floor(minutesOfDay(ctx.now) / 60) + 1) * 60) : atTime(day, 9)
                    const top = rect.top + (minutesOfDay(ms) / 60) * HOUR_PX
                    ctx.openAdd({ ms, allDay: false }, rectAnchor(new DOMRect(rect.left, top, rect.width, HOUR_PX / 2)))
                  }
                }}
                onDragOver={(e) => {
                  if (!drag.current) return
                  e.preventDefault()
                  setOver(day)
                }}
                onDragLeave={() => setOver((o) => (o === day ? null : o))}
                onDrop={(e) => dropSlot(e, day)}
                style={{ backgroundImage: 'linear-gradient(to bottom, rgb(var(--line)) 1px, transparent 1px)', backgroundSize: `100% ${HOUR_PX}px` }}
                className={`relative border-l border-line outline-none first:border-l-0 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary ${over === day ? 'bg-selected/40' : ''}`}
              >
                {slot !== null && (
                  <div className="pointer-events-none absolute inset-x-0.5 rounded bg-primary/20 ring-1 ring-primary" style={{ top: `${(minutesOfDay(slot) / MINUTES_PER_DAY) * 100}%`, height: HOUR_PX / 2 }} />
                )}
                {blocks.map((b) => {
                  const color = ctx.colorOf(b.event.task)
                  const tall = ((b.endMin - b.startMin) / 60) * HOUR_PX >= 40
                  return (
                    <button
                      key={b.event.task.id}
                      type="button"
                      draggable
                      data-task-id={b.event.task.id}
                      aria-label={`${b.event.task.title}, ${eventWhen(b.event, ctx.hour12)}`}
                      onClick={(e) => {
                        e.stopPropagation()
                        ctx.openTask(b.event.task, e.currentTarget)
                      }}
                      onDragStart={(e) => {
                        const rect = e.currentTarget.getBoundingClientRect()
                        beginDrag(e, { task: b.event.task, grabDay: day, startDay: day, grabMinutes: ((e.clientY - rect.top) / HOUR_PX) * 60 })
                      }}
                      style={{
                        top: `${b.top}%`,
                        height: `${b.height}%`,
                        left: `${(b.col / b.cols) * 100}%`,
                        width: `${100 / b.cols}%`,
                        minHeight: 18,
                        backgroundColor: wash(color, b.event.done ? 8 : 22),
                        borderLeftColor: color,
                        opacity: b.event.done ? 0.7 : 1,
                      }}
                      className={`absolute z-10 flex flex-col overflow-hidden rounded-[4px] border-l-[3px] px-1.5 py-0.5 text-left text-s leading-4 outline-none ring-1 ring-surface focus-visible:ring-2 focus-visible:ring-primary ${b.event.done ? 'text-grey line-through' : 'text-text'}`}
                    >
                      <span className="truncate font-semibold">{b.event.task.title}</span>
                      {tall && <span className="truncate text-grey">{formatTime(b.event.startMs, ctx.hour12)}</span>}
                    </button>
                  )
                })}
                {isToday && (
                  <div aria-hidden className="pointer-events-none absolute inset-x-0 z-20 h-0.5 bg-danger" style={{ top: `${(minutesOfDay(ctx.now) / MINUTES_PER_DAY) * 100}%` }}>
                    <span className="absolute -left-1 -top-[3px] h-2 w-2 rounded-full bg-danger" />
                  </div>
                )}
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
