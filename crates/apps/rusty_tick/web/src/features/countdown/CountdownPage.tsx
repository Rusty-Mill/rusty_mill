import { Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { useActions, useServices } from '@/app/services'
import { dayKey, startOfDay } from '@/lib/date'
import { useNow } from '@/lib/hooks'
import { byUpcoming, daysLeft, describe, fromForm } from './logic'
import { useCountdowns } from './store'

export function CountdownPage() {
  const { api } = useServices()
  const { notify } = useActions()
  const now = useNow(60_000)
  const items = useCountdowns((s) => s.items)
  const [name, setName] = useState('')
  const [day, setDay] = useState(dayKey(startOfDay(Date.now())))

  useEffect(() => {
    void useCountdowns.getState().load(api, notify)
  }, [api, notify])

  const sorted = useMemo(() => byUpcoming(items, now), [items, now])
  const body = fromForm(name, day)
  const submit = (e: FormEvent): void => {
    e.preventDefault()
    if (!body) return
    useCountdowns.getState().add(body)
    setName('')
  }

  return (
    <main className="flex min-w-0 flex-1 flex-col">
      <header className="flex h-14 shrink-0 items-center px-4">
        <h1 className="text-h1 font-semibold">Countdown</h1>
      </header>
      <form onSubmit={submit} className="flex flex-wrap items-end gap-3 px-4 pb-4">
        <label className="flex flex-col gap-1">
          <span className="text-s text-grey">Name</span>
          <input value={name} maxLength={200} onChange={(e) => setName(e.target.value)} placeholder="Exam, launch, birthday" className="h-9 w-64 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-s text-grey">Date</span>
          <input type="date" value={day} onChange={(e) => setDay(e.target.value)} className="h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" />
        </label>
        <button type="submit" disabled={!body} className="h-9 rounded-row bg-primary px-4 text-white disabled:opacity-40">
          Add countdown
        </button>
      </form>
      {sorted.length === 0 ? (
        <p className="px-4 text-grey">No countdowns yet. Add an important date to count down to.</p>
      ) : (
        <ul aria-label="Countdowns" className="flex flex-col gap-2 overflow-y-auto px-4 pb-4">
          {sorted.map((c) => {
            const days = daysLeft(c.dateMs, now)
            return (
              <li key={c.id} className="group flex items-center gap-3 rounded-row border border-line bg-surface px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="truncate font-semibold">{c.name}</div>
                  <div className="text-s text-grey">{new Date(c.dateMs).toLocaleDateString(undefined, { weekday: 'short', year: 'numeric', month: 'short', day: 'numeric' })}</div>
                </div>
                <span className={`tabular-nums ${days < 0 ? 'text-grey' : days === 0 ? 'font-semibold text-primary' : ''}`}>{describe(days)}</span>
                <button type="button" aria-label={`Delete ${c.name}`} onClick={() => useCountdowns.getState().remove(c.id)} className="rounded-row p-1.5 text-grey opacity-0 hover:bg-selected focus-visible:opacity-100 group-hover:opacity-100">
                  <Trash2 size={16} strokeWidth={1.5} />
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </main>
  )
}
