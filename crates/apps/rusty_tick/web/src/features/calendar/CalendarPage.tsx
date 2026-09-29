import { useCallback, useMemo, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import type { Task } from '@/api/types'
import { calendarPath, type CalendarMode } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { atTime, diffDays, startOfDay } from '@/lib/date'
import { useNow } from '@/lib/hooks'
import { usePrefs } from '../settings/prefs'
import { AddPopover } from './AddPopover'
import { AgendaView } from './AgendaView'
import { DEFAULT_COLOR, buildEvents, parseMode, rangeTitle, stepAnchor, visibleRange, type CalEvent, type Reschedule } from './layout'
import type { AddTarget, CalendarCtx } from './context'
import { DayPopover } from './DayPopover'
import { MonthView } from './MonthView'
import { TaskPopover } from './TaskPopover'
import { TimeGridView } from './TimeGridView'
import { Toolbar } from './Toolbar'

type Pop = { kind: 'task'; taskId: string; anchor: HTMLElement } | { kind: 'add'; target: AddTarget; anchor: HTMLElement } | { kind: 'day'; day: number; anchor: HTMLElement }

/** Tasks with dates on a month grid, a week or day time grid, or an agenda list. */
export function CalendarPage() {
  const mode: CalendarMode = parseMode(useParams().mode)
  const navigate = useNavigate()
  const actions = useActions()
  const { weekStart, hour12 } = usePrefs((s) => s.prefs)
  const now = useNow()
  const tasks = useData((s) => s.tasks)
  const lists = useData((s) => s.lists)
  const [anchor, setAnchor] = useState(() => Date.now())
  const [showDone, setShowDone] = useState(false)
  const [pop, setPop] = useState<Pop | null>(null)

  const events = useMemo(() => buildEvents(Object.values(tasks), showDone), [tasks, showDone])
  const range = useMemo(() => visibleRange(mode, anchor, weekStart), [mode, anchor, weekStart])
  const closePop = useCallback(() => setPop(null), [])

  const colorOf = useCallback((t: Task): string => lists[t.listId]?.color ?? DEFAULT_COLOR, [lists])
  const listName = useCallback((id: string): string => lists[id]?.name ?? 'Inbox', [lists])

  const ctx: CalendarCtx = {
    now,
    hour12,
    weekStart,
    colorOf,
    openTask: (task, el) => setPop({ kind: 'task', taskId: task.id, anchor: el }),
    openAdd: (target, el) => setPop({ kind: 'add', target, anchor: el }),
    openDay: (day, el) => setPop({ kind: 'day', day, anchor: el }),
    move: (task: Task, to: Reschedule) => {
      const patch: Parameters<typeof actions.updateTask>[1] = { dueMs: to.dueMs, isAllDay: to.isAllDay }
      if (to.startMs !== undefined) patch.startMs = to.startMs
      void actions.updateTask(task.id, patch)
    },
    addSlot: pop?.kind === 'add' ? pop.target : null,
  }

  const addFromToolbar = (el: HTMLElement): void => {
    // The visible day that is today, else the first day shown.
    const day = range.days.find((d) => diffDays(d, now) === 0) ?? (mode === 'm' ? startOfDay(anchor) : range.start)
    setPop({ kind: 'add', target: { ms: mode === 'd' || mode === 'w' ? atTime(day, 9) : day, allDay: mode !== 'd' && mode !== 'w' }, anchor: el })
  }

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-surface">
      <Toolbar
        title={rangeTitle(mode, anchor, range)}
        mode={mode}
        showDone={showDone}
        onMode={(m) => navigate(calendarPath(m))}
        onShowDone={setShowDone}
        onPrev={() => setAnchor(stepAnchor(mode, anchor, -1))}
        onNext={() => setAnchor(stepAnchor(mode, anchor, 1))}
        onToday={() => setAnchor(Date.now())}
        onAdd={addFromToolbar}
      />
      {mode === 'm' && <MonthView range={range} anchor={anchor} events={events} ctx={ctx} />}
      {(mode === 'w' || mode === 'd') && <TimeGridView key={mode} range={range} events={events} ctx={ctx} />}
      {mode === 'a' && <AgendaView range={range} events={events} ctx={ctx} listName={listName} />}

      {pop?.kind === 'task' && tasks[pop.taskId] && <TaskPopover taskId={pop.taskId} anchor={pop.anchor} color={colorOf(tasks[pop.taskId]!)} hour12={hour12} onClose={closePop} />}
      {pop?.kind === 'add' && <AddPopover anchor={pop.anchor} target={pop.target} onClose={closePop} />}
      {pop?.kind === 'day' && (
        <DayPopover
          anchor={pop.anchor}
          day={pop.day}
          events={events}
          colorOf={(e: CalEvent) => colorOf(e.task)}
          hour12={hour12}
          onPick={(e, el) => el && setPop({ kind: 'task', taskId: e.task.id, anchor: el })}
          onClose={closePop}
        />
      )}
    </main>
  )
}
