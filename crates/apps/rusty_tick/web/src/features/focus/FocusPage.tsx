import { ChevronRight, MoreHorizontal, Plus } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useActions, useData, useServices } from '@/app/services'
import { Confirm } from '@/components/Confirm'
import { usePrefs } from '../settings/prefs'
import { FocusRecords } from './FocusRecords'
import { Overview } from './Overview'
import { SettingsRow } from './SettingsRow'
import { TimerRing } from './TimerRing'
import { computeStats, elapsedMs, remainingSec, ringProgress, type FocusMode } from './logic'
import { useFocus } from './store'
import { useNow } from './useNow'

export function FocusPage() {
  const { api } = useServices()
  const { notify } = useActions()
  const hour12 = usePrefs((s) => s.prefs.hour12)
  const { records, settings, mode, session, offer } = useFocus()
  const tasks = useData((s) => s.tasks)
  const [taskId, setTaskId] = useState('')
  const [confirmGiveUp, setConfirmGiveUp] = useState(false)
  const running = session !== null && session.pausedAt === null
  const now = useNow(running)
  const f = useFocus.getState

  useEffect(() => {
    void f().load(api, notify)
  }, [api, notify, f])

  // Backstop for the end-of-countdown timer in the store.
  useEffect(() => {
    f().checkFinished()
  }, [now, f])

  const openTasks = useMemo(
    () => Object.values(tasks).filter((t) => t.deletedMs === null && t.status === 'open').sort((a, b) => a.title.localeCompare(b.title)),
    [tasks],
  )
  const stats = useMemo(() => computeStats(records, now), [records, now])

  const isBreak = session?.phase === 'break'
  const shownMode: FocusMode = session ? session.mode : mode
  const secs = session
    ? (remainingSec(session, now) ?? Math.floor(elapsedMs(session, now) / 1000))
    : shownMode === 'pomo'
      ? settings.focusMin * 60
      : 0
  const caption = isBreak ? 'Break' : shownMode === 'pomo' ? 'Focus' : 'Stopwatch'
  const start = (): void => {
    const t = tasks[taskId]
    f().start(t ? { id: t.id, title: t.title } : null)
  }

  // Announced text only changes each minute (and on completion), so screen readers are not flooded.
  const remaining = session ? remainingSec(session, now) : null
  const live = offer
    ? `Pomo complete. Time for a ${offer.kind} break.`
    : remaining !== null
      ? `${Math.ceil(remaining / 60)} minutes remaining`
      : ''

  return (
    <main className="flex min-w-0 flex-1">
      <section aria-label="Timer" className="flex min-w-0 flex-1 flex-col items-center overflow-y-auto px-6 py-4">
        <header className="grid w-full grid-cols-[1fr_auto_1fr] items-center">
          <h1 className="text-title font-semibold">Pomodoro</h1>
        <div role="tablist" aria-label="Timer mode" className="flex rounded-full bg-side p-0.5">
          {(['pomo', 'stopwatch'] as const).map((m) => (
            <button
              key={m}
              type="button"
              role="tab"
              aria-selected={shownMode === m}
              disabled={session !== null}
              onClick={() => f().setMode(m)}
              className={`h-7 rounded-full px-5 disabled:cursor-default ${shownMode === m ? 'bg-surface font-semibold shadow-sm' : 'text-grey hover:text-text'}`}
            >
              {m === 'pomo' ? 'Pomo' : 'Stopwatch'}
            </button>
          ))}
        </div>
          <div className="flex justify-end gap-1 text-grey">
            <button type="button" disabled aria-label="Add a focus record" title="Adding records by hand is not available yet" className="flex h-8 w-8 items-center justify-center rounded-row opacity-50">
              <Plus size={20} />
            </button>
            <button type="button" disabled aria-label="More" className="flex h-8 w-8 items-center justify-center rounded-row opacity-50">
              <MoreHorizontal size={20} />
            </button>
          </div>
        </header>

        <label className="relative mt-10 flex cursor-pointer items-center gap-1 text-grey">
          <span>{tasks[taskId]?.title ?? (isBreak ? 'Break' : shownMode === 'pomo' ? 'Focus' : 'Stopwatch')}</span>
          <ChevronRight size={14} aria-hidden />
          <select
            aria-label="Task"
            value={taskId}
            disabled={session !== null}
            onChange={(e) => setTaskId(e.target.value)}
            className="absolute inset-0 cursor-pointer opacity-0"
          >
            <option value="">No task linked</option>
            {openTasks.map((t) => (
              <option key={t.id} value={t.id}>
                {t.title}
              </option>
            ))}
          </select>
        </label>

        <div className="mt-6">
          <TimerRing
            progress={offer ? 0 : ringProgress(session, shownMode, now)}
            seconds={offer ? offer.sec : secs}
            caption={offer ? `${offer.kind === 'long' ? 'Long' : 'Short'} break` : caption}
            tone={isBreak || offer ? 'break' : 'focus'}
          />
        </div>
        <p className="sr-only" aria-live="polite">{live}</p>

        {session?.taskTitle && <p className="mt-2 text-s text-grey">Focusing on {session.taskTitle}</p>}

        <div className="mt-6 flex gap-3">
          {offer ? (
            <>
              <button type="button" onClick={() => f().startBreak()} className={primary}>
                Start break
              </button>
              <button type="button" onClick={() => f().skipBreak()} className={secondary}>
                Skip
              </button>
            </>
          ) : !session ? (
            <button type="button" onClick={start} className={primary}>
              Start
            </button>
          ) : (
            <>
              <button type="button" onClick={() => (session.pausedAt === null ? f().pause() : f().resume())} className={primary}>
                {session.pausedAt === null ? 'Pause' : 'Continue'}
              </button>
              {isBreak ? (
                <button type="button" onClick={() => f().stop()} className={secondary}>
                  Skip break
                </button>
              ) : session.mode === 'pomo' ? (
                <button type="button" onClick={() => setConfirmGiveUp(true)} className={secondary}>
                  Give up
                </button>
              ) : (
                <button type="button" onClick={() => f().stop()} className={secondary}>
                  Stop
                </button>
              )}
            </>
          )}
        </div>

        <div className="mt-10">
          <SettingsRow settings={settings} onChange={(p) => f().updateSettings(p)} />
        </div>
      </section>

      <aside aria-label="Focus overview" className="flex w-[340px] shrink-0 flex-col border-l border-line max-[1000px]:w-[280px]">
        <Overview stats={stats} />
        <div className="flex items-center justify-between px-5 pb-1 pt-5">
          <h3 className="font-semibold">Focus Record</h3>
          <Plus size={18} aria-hidden className="text-grey opacity-50" />
        </div>
        <FocusRecords records={records} now={now} hour12={hour12} onDelete={(id) => f().deleteRecord(id)} />
      </aside>

      <Confirm
        open={confirmGiveUp}
        title="Give up this pomo?"
        message="The time so far will not be recorded."
        confirmLabel="Give up"
        danger
        onCancel={() => setConfirmGiveUp(false)}
        onConfirm={() => {
          setConfirmGiveUp(false)
          f().stop()
        }}
      />
    </main>
  )
}

const primary = 'h-12 min-w-[170px] rounded-full bg-primary px-8 font-semibold text-white outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-surface'
const secondary = 'h-12 min-w-[120px] rounded-full border border-line px-6 hover:bg-hover'
