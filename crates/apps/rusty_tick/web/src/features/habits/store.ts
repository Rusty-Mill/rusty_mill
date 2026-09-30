/**
 * Habits and their check-ins, held in a module-level store so leaving the page
 * loses nothing. Both are client documents: `habit` and `habit_checkin`. Edits
 * apply at once; a failed save keeps the change on screen, tells the user, and
 * is retried the next time the page loads.
 */
import { create } from 'zustand'
import type { ApiClient } from '@/api/client'
import { NotFoundError } from '@/api/errors'
import type { DocKind } from '@/api/types'
import { newId } from '@/lib/id'
import { asCheckinBody, asHabitBody, checkinId, parseGoal, type Checkin, type Habit, type HabitBody } from './logic'

const CACHE = 'tick-local:habits:v1'
type Notify = (kind: 'error' | 'info', message: string) => void

export type HabitInput = Pick<HabitBody, 'name' | 'color' | 'goal' | 'frequency' | 'reminder'>

interface HabitsState {
  habits: Habit[]
  /** Keyed by doc id (which is derived from habit and day). */
  checkins: Record<string, Checkin>
  load(api: ApiClient, notify: Notify): Promise<void>
  add(input: HabitInput): string
  update(id: string, input: HabitInput): void
  remove(id: string): void
  toggle(habitId: string, day: string): void
}

let api: ApiClient | null = null
let notify: Notify = () => undefined
/** Docs whose last save failed, to be sent again on the next load. */
const unsaved = new Map<string, { kind: DocKind; body: unknown }>()

function readCache(): Pick<HabitsState, 'habits' | 'checkins'> {
  try {
    const raw = localStorage.getItem(CACHE)
    if (raw) {
      const o = JSON.parse(raw) as { habits?: Habit[]; checkins?: Checkin[] }
      const habits = (o.habits ?? []).flatMap((h) => {
        const body = asHabitBody(h)
        return body ? [{ id: h.id, ...body }] : []
      })
      const checkins: Record<string, Checkin> = {}
      for (const c of o.checkins ?? []) {
        const body = asCheckinBody(c)
        if (body) checkins[c.id] = { id: c.id, ...body }
      }
      return { habits, checkins }
    }
  } catch {
    /* unreadable cache: start empty */
  }
  return { habits: [], checkins: {} }
}

function writeCache(s: Pick<HabitsState, 'habits' | 'checkins'>): void {
  try {
    localStorage.setItem(CACHE, JSON.stringify({ habits: s.habits, checkins: Object.values(s.checkins) }))
  } catch {
    /* storage unavailable */
  }
}

function put(kind: DocKind, id: string, body: unknown, what: string): void {
  unsaved.delete(id)
  api?.putDoc(kind, id, body).catch(() => {
    unsaved.set(id, { kind, body })
    notify('error', `Could not save ${what}. It will be retried when you are back online.`)
  })
}

function del(kind: DocKind, id: string, what: string): void {
  unsaved.delete(id)
  api?.deleteDoc(kind, id).catch((e: unknown) => {
    if (!(e instanceof NotFoundError)) notify('error', `Could not delete ${what}.`) // already gone is fine
  })
}

const bodyOf = ({ id: _id, ...body }: Habit): HabitBody => body

export const useHabits = create<HabitsState>()((set, get) => {
  const commit = (next: Pick<HabitsState, 'habits' | 'checkins'>): void => {
    set(next)
    writeCache(next)
  }
  return {
    ...readCache(),

    async load(client, notifyFn) {
      api = client
      notify = notifyFn
      try {
        const [hs, cs] = await Promise.all([client.listDocs('habit'), client.listDocs('habit_checkin')])
        const habits = hs.flatMap((d) => {
          const body = asHabitBody(d.body)
          return body ? [{ id: d.id, ...body }] : []
        })
        const checkins: Record<string, Checkin> = {}
        for (const d of cs) {
          const body = asCheckinBody(d.body)
          if (body && body.count > 0) checkins[d.id] = { id: d.id, ...body }
        }
        // Local edits that never reached the server win over its older copy.
        for (const [id, u] of unsaved) {
          if (u.kind === 'habit') {
            const body = asHabitBody(u.body)
            if (body) {
              const at = habits.findIndex((h) => h.id === id)
              if (at >= 0) habits[at] = { id, ...body }
              else habits.push({ id, ...body })
            }
          } else {
            const body = asCheckinBody(u.body)
            if (body) checkins[id] = { id, ...body }
          }
          put(u.kind, id, u.body, 'your changes')
        }
        commit({ habits, checkins })
      } catch {
        /* offline: the cached copy stays */
      }
    },

    add(input) {
      const id = newId()
      const body: HabitBody = { ...input, createdMs: Date.now(), archived: false }
      commit({ habits: [...get().habits, { id, ...body }], checkins: get().checkins })
      put('habit', id, body, 'the habit')
      return id
    },

    update(id, input) {
      const habits = get().habits.map((h) => (h.id === id ? { ...h, ...input } : h))
      const next = habits.find((h) => h.id === id)
      if (!next) return
      commit({ habits, checkins: get().checkins })
      put('habit', id, bodyOf(next), 'the habit')
    },

    remove(id) {
      const checkins = Object.fromEntries(Object.entries(get().checkins).filter(([, c]) => c.habitId !== id))
      const gone = Object.values(get().checkins).filter((c) => c.habitId === id)
      commit({ habits: get().habits.filter((h) => h.id !== id), checkins })
      del('habit', id, 'the habit')
      for (const c of gone) del('habit_checkin', c.id, 'a check-in')
    },

    toggle(habitId, day) {
      const id = checkinId(habitId, day)
      const goal = parseGoal(get().habits.find((h) => h.id === habitId)?.goal ?? '').count
      const count = get().checkins[id]?.count ?? 0
      const next = count >= goal ? 0 : count + 1 // each click adds one until the goal is met; one more clears the day
      const checkins = { ...get().checkins }
      if (next === 0) {
        delete checkins[id]
        commit({ habits: get().habits, checkins })
        del('habit_checkin', id, 'the check-in')
      } else {
        const body = { habitId, day, count: next }
        checkins[id] = { id, ...body }
        commit({ habits: get().habits, checkins })
        put('habit_checkin', id, body, 'the check-in')
      }
    },
  }
})

/** For tests: forget everything held at module level. */
export function resetHabitsStore(): void {
  api = null
  notify = () => undefined
  unsaved.clear()
  useHabits.setState({ habits: [], checkins: {} })
}
