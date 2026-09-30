import { useEffect, useState } from 'react'
import { useData } from '@/app/services'
import { usePrefs } from '@/features/settings/prefs'
import { upcoming } from './plan'

const HORIZON_MS = 24 * 3_600_000
const MAX_TIMER_MS = 2_147_483_647

/** Shows a browser notification when a task's reminder comes due, if the user turned that on and allowed it. */
export function Reminders() {
  const on = usePrefs((s) => s.prefs.notifications)
  const tasks = useData((s) => s.tasks)
  const [fired, setFired] = useState(0) // bumped after each notification so the next reminder is planned
  useEffect(() => {
    if (!on || typeof Notification === 'undefined' || Notification.permission !== 'granted') return
    const [next] = upcoming(Object.values(tasks), Date.now(), HORIZON_MS)
    if (!next) return
    const timer = setTimeout(() => {
      if (next.atMs - Date.now() <= 0) new Notification(next.title, { tag: next.key }) // not the 24-day cap firing early
      setFired((n) => n + 1)
    }, Math.min(next.atMs - Date.now(), MAX_TIMER_MS))
    return () => clearTimeout(timer) // a task change replans; the tag stops a repeat showing twice
  }, [on, tasks, fired])
  return null
}
