import { ChevronDown, ChevronLeft, ChevronRight, LayoutGrid, MoreHorizontal, Plus } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useActions, useServices } from '@/app/services'
import { Confirm } from '@/components/Confirm'
import { HabitArt } from '@/components/Illustrations'
import { addDays, monthName, startOfDay } from '@/lib/date'
import { usePrefs } from '../settings/prefs'
import { HabitDialog } from './HabitDialog'
import { HabitRow, ROW_GRID } from './HabitRow'
import { weekDays, type Habit } from './logic'
import { useHabits } from './store'

const LETTERS = ['S', 'M', 'T', 'W', 'T', 'F', 'S']

/** The seven days ending today, each with an empty hatched circle: what a habit's check-in row will look like. */
function DayStrip({ today }: { today: number }) {
  const days = Array.from({ length: 7 }, (_, i) => addDays(today, i - 6))
  return (
    <ol aria-label="Last seven days" className="mx-6 mt-2 grid grid-cols-7">
      {days.map((d) => {
        const isToday = d === today
        return (
          <li key={d} className="flex flex-col items-center gap-1 text-s">
            <span className="text-grey">{new Date(d).toLocaleDateString('en-US', { weekday: 'short' })}</span>
            <span className={isToday ? 'font-semibold text-primary' : 'font-semibold text-grey'}>{new Date(d).getDate()}</span>
            <svg width="24" height="24" viewBox="0 0 24 24" aria-hidden>
              <defs>
                <pattern id="hatch" width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                  <line x1="0" y1="0" x2="0" y2="4" stroke="rgb(var(--line))" strokeWidth="2" />
                </pattern>
              </defs>
              <circle cx="12" cy="12" r="10" fill="url(#hatch)" stroke="rgb(var(--line))" />
            </svg>
          </li>
        )
      })}
    </ol>
  )
}

export function HabitsPage() {
  const { api } = useServices()
  const { notify } = useActions()
  const weekStart = usePrefs((s) => s.prefs.weekStart)
  const habits = useHabits((s) => s.habits)
  const checkins = useHabits((s) => s.checkins)
  const [offset, setOffset] = useState(0) // weeks from the current one
  const [dialog, setDialog] = useState<{ habit: Habit | null } | null>(null)
  const [deleting, setDeleting] = useState<Habit | null>(null)
  const today = startOfDay(Date.now())

  useEffect(() => {
    void useHabits.getState().load(api, notify)
  }, [api, notify])

  const days = useMemo(() => weekDays(addDays(today, offset * 7), weekStart), [today, offset, weekStart])
  const visible = useMemo(() => habits.filter((h) => !h.archived), [habits])
  const countsByHabit = useMemo(() => {
    const map = new Map<string, Map<string, number>>()
    for (const c of Object.values(checkins)) {
      const days = map.get(c.habitId) ?? new Map<string, number>()
      days.set(c.day, c.count)
      map.set(c.habitId, days)
    }
    return map
  }, [checkins])

  const first = new Date(days[0]!)
  const last = new Date(days[6]!)
  const range = `${monthName(first.getMonth())} ${first.getDate()} – ${first.getMonth() === last.getMonth() ? '' : `${monthName(last.getMonth())} `}${last.getDate()}, ${last.getFullYear()}`
  const submit = (input: Parameters<ReturnType<typeof useHabits.getState>['add']>[0]): void => {
    const target = dialog?.habit
    if (target) useHabits.getState().update(target.id, input)
    else useHabits.getState().add(input)
    setDialog(null)
  }

  return (
    <main className="flex min-w-0 flex-1 flex-col">
      <header className="flex h-14 shrink-0 items-center gap-3 px-4">
        <h1 className="flex items-center gap-1 text-h1 font-semibold">
          Habit <ChevronDown size={16} aria-hidden className="text-grey" />
        </h1>
        <div className="ml-auto flex items-center gap-1">
          <button type="button" aria-label="Previous week" onClick={() => setOffset((o) => o - 1)} className={iconBtn}>
            <ChevronLeft size={20} strokeWidth={1.5} />
          </button>
          <span className="min-w-[150px] text-center" aria-live="polite">
            {range}
          </span>
          <button type="button" aria-label="Next week" onClick={() => setOffset((o) => o + 1)} className={iconBtn}>
            <ChevronRight size={20} strokeWidth={1.5} />
          </button>
          <button type="button" disabled={offset === 0} onClick={() => setOffset(0)} className="ml-1 h-8 rounded-row border border-line px-3 hover:bg-hover disabled:opacity-40">
            This week
          </button>
          <button type="button" disabled aria-label="Habit sections" title="Sections are not available yet" className={`${iconBtn} ml-2 opacity-50`}>
            <LayoutGrid size={20} strokeWidth={1.5} />
          </button>
          {visible.length > 0 && (
            <button type="button" aria-label="Add habit" onClick={() => setDialog({ habit: null })} className={iconBtn}>
              <Plus size={20} strokeWidth={1.5} />
            </button>
          )}
          <button type="button" disabled aria-label="More" className={`${iconBtn} opacity-50`}>
            <MoreHorizontal size={20} strokeWidth={1.5} />
          </button>
        </div>
      </header>

      {visible.length === 0 && <DayStrip today={today} />}
      {visible.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 pb-16 text-center">
          <HabitArt />
          <h2 className="text-title font-semibold">Develop a habit</h2>
          <p className="text-grey">Every little bit counts</p>
          <button type="button" onClick={() => setDialog({ habit: null })} className="mt-3 h-9 rounded-row bg-primary px-5 text-white">
            Add habit
          </button>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col overflow-x-auto">
          <div className="min-w-[640px]">
            <div className={`${ROW_GRID} border-b border-line py-2 text-s`} role="row" aria-label="Days of the week">
              <span />
              {days.map((d) => {
                const isToday = d === today
                return (
                  <div key={d} className={`flex flex-col items-center leading-tight ${isToday ? 'text-primary' : 'text-grey'}`} aria-current={isToday ? 'date' : undefined}>
                    <span>{LETTERS[new Date(d).getDay()]}</span>
                    <span className={`mt-0.5 flex h-6 w-6 items-center justify-center rounded-full text-base ${isToday ? 'bg-primary font-semibold text-white' : 'text-text'}`}>{new Date(d).getDate()}</span>
                  </div>
                )
              })}
              <span className="text-center">Streak</span>
              <span />
            </div>
            <ul className="overflow-y-auto" aria-label="Habits">
              {visible.map((h) => (
                <HabitRow
                  key={h.id}
                  habit={h}
                  days={days}
                  counts={countsByHabit.get(h.id) ?? EMPTY}
                  today={today}
                  weekStart={weekStart}
                  onToggle={(day) => useHabits.getState().toggle(h.id, day)}
                  onEdit={() => setDialog({ habit: h })}
                  onDelete={() => setDeleting(h)}
                />
              ))}
            </ul>
          </div>
        </div>
      )}

      <HabitDialog open={dialog !== null} habit={dialog?.habit ?? null} onClose={() => setDialog(null)} onSubmit={submit} />
      <Confirm
        open={deleting !== null}
        title="Delete habit?"
        message={`"${deleting?.name ?? ''}" and all of its check-ins will be deleted.`}
        confirmLabel="Delete"
        danger
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          if (deleting) useHabits.getState().remove(deleting.id)
          setDeleting(null)
        }}
      />
    </main>
  )
}

const EMPTY: ReadonlyMap<string, number> = new Map()
const iconBtn = 'flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary'
