import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import type { Task } from '@/api/types'
import { calendarPath, type CalendarMode } from '@/app/paths'
import { useActions, useData, useServices } from '@/app/services'
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
import { isStale } from '../subscriptions/logic'
import { useSubscriptions } from '../subscriptions/store'
import { syncSubscription } from '../subscriptions/sync'
import { SubscriptionsDialog } from '../subscriptions/SubscriptionsDialog'
import { Toolbar } from './Toolbar'
import { parseIcs } from '@/lib/ics'

/** A calendar file can hold years of events; each import is capped so it cannot flood the queue. */
const IMPORT_LIMIT = 500
/** How old a subscription may get before opening the calendar refreshes it. */
const STALE_MS = 6 * 60 * 60 * 1000

const readText = (file: File): Promise<string> =>
  new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(String(r.result))
    r.onerror = () => reject(r.error ?? new Error('unreadable file'))
    r.readAsText(file)
  })

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
  const inboxId = useData((s) => s.inboxId)
  const [anchor, setAnchor] = useState(() => Date.now())
  const [showDone, setShowDone] = useState(false)
  const [pop, setPop] = useState<Pop | null>(null)

  const range = useMemo(() => visibleRange(mode, anchor, weekStart), [mode, anchor, weekStart])
  const events = useMemo(() => buildEvents(Object.values(tasks), showDone, range.end), [tasks, showDone, range.end])
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

  const [subsOpen, setSubsOpen] = useState(false)
  const services = useServices()
  const { api } = services

  // Subscriptions load with the page, and any not refreshed for six hours are refreshed now (a failure is reported, not retried).
  useEffect(() => {
    void (async () => {
      await useSubscriptions.getState().load(api, actions.notify)
      const { tasks: current } = services.store.getState()
      for (const sub of useSubscriptions.getState().items) {
        if (!isStale(sub, Date.now(), STALE_MS)) continue
        await syncSubscription(sub, { api, actions, tasks: current }).catch((e: unknown) => actions.notify('error', `Could not refresh ${sub.name}: ${e instanceof Error ? e.message : String(e)}`))
      }
    })()
  }, [api, actions, services.store])
  const fileRef = useRef<HTMLInputElement>(null)
  const importFile = async (e: React.ChangeEvent<HTMLInputElement>): Promise<void> => {
    const file = e.target.files?.[0]
    e.target.value = '' // so choosing the same file again fires again
    if (!file) return
    try {
      const { tasks: found, skipped, droppedRepeats } = parseIcs(await readText(file))
      const batch = found.slice(0, IMPORT_LIMIT)
      for (const t of batch) await actions.createTask({ listId: inboxId, ...t })
      const notes = [skipped && `${skipped} skipped (cancelled, completed or undated)`, droppedRepeats && `${droppedRepeats} repeating events imported once`, found.length > batch.length && `only the first ${IMPORT_LIMIT} imported`]
      actions.notify('info', `Imported ${batch.length} from ${file.name} into Inbox${notes.some(Boolean) ? ` (${notes.filter(Boolean).join('; ')})` : ''}`)
    } catch (err) {
      actions.notify('error', `Could not import ${file.name}: ${err instanceof Error ? err.message : String(err)}`)
    }
  }

  const addFromToolbar = (el: HTMLElement): void => {
    // The visible day that is today, else the first day shown.
    const day = range.days.find((d) => diffDays(d, now) === 0) ?? (mode === 'm' ? startOfDay(anchor) : range.start)
    const timed = mode !== 'm' && mode !== 'a'
    setPop({ kind: 'add', target: { ms: timed ? atTime(day, 9) : day, allDay: !timed }, anchor: el })
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
        onImport={() => fileRef.current?.click()}
        onSubscriptions={() => setSubsOpen(true)}
      />
      <input ref={fileRef} type="file" accept=".ics,text/calendar" aria-label="Import calendar file" hidden onChange={(e) => void importFile(e)} />
      <SubscriptionsDialog open={subsOpen} onClose={() => setSubsOpen(false)} />
      {mode === 'm' && <MonthView range={range} anchor={anchor} events={events} ctx={ctx} />}
      {mode !== 'm' && mode !== 'a' && <TimeGridView key={mode} range={range} events={events} ctx={ctx} />}
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
