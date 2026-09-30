import { useEffect, useState } from 'react'
import { useData } from '@/app/services'
import { usePrefs } from '@/features/settings/prefs'
import { useHabits } from '@/features/habits/store'
import { upcoming, upcomingHabits } from './plan'

const HORIZON_MS = 24 * 3_600_000
const MAX_TIMER_MS = 2_147_483_647

/** Shows a browser notification when a task's or habit's reminder comes due, if the user turned that on and allowed it. */
export function Reminders() {
  const on = usePrefs((s) => s.prefs.notifications)
  const tasks = useData((s) => s.tasks)
  const habits = useHabits((s) => s.habits)
  const checkins = useHabits((s) => s.checkins)
  const [fired, setFired] = useState(0) // bumped after each notification so the next reminder is planned
  useEffect(() => {
    if (!on || typeof Notification === 'undefined' || Notification.permission !== 'granted') return
    const now = Date.now()
    const [next] = [...upcoming(Object.values(tasks), now, HORIZON_MS), ...upcomingHabits(habits, checkins, now, HORIZON_MS)].sort((a, b) => a.atMs - b.atMs)
    if (!next) return
    const timer = setTimeout(() => {
      if (next.atMs - Date.now() <= 0) new Notification(next.title, { body: next.body, tag: next.key }) // not the 24-day cap firing early
      setFired((n) => n + 1)
    }, Math.min(next.atMs - Date.now(), MAX_TIMER_MS))
    return () => clearTimeout(timer) // a task change replans; the tag stops a repeat showing twice
  }, [on, tasks, habits, checkins, fired])
  return null
}
