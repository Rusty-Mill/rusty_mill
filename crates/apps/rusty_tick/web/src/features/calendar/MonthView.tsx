import { useState } from 'react'
import { addDays, diffDays } from '@/lib/date'
import type { CalendarCtx } from './context'
import { beginDrag, drag, endDrag } from './dnd'
import { EventBar } from './EventBar'
import { cellLabel, chunkWeeks, coversDay, hiddenPerColumn, moveByDays, weekdayShort, weekSegments, type CalEvent, type Range } from './layout'

/** Bars shown per cell before "+N more". */
export const MAX_LANES = 3

interface Props {
  range: Range
  /** Any day of the month being shown; days outside it are greyed. */
  anchor: number
  events: CalEvent[]
  ctx: CalendarCtx
}

/** Six week rows under a weekday header; bars are laid over the cells so multi-day tasks span them. */
export function MonthView({ range, anchor, events, ctx }: Props) {
  const month = new Date(anchor).getMonth()
  const [focusDay, setFocusDay] = useState(() => range.days.find((d) => diffDays(d, anchor) === 0) ?? range.days[0])
  const [over, setOver] = useState<number | null>(null)
  const today = ctx.now

  const onKeyDown = (e: React.KeyboardEvent, day: number): void => {
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key]
    if (step === undefined) return
    const next = addDays(day, step)
    const el = (e.currentTarget.closest('[role=grid]') as HTMLElement | null)?.querySelector<HTMLElement>(`[data-day="${next}"]`)
    if (!el) return
    e.preventDefault()
    setFocusDay(next)
    el.focus()
  }

  return (
    <div role="grid" aria-label="Month" className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div role="row" className="sticky top-0 z-20 grid shrink-0 grid-cols-7 border-b border-line bg-surface">
        {range.days.slice(0, 7).map((d) => (
          <div key={d} role="columnheader" className="px-2 py-1.5 text-right text-s font-semibold text-grey">
            {weekdayShort(new Date(d).getDay())}
          </div>
        ))}
      </div>
      {chunkWeeks(range.days).map((week) => {
        const { segments } = weekSegments(events, week)
        const hidden = hiddenPerColumn(segments, week.length, MAX_LANES)
        return (
          <div
            key={week[0]}
            role="row"
            className="relative grid min-h-[112px] flex-1 grid-cols-7 border-b border-line"
            style={{ gridTemplateRows: `28px repeat(${MAX_LANES}, 22px) minmax(0, 1fr)` }}
          >
            {week.map((day, i) => {
              const d = new Date(day)
              const isToday = diffDays(day, today) === 0
              const count = events.filter((e) => coversDay(e, day)).length
              const outside = d.getMonth() !== month
              return (
                <div
                  key={day}
                  role="gridcell"
                  data-day={day}
                  aria-label={cellLabel(day, count)}
                  aria-selected={isToday || undefined}
                  tabIndex={day === focusDay ? 0 : -1}
                  style={{ gridColumn: i + 1, gridRow: '1 / -1' }}
                  onFocus={() => setFocusDay(day)}
                  onKeyDown={(e) => {
                    if (e.target !== e.currentTarget) return
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault()
                      ctx.openAdd({ ms: day, allDay: true }, e.currentTarget)
                    } else onKeyDown(e, day)
                  }}
                  onClick={(e) => ctx.openAdd({ ms: day, allDay: true }, e.currentTarget)}
                  onDragOver={(e) => {
                    if (!drag.current) return
                    e.preventDefault()
                    setOver(day)
                  }}
                  onDragLeave={() => setOver((o) => (o === day ? null : o))}
                  onDrop={(e) => {
                    e.preventDefault()
                    setOver(null)
                    const info = drag.current
                    endDrag()
                    if (!info) return
                    const delta = diffDays(info.grabDay, day)
                    if (delta !== 0) ctx.move(info.task, moveByDays(info.task, delta))
                  }}
                  className={`cursor-default border-l border-line outline-none first:border-l-0 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary ${over === day ? 'bg-selected' : ''}`}
                >
                  <span
                    aria-hidden
                    className={`m-1 flex h-6 min-w-6 items-center justify-center rounded-full px-1 text-s ${isToday ? 'bg-primary font-semibold text-white' : outside ? 'text-grey' : 'text-text'} ${d.getDate() === 1 && !isToday ? 'font-semibold' : ''}`}
                    style={{ width: 'fit-content', marginLeft: 'auto' }}
                  >
                    {d.getDate() === 1 ? `${new Date(day).toLocaleString('en-US', { month: 'short' })} 1` : d.getDate()}
                  </span>
                </div>
              )
            })}
            {segments
              .filter((s) => s.lane < MAX_LANES)
              .map((s) => (
                <div
                  key={`${s.event.task.id}-${s.startCol}`}
                  className="pointer-events-none z-10 flex items-center px-0.5"
                  style={{ gridColumn: `${s.startCol + 1} / ${s.endCol + 2}`, gridRow: s.lane + 2 }}
                >
                  <EventBar
                    event={s.event}
                    color={ctx.colorOf(s.event.task)}
                    hour12={ctx.hour12}
                    cutStart={s.cutStart}
                    cutEnd={s.cutEnd}
                    onOpen={(el) => ctx.openTask(s.event.task, el)}
                    onDragStart={(e) => {
                      const row = e.currentTarget.closest('[role=row]')!.getBoundingClientRect()
                      const col = Math.max(0, Math.min(week.length - 1, Math.floor(((e.clientX - row.left) / row.width) * week.length)))
                      beginDrag(e, { task: s.event.task, grabDay: week[col]!, startDay: s.event.startDay, grabMinutes: 0 })
                    }}
                    style={s.event.done ? { opacity: 0.7 } : undefined}
                  />
                </div>
              ))}
            {hidden.map((n, i) =>
              n > 0 ? (
                <div key={`more-${i}`} className="pointer-events-none z-10 px-0.5" style={{ gridColumn: i + 1, gridRow: MAX_LANES + 2 }}>
                  <button
                    type="button"
                    onClick={(e) => ctx.openDay(week[i]!, e.currentTarget)}
                    className="pointer-events-auto rounded px-1.5 text-s text-grey outline-none hover:bg-hover hover:text-text focus-visible:ring-2 focus-visible:ring-primary"
                  >
                    +{n} more
                  </button>
                </div>
              ) : null,
            )}
          </div>
        )
      })}
    </div>
  )
}
