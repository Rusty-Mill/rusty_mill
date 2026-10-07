import { Check, Flame, MoreHorizontal } from 'lucide-react'
import { useState } from 'react'
import { Menu } from '@/components/Menu'
import { dayKey, monthName } from '@/lib/date'
import { computeStreak, frequencyLabel, isDue, parseGoal, type Habit } from './logic'
import type { WeekStart } from '@/lib/date'

export const ROW_GRID = 'grid grid-cols-[minmax(150px,1fr)_repeat(7,44px)_96px_36px] items-center'
const WEEKDAY_FULL = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']

interface Props {
  habit: Habit
  days: number[]
  /** How many times this habit was done on each `YYYY-MM-DD` day. */
  counts: ReadonlyMap<string, number>
  today: number
  weekStart: WeekStart
  onToggle: (day: string) => void
  onEdit: () => void
  onDelete: () => void
}

/** "Read, Tuesday Sep 29, not done", or "Water, Tuesday Sep 29, 2 of 8" for a goal of several. */
export function cellLabel(name: string, ms: number, count: number, goal = 1): string {
  const d = new Date(ms)
  const status = goal > 1 ? `${count} of ${goal}` : count >= goal ? 'done' : 'not done'
  return `${name}, ${WEEKDAY_FULL[d.getDay()]} ${monthName(d.getMonth())} ${d.getDate()}, ${status}`
}

export function HabitRow({ habit, days, counts, today, weekStart, onToggle, onEdit, onDelete }: Props) {
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)
  const goal = parseGoal(habit.goal).count
  const checked = new Set([...counts].filter(([, n]) => n >= goal).map(([day]) => day)) // only days that met the goal keep a streak
  const streak = computeStreak(habit, checked, today, weekStart)
  const unit = streak.unit === 'day' ? 'day' : 'week'
  return (
    <li className={`${ROW_GRID} group border-b border-line py-2 hover:bg-hover/60`}>
      <div className="flex min-w-0 items-center gap-3 pl-4">
        <span aria-hidden className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full font-semibold text-white" style={{ backgroundColor: habit.color }}>
          {[...habit.name][0]?.toUpperCase()}
        </span>
        <div className="min-w-0">
          <div className="truncate font-semibold">{habit.name}</div>
          <div className="truncate text-s text-grey">
            {habit.goal} · {frequencyLabel(habit.frequency)}
          </div>
        </div>
      </div>
      {days.map((d) => {
        const key = dayKey(d)
        const count = counts.get(key) ?? 0
        const done = count >= goal
        const partial = count > 0 && !done
        const future = d > today
        const due = isDue(habit, d)
        return (
          <div key={key} className="flex justify-center">
            <button
              type="button"
              aria-label={cellLabel(habit.name, d, count, goal)}
              aria-pressed={done}
              disabled={future}
              onClick={() => onToggle(key)}
              style={done ? { backgroundColor: habit.color, borderColor: habit.color } : partial ? { borderColor: habit.color, color: habit.color } : undefined}
              className={`flex h-7 w-7 items-center justify-center rounded-full border-[1.5px] outline-hidden focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-surface disabled:cursor-default ${
                done ? 'text-white' : partial ? 'text-s font-semibold' : `border-dashed border-grey hover:bg-selected ${due ? 'opacity-90' : 'opacity-40'} ${future ? 'opacity-30' : ''}`
              }`}
            >
              {done && <Check size={16} strokeWidth={2.5} aria-hidden />}
              {partial && <span aria-hidden>{count}</span>}
            </button>
          </div>
        )
      })}
      <div className={`flex items-center justify-center gap-1 ${streak.current > 0 ? 'text-text' : 'text-grey'}`} title={`Best streak: ${streak.best} ${unit}${streak.best === 1 ? '' : 's'}`}>
        <Flame size={16} strokeWidth={1.5} className={streak.current > 0 ? 'text-[#faa700]' : ''} aria-hidden />
        <span className="tabular-nums">
          {streak.current} {unit}
          {streak.current === 1 ? '' : 's'}
        </span>
      </div>
      <div className="flex justify-center">
        <button
          type="button"
          ref={setMenuAnchor}
          aria-label={`More actions for ${habit.name}`}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((o) => !o)}
          className="rounded-row p-1.5 text-grey hover:bg-selected focus-visible:ring-2 focus-visible:ring-primary"
        >
          <MoreHorizontal size={20} strokeWidth={1.5} />
        </button>
        <Menu
          anchor={menuAnchor}
          open={menuOpen}
          onClose={() => setMenuOpen(false)}
          placement="bottom-end"
          label={`${habit.name} actions`}
          items={[
            { id: 'edit', label: 'Edit', onSelect: onEdit },
            { id: 'delete', label: 'Delete', danger: true, onSelect: onDelete },
          ]}
        />
      </div>
    </li>
  )
}
