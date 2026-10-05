/**
 * The focus store. Module-level on purpose: a running session must survive the
 * page being unmounted (navigating away and back), so nothing about it lives in
 * component state. Records and settings are `focus` docs on the server.
 */
import { create } from 'zustand'
import type { ApiClient } from '@/api/client'
import { NotFoundError } from '@/api/errors'
import { newId } from '@/lib/id'
import {
  DEFAULT_SETTINGS,
  asRecordBody,
  breakFor,
  elapsedMs,
  isFinished,
  pomosToday,
  sanitizeSettings,
  type FocusMode,
  type FocusRecord,
  type FocusRecordBody,
  type FocusSettings,
  type Session,
} from './logic'

/** The one settings document; its body has no `startMs`, which tells it apart from a record. */
export const SETTINGS_ID = '00000000-0000-7000-8000-0000000000f0'
const CACHE = 'tick-local:focus:v1'

type Notify = (kind: 'error' | 'info', message: string) => void

interface FocusState {
  records: FocusRecord[]
  settings: FocusSettings
  mode: FocusMode
  session: Session | null
  /** A finished pomo offers this break; `null` when there is nothing to offer. */
  offer: { kind: 'short' | 'long'; sec: number } | null
  /** Set when a countdown ends; drives the polite announcement. */
  lastCompleted: number | null
  load(api: ApiClient, notify: Notify): Promise<void>
  setMode(mode: FocusMode): void
  updateSettings(patch: Partial<FocusSettings>): void
  start(task: { id: string; title: string } | null): void
  /** Log that the user was pulled away from the running focus session. */
  interrupt(): void
  pause(): void
  resume(): void
  /** Stop the stopwatch (records it) or give up a pomo / break (records nothing). */
  stop(): void
  /** Called on every tick and by the end-of-countdown timer; a no-op until the countdown is over. */
  checkFinished(): void
  startBreak(): void
  skipBreak(): void
  deleteRecord(id: string): void
}

let api: ApiClient | null = null
let notify: Notify = () => undefined
let endTimer: ReturnType<typeof setTimeout> | undefined

function readCache(): { settings: FocusSettings; records: FocusRecord[] } {
  try {
    const raw = localStorage.getItem(CACHE)
    if (raw) {
      const o = JSON.parse(raw) as { settings?: unknown; records?: { id: string; body: unknown }[] }
      const records = (o.records ?? []).flatMap((r) => {
        const body = asRecordBody(r.body)
        return body ? [{ id: r.id, ...body }] : []
      })
      return { settings: sanitizeSettings(o.settings), records }
    }
  } catch {
    /* unreadable cache: start empty */
  }
  return { settings: DEFAULT_SETTINGS, records: [] }
}

function writeCache(settings: FocusSettings, records: FocusRecord[]): void {
  try {
    localStorage.setItem(CACHE, JSON.stringify({ settings, records: records.map(({ id, ...body }) => ({ id, body })) }))
  } catch {
    /* storage unavailable */
  }
}

function save(id: string, body: unknown, what: string): void {
  api?.putDoc('focus', id, body).catch(() => notify('error', `Could not save ${what}. It will stay on this device for now.`))
}

export const useFocus = create<FocusState>()((set, get) => {
  const cached = readCache()

  const armTimer = (): void => {
    clearTimeout(endTimer)
    const s = get().session
    if (!s || s.pausedAt !== null || s.targetSec === null) return
    // Timers can fire late in a throttled tab; checkFinished works from timestamps so lateness is harmless.
    endTimer = setTimeout(() => get().checkFinished(), Math.max(0, s.targetSec * 1000 - elapsedMs(s, Date.now())) + 20)
  }

  const commit = (rec: FocusRecordBody): void => {
    const record: FocusRecord = { id: newId(), ...rec }
    const records = [...get().records, record]
    set({ records })
    writeCache(get().settings, records)
    save(record.id, rec, 'the focus record')
  }

  return {
    records: cached.records,
    settings: cached.settings,
    mode: 'pomo',
    session: null,
    offer: null,
    lastCompleted: null,

    async load(client, notifyFn) {
      api = client
      notify = notifyFn
      try {
        const docs = await client.listDocs('focus')
        const records: FocusRecord[] = []
        let settings = get().settings
        for (const d of docs) {
          if (d.id === SETTINGS_ID) settings = sanitizeSettings(d.body)
          else {
            const body = asRecordBody(d.body)
            if (body) records.push({ id: d.id, ...body })
          }
        }
        // Keep records made locally that the server has not returned yet.
        const known = new Set(records.map((r) => r.id))
        const merged = [...records, ...get().records.filter((r) => !known.has(r.id))]
        set({ records: merged, settings })
        writeCache(settings, merged)
      } catch {
        /* offline: the cached copy stays */
      }
    },

    setMode(mode) {
      if (!get().session) set({ mode, offer: null })
    },

    updateSettings(patch) {
      const settings = sanitizeSettings({ ...get().settings, ...patch })
      set({ settings })
      writeCache(settings, get().records)
      save(SETTINGS_ID, settings, 'the focus settings')
    },

    start(task) {
      const { mode, settings } = get()
      set({
        offer: null,
        session: {
          mode,
          phase: 'focus',
          targetSec: mode === 'pomo' ? settings.focusMin * 60 : null,
          startedMs: Date.now(),
          pausedAt: null,
          pausedTotalMs: 0,
          taskId: task?.id ?? null,
          taskTitle: task?.title ?? null,
          interruptions: 0,
        },
      })
      armTimer()
    },

    interrupt() {
      const s = get().session
      if (s && s.phase === 'focus') set({ session: { ...s, interruptions: s.interruptions + 1 } })
    },

    pause() {
      const s = get().session
      if (s && s.pausedAt === null) set({ session: { ...s, pausedAt: Date.now() } })
      armTimer()
    },

    resume() {
      const s = get().session
      if (s && s.pausedAt !== null) set({ session: { ...s, pausedAt: null, pausedTotalMs: s.pausedTotalMs + (Date.now() - s.pausedAt) } })
      armTimer()
    },

    stop() {
      const s = get().session
      if (!s) return
      clearTimeout(endTimer)
      const now = Date.now()
      if (s.phase === 'focus' && s.mode === 'stopwatch') {
        const sec = Math.round(elapsedMs(s, now) / 1000)
        if (sec >= 1) commit({ startMs: s.startedMs, endMs: s.pausedAt ?? now, durationSec: sec, kind: 'stopwatch', taskId: s.taskId, taskTitle: s.taskTitle, interruptions: s.interruptions })
      }
      set({ session: null, offer: null })
    },

    checkFinished() {
      const s = get().session
      const now = Date.now()
      if (!s || s.pausedAt !== null || !isFinished(s, now)) return
      clearTimeout(endTimer)
      if (s.phase === 'break') return set({ session: null, offer: null, lastCompleted: now })
      // The end is when the countdown ran out, not when we noticed.
      const endMs = s.startedMs + s.pausedTotalMs + (s.targetSec ?? 0) * 1000
      commit({ startMs: s.startedMs, endMs, durationSec: s.targetSec ?? 0, kind: 'pomo', taskId: s.taskId, taskTitle: s.taskTitle, interruptions: s.interruptions })
      set({ session: null, offer: breakFor(pomosToday(get().records, now), get().settings), lastCompleted: now })
    },

    startBreak() {
      const offer = get().offer
      if (!offer) return
      set({
        offer: null,
        session: { mode: 'pomo', phase: 'break', targetSec: offer.sec, startedMs: Date.now(), pausedAt: null, pausedTotalMs: 0, taskId: null, taskTitle: null, interruptions: 0 },
      })
      armTimer()
    },

    skipBreak() {
      set({ offer: null })
    },

    deleteRecord(id) {
      const records = get().records.filter((r) => r.id !== id)
      set({ records })
      writeCache(get().settings, records)
      api?.deleteDoc('focus', id).catch((e: unknown) => {
        // Already gone on the server is the outcome we wanted.
        if (!(e instanceof NotFoundError)) notify('error', 'Could not delete the focus record.')
      })
    },
  }
})

/** For tests: forget everything held at module level. */
export function resetFocusStore(): void {
  clearTimeout(endTimer)
  api = null
  notify = () => undefined
  useFocus.setState({ records: [], settings: DEFAULT_SETTINGS, mode: 'pomo', session: null, offer: null, lastCompleted: null })
}
