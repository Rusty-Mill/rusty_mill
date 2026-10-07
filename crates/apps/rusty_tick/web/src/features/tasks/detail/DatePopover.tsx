import { Bell, CalendarRange, ChevronLeft, ChevronRight, Clock, Moon, Repeat, Sun, Sunrise } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Popover } from '@/components/Popover'
import { addDays, addMonths, dayKey, monthGrid, monthName, startOfDay, weekdayName, type WeekStart } from '@/lib/date'
import { describeRule } from '@/lib/recurrence'
import { usePrefs } from '../../settings/prefs'
import {
  CLEARED,
  presetOf,
  quickDates,
  reminderOptions,
  repeatRule,
  selectionOf,
  toFields,
  type DateFields,
  type RepeatPreset,
  type Selection,
} from '../dateSelection'

interface Props {
  anchor: HTMLElement | null
  open: boolean
  onClose: () => void
  fields: DateFields
  now: number
  onApply: (fields: DateFields) => void
}

const QUICK_ICONS = { today: Sun, tomorrow: Sunrise, nextWeek: CalendarRange, evening: Moon } as const

const pad = (n: number): string => String(n).padStart(2, '0')
const timeValue = (t: { h: number; m: number } | null): string => (t ? `${pad(t.h)}:${pad(t.m)}` : '')
const parseTime = (v: string): { h: number; m: number } | null => {
  const m = /^(\d{2}):(\d{2})$/.exec(v)
  return m ? { h: Number(m[1]), m: Number(m[2]) } : null
}

/**
 * The due-date popover: Date / Duration tabs, the four quick buttons, a month
 * grid, and rows for time, reminder and repeat. Nothing is saved until OK.
 */
export function DatePopover(props: Props) {
  const { anchor, open, onClose } = props
  return (
    <Popover anchor={anchor} open={open} onClose={onClose} placement="bottom-start" ariaLabel="Due date" role="dialog" className="w-[320px] p-3">
      {/* Mounted only while open, so it always starts from the task's current values. */}
      {open && <PopoverBody {...props} />}
    </Popover>
  )
}

function PopoverBody({ onClose, fields, now, onApply }: Props) {
  const prefs = usePrefs((s) => s.prefs)
  const [sel, setSel] = useState<Selection>(() => selectionOf(fields))
  const [tab, setTab] = useState<'date' | 'duration'>(fields.startMs !== null ? 'duration' : 'date')
  const [target, setTarget] = useState<'due' | 'start'>('due')
  const [shown, setShown] = useState(() => startOfDay(sel.day ?? now))
  const [customOpen, setCustomOpen] = useState(false)
  const [custom, setCustom] = useState<{ freq: 'DAILY' | 'WEEKLY' | 'MONTHLY' | 'YEARLY'; interval: number }>({ freq: 'DAILY', interval: 2 })

  const today = startOfDay(now)
  const grid = useMemo(() => monthGrid(shown, prefs.weekStart), [shown, prefs.weekStart])
  const weekdays = Array.from({ length: 7 }, (_, i) => weekdayName((prefs.weekStart + i) % 7))
  const shownMonth = new Date(shown).getMonth()

  const pickDay = (day: number): void => {
    setSel((s) => {
      if (tab === 'duration' && target === 'start') return { ...s, startDay: day, day: s.day !== null && s.day < day ? day : s.day ?? day }
      // The due day: a start after it would be nonsense, so pull the start back.
      return { ...s, day, startDay: tab === 'duration' && s.startDay !== null && s.startDay > day ? day : s.startDay }
    })
  }
  const applyQuick = (q: ReturnType<typeof quickDates>[number]): void => {
    setSel((s) => ({ ...s, day: q.day, time: q.time ?? s.time, reminder: q.time && !s.reminder ? prefs.defaultReminder : s.reminder }))
    setShown(startOfDay(q.day))
  }
  const setTime = (time: { h: number; m: number } | null): void =>
    setSel((s) => ({
      ...s,
      day: s.day ?? today,
      time,
      // A time with no reminder yet gets the user's default; removing the time drops a timed reminder.
      reminder: time ? (s.reminder || (s.time ? s.reminder : prefs.defaultReminder)) : s.time ? '' : s.reminder,
    }))

  const allDay = sel.time === null
  const reminderChoices = reminderOptions(allDay)
  const reminderValid = reminderChoices.some((r) => r.value === sel.reminder)
  const preset = presetOf(sel.repeat)
  const dueAnchor = (sel.day ?? today) + (sel.time ? sel.time.h * 3_600_000 + sel.time.m * 60_000 : 0)

  const chooseRepeat = (p: RepeatPreset): void => {
    if (p === 'custom') return setCustomOpen(true)
    setCustomOpen(false)
    setSel((s) => ({ ...s, day: s.day ?? today, repeat: repeatRule(p, dueAnchor) }))
  }

  const ok = (): void => {
    onApply(toFields(sel))
    onClose()
  }
  const clear = (): void => {
    onApply(CLEARED)
    onClose()
  }

  return (
    <div className="flex flex-col gap-2.5 text-base">
      <div role="tablist" aria-label="Date type" className="grid grid-cols-2 gap-1 rounded-row bg-black/[.05] p-0.5">
        {(['date', 'duration'] as const).map((t) => (
          <button key={t} type="button" role="tab" aria-selected={tab === t} onClick={() => setTab(t)} className={`h-7 rounded-md text-base ${tab === t ? 'bg-surface font-semibold shadow-xs' : 'text-grey'}`}>
            {t === 'date' ? 'Date' : 'Duration'}
          </button>
        ))}
      </div>

      {tab === 'duration' && (
        <div role="group" aria-label="Which date to set" className="grid grid-cols-2 gap-1 text-s">
          {(['start', 'due'] as const).map((k) => {
            const day = k === 'start' ? sel.startDay : sel.day
            return (
              <button key={k} type="button" aria-pressed={target === k} onClick={() => setTarget(k)} className={`rounded-row border px-2 py-1 text-left ${target === k ? 'border-primary text-primary' : 'border-line text-grey'}`}>
                <span className="block text-[11px] uppercase">{k === 'start' ? 'Start' : 'End'}</span>
                {day === null ? 'Not set' : `${monthName(new Date(day).getMonth())} ${new Date(day).getDate()}`}
              </button>
            )
          })}
        </div>
      )}

      <div className="flex justify-between px-1" role="group" aria-label="Quick dates">
        {quickDates(now, prefs.weekStart).map((q) => {
          const Icon = QUICK_ICONS[q.id]
          return (
            <button key={q.id} type="button" title={q.label} aria-label={q.label} onClick={() => applyQuick(q)} className="flex h-9 w-9 items-center justify-center rounded-row text-grey hover:bg-hover hover:text-primary">
              <Icon size={20} strokeWidth={1.5} />
            </button>
          )
        })}
      </div>

      <div className="flex items-center justify-between px-1">
        <span className="font-semibold" aria-live="polite">
          {monthName(shownMonth)} {new Date(shown).getFullYear()}
        </span>
        <span className="flex">
          <button type="button" aria-label="Previous month" onClick={() => setShown((s) => addMonths(s, -1))} className="flex h-7 w-7 items-center justify-center rounded hover:bg-hover">
            <ChevronLeft size={16} />
          </button>
          <button type="button" aria-label="Next month" onClick={() => setShown((s) => addMonths(s, 1))} className="flex h-7 w-7 items-center justify-center rounded hover:bg-hover">
            <ChevronRight size={16} />
          </button>
        </span>
      </div>

      <DayGrid grid={grid} weekdays={weekdays} shownMonth={shownMonth} today={today} sel={sel} weekStart={prefs.weekStart} onPick={pickDay} onFocusMonth={setShown} />

      <Row icon={<Clock size={18} />} label="Time">
        <div className="flex items-center gap-1.5">
          <input
            type="time"
            aria-label="Time"
            value={timeValue(sel.time)}
            onChange={(e) => setTime(parseTime(e.target.value))}
            className="h-7 rounded border border-line bg-surface px-1.5 text-base outline-hidden focus:border-primary"
          />
          {sel.time && (
            <button type="button" onClick={() => setTime(null)} className="text-s text-grey hover:text-text">
              All day
            </button>
          )}
        </div>
      </Row>
      <Row icon={<Bell size={18} />} label="Reminder">
        <select
          aria-label="Reminder"
          value={reminderValid ? sel.reminder : ''}
          disabled={sel.day === null}
          onChange={(e) => setSel((s) => ({ ...s, reminder: e.target.value }))}
          className="h-7 max-w-[170px] rounded border border-line bg-surface px-1 text-base outline-hidden focus:border-primary disabled:opacity-40"
        >
          {reminderChoices.map((r) => (
            <option key={r.value} value={r.value}>
              {r.label}
            </option>
          ))}
        </select>
      </Row>
      <Row icon={<Repeat size={18} />} label="Repeat">
        <select
          aria-label="Repeat"
          value={preset}
          disabled={sel.day === null}
          onChange={(e) => chooseRepeat(e.target.value as RepeatPreset)}
          className="h-7 max-w-[170px] rounded border border-line bg-surface px-1 text-base outline-hidden focus:border-primary disabled:opacity-40"
        >
          <option value="none">Does not repeat</option>
          <option value="daily">Daily</option>
          <option value="weekly">Weekly</option>
          <option value="monthly">Monthly</option>
          <option value="yearly">Yearly</option>
          <option value="custom">{preset === 'custom' && !customOpen ? describeRule(sel.repeat) : 'Custom…'}</option>
        </select>
      </Row>
      {(customOpen || preset === 'custom') && sel.day !== null && (
        <div className="flex items-center gap-1.5 pl-7 text-s">
          Every
          <input
            type="number"
            min={1}
            max={999}
            aria-label="Repeat every"
            value={custom.interval}
            onChange={(e) => {
              const next = { ...custom, interval: Math.max(1, Number(e.target.value) || 1) }
              setCustom(next)
              setSel((s) => ({ ...s, repeat: repeatRule('custom', dueAnchor, next) }))
            }}
            className="h-7 w-14 rounded border border-line bg-surface px-1.5 outline-hidden focus:border-primary"
          />
          <select
            aria-label="Repeat unit"
            value={custom.freq}
            onChange={(e) => {
              const next = { ...custom, freq: e.target.value as typeof custom.freq }
              setCustom(next)
              setSel((s) => ({ ...s, repeat: repeatRule('custom', dueAnchor, next) }))
            }}
            className="h-7 rounded border border-line bg-surface px-1 outline-hidden focus:border-primary"
          >
            <option value="DAILY">days</option>
            <option value="WEEKLY">weeks</option>
            <option value="MONTHLY">months</option>
            <option value="YEARLY">years</option>
          </select>
        </div>
      )}

      <div className="flex items-center justify-between border-t border-line pt-2">
        <button type="button" onClick={clear} className="h-8 rounded-row px-3 text-grey hover:bg-hover">
          Clear
        </button>
        <button type="button" onClick={ok} data-autofocus className="h-8 rounded-row bg-primary px-5 text-white">
          OK
        </button>
      </div>
    </div>
  )
}

function Row({ icon, label, children }: { icon: React.ReactNode; label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2 px-1">
      <span className="flex items-center gap-2 text-grey">
        {icon}
        {label}
      </span>
      {children}
    </div>
  )
}

interface GridProps {
  grid: number[]
  weekdays: string[]
  shownMonth: number
  today: number
  sel: Selection
  weekStart: WeekStart
  onPick: (day: number) => void
  onFocusMonth: (day: number) => void
}

/** The month grid. Arrow keys move by a day or a week, paging the month as needed. */
function DayGrid({ grid, weekdays, shownMonth, today, sel, onPick, onFocusMonth }: GridProps) {
  const focusDay = sel.day ?? today
  const move = (from: number, days: number, el: HTMLElement): void => {
    const to = startOfDay(addDays(from, days))
    if (new Date(to).getMonth() !== shownMonth) onFocusMonth(to)
    // Focus after the grid has re-rendered for the new month.
    requestAnimationFrame(() => el.closest('[role="grid"]')?.querySelector<HTMLElement>(`[data-day="${dayKey(to)}"]`)?.focus())
  }
  const rangeStart = sel.startDay
  return (
    <div role="grid" aria-label="Calendar">
      <div role="row" className="grid grid-cols-7 text-center text-s text-grey">
        {weekdays.map((d) => (
          <span key={d} role="columnheader" className="py-1">
            {d.slice(0, 2)}
          </span>
        ))}
      </div>
      {Array.from({ length: 6 }, (_, w) => (
        <div key={w} role="row" className="grid grid-cols-7">
          {grid.slice(w * 7, w * 7 + 7).map((day) => {
            const d = new Date(day)
            const isSel = sel.day === day
            const isStart = rangeStart === day
            const inRange = rangeStart !== null && sel.day !== null && day > rangeStart && day < sel.day
            const isToday = day === today
            const outside = d.getMonth() !== shownMonth
            return (
              <button
                key={day}
                type="button"
                role="gridcell"
                data-day={dayKey(day)}
                tabIndex={day === focusDay ? 0 : -1}
                aria-selected={isSel || isStart}
                aria-label={`${weekdayName(d.getDay())}, ${monthName(d.getMonth())} ${d.getDate()}, ${d.getFullYear()}${isToday ? ', today' : ''}`}
                onClick={() => onPick(day)}
                onKeyDown={(e) => {
                  const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key]
                  if (step !== undefined) {
                    e.preventDefault()
                    move(day, step, e.currentTarget)
                  }
                }}
                className={`m-px flex h-8 items-center justify-center rounded-full text-base ${isSel || isStart ? 'bg-primary text-white' : inRange ? 'bg-primary/10' : isToday ? 'font-semibold text-primary ring-1 ring-primary/40' : outside ? 'text-grey/60 hover:bg-hover' : 'hover:bg-hover'}`}
              >
                {d.getDate()}
              </button>
            )
          })}
        </div>
      ))}
    </div>
  )
}
