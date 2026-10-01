import { useState, type FormEvent } from 'react'
import { Dialog } from '@/components/Dialog'
import { SWATCHES } from '@/lib/colors'
import { Swatch } from '../lists/ListDialog'
import { usePrefs } from '../settings/prefs'
import { formatGoal, parseGoal, weekdayOrder, type Frequency, type Habit } from './logic'
import type { HabitInput } from './store'

const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

interface Props {
  open: boolean
  onClose: () => void
  /** The habit being edited; `null` creates one. */
  habit: Habit | null
  onSubmit: (input: HabitInput) => void
}

export function HabitDialog({ open, onClose, habit, onSubmit }: Props) {
  return (
    <Dialog open={open} onClose={onClose} title={habit ? 'Edit Habit' : 'Create Habit'} width={440}>
      {/* Remount per open so the fields start from the habit being edited. */}
      {open && <HabitForm key={habit?.id ?? 'new'} habit={habit} onCancel={onClose} onSubmit={onSubmit} />}
    </Dialog>
  )
}

type Kind = Frequency['kind']

function HabitForm({ habit, onCancel, onSubmit }: { habit: Habit | null; onCancel: () => void; onSubmit: Props['onSubmit'] }) {
  const weekStart = usePrefs((s) => s.prefs.weekStart)
  const goal = parseGoal(habit?.goal ?? '')
  const f = habit?.frequency
  const [name, setName] = useState(habit?.name ?? '')
  const [count, setCount] = useState(String(goal.count))
  const [unit, setUnit] = useState(goal.unit)
  const [kind, setKind] = useState<Kind>(f?.kind ?? 'daily')
  const [days, setDays] = useState<number[]>(f?.kind === 'weekdays' ? f.days : [1, 2, 3, 4, 5])
  const [times, setTimes] = useState(String(f?.kind === 'perWeek' ? f.times : 3))
  const [color, setColor] = useState(habit?.color ?? SWATCHES[5])
  const [reminder, setReminder] = useState(habit?.reminder ?? '')

  const goalCount = Math.min(9999, Math.max(1, Math.round(Number(count)) || 1))
  const timesNum = Math.min(7, Math.max(1, Math.round(Number(times)) || 1))
  const valid = name.trim().length > 0 && (kind !== 'weekdays' || days.length > 0)

  const submit = (e: FormEvent): void => {
    e.preventDefault()
    if (!valid) return
    const frequency: Frequency = kind === 'daily' ? { kind } : kind === 'weekdays' ? { kind, days: [...days].sort((a, b) => a - b) } : { kind, times: timesNum }
    onSubmit({ name: name.trim(), color, goal: formatGoal(goalCount, unit), frequency, reminder: reminder || null })
  }

  const toggleDay = (d: number): void => setDays((cur) => (cur.includes(d) ? cur.filter((x) => x !== d) : [...cur, d]))
  const input = 'h-9 rounded-row border border-line bg-surface px-3 outline-none focus:border-primary'

  const field = 'grid grid-cols-[104px_1fr] items-center gap-x-3'
  const label = 'text-base'
  const disabledSelect = `${input} w-full appearance-none bg-surface text-text opacity-70`
  const today = new Date()

  return (
    <form onSubmit={submit} className="flex flex-col gap-3 overflow-y-auto px-6 pb-6 pt-2">
      <div className={field}>
        <span aria-hidden className="flex h-10 w-10 items-center justify-center rounded-full text-base font-semibold text-white" style={{ backgroundColor: color }}>
          {name.trim().charAt(0).toUpperCase() || '☺'}
        </span>
        <input data-autofocus aria-label="Name" value={name} maxLength={100} onChange={(e) => setName(e.target.value)} placeholder="Habit name" className={input} />
      </div>

      <label className={field}>
        <span className={label}>Frequency</span>
        <select value={kind} onChange={(e) => setKind(e.target.value as Kind)} className={`${input} w-full`}>
          <option value="daily">Daily</option>
          <option value="weekdays">Specific days</option>
          <option value="perWeek">Times per week</option>
        </select>
      </label>
      {kind === 'weekdays' && (
        <div className="flex gap-1.5 pl-[116px]" role="group" aria-label="Days of the week">
          {weekdayOrder(weekStart).map((d) => (
            <button
              key={d}
              type="button"
              aria-pressed={days.includes(d)}
              onClick={() => toggleDay(d)}
              className={`h-8 w-9 rounded-row border text-s ${days.includes(d) ? 'border-primary bg-primary text-white' : 'border-line hover:bg-hover'}`}
            >
              {DAY_NAMES[d]}
            </button>
          ))}
        </div>
      )}
      {kind === 'perWeek' && (
        <label className="flex items-center gap-2 pl-[116px]">
          <input type="number" min={1} max={7} aria-label="Times per week" value={times} onChange={(e) => setTimes(e.target.value)} className={`${input} w-20`} />
          <span className="text-grey">times per week</span>
        </label>
      )}

      <div className={field} role="group" aria-label="Goal">
        <span className={label}>Goal</span>
        <div className="flex items-center gap-2">
          <input type="number" min={1} max={9999} aria-label="Goal amount" value={count} onChange={(e) => setCount(e.target.value)} className={`${input} w-20`} />
          <input aria-label="Goal unit" value={unit} maxLength={30} onChange={(e) => setUnit(e.target.value)} placeholder="time" className={`${input} w-24 min-w-0`} />
          <span className="text-grey">per day</span>
        </div>
      </div>

      <div className={field}>
        <span className={label}>Start Date</span>
        <select disabled aria-label="Start Date" className={disabledSelect}>
          <option>{today.toLocaleDateString('en-US', { month: 'long', day: 'numeric' })}</option>
        </select>
      </div>
      <div className={field}>
        <span className={label}>Goal Days</span>
        <select disabled aria-label="Goal Days" className={disabledSelect}>
          <option>Forever</option>
        </select>
      </div>
      <div className={field}>
        <span className={label}>Section</span>
        <select disabled aria-label="Section" className={disabledSelect}>
          <option>Others</option>
        </select>
      </div>

      <div className={field} role="radiogroup" aria-label="Colour">
        <span className={label}>Colour</span>
        <div className="flex flex-wrap gap-2">
          {SWATCHES.map((c) => (
            <Swatch key={c} label={c} selected={color === c} color={c} onSelect={() => setColor(c)} />
          ))}
        </div>
      </div>

      <label className={field}>
        <span className={label}>Reminder</span>
        <div className="flex items-center gap-2">
          <input type="time" aria-label="Reminder" value={reminder} onChange={(e) => setReminder(e.target.value)} className={`${input} w-32`} />
          {reminder && (
            <button type="button" onClick={() => setReminder('')} className="h-8 rounded-row px-3 text-grey hover:bg-hover">
              Clear
            </button>
          )}
        </div>
      </label>

      <label className="flex items-center gap-2 border-t border-line pt-3 text-grey">
        <input type="checkbox" disabled /> Auto pop-up of habit log
      </label>

      <div className="flex justify-end gap-2 pt-1">
        <button type="button" onClick={onCancel} className="h-9 rounded-row border border-line px-5 hover:bg-hover">
          Cancel
        </button>
        <button type="submit" disabled={!valid} className="h-9 rounded-row bg-primary px-6 text-white disabled:opacity-40">
          Save
        </button>
      </div>
    </form>
  )
}
