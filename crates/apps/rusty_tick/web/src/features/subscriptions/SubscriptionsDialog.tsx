import { RefreshCw, Trash2 } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { useActions, useData, useServices } from '@/app/services'
import { Dialog } from '@/components/Dialog'
import { syncSubscription, subscribe, unsubscribe, type SyncResult } from './sync'
import { useSubscriptions } from './store'
import type { SubscriptionBody } from './logic'
import type { DocItem } from '@/lib/docStore'

const summary = (r: SyncResult): string => `${r.created} added, ${r.updated} updated, ${r.trashed} removed`

/** Subscribe to calendar feeds by URL; each lives in its own list and refreshes on demand. */
export function SubscriptionsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { api } = useServices()
  const actions = useActions()
  const tasks = useData((s) => s.tasks)
  const items = useSubscriptions((s) => s.items)
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState('')
  const valid = name.trim() !== '' && url.trim() !== ''

  const run = async (label: string, job: () => Promise<SyncResult | void>): Promise<void> => {
    setBusy(label)
    setError('')
    try {
      const result = await job()
      if (result) actions.notify('info', `${label}: ${summary(result)}`)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(null)
    }
  }
  const add = (e: FormEvent): void => {
    e.preventDefault()
    if (!valid || busy) return
    void run(name.trim(), async () => {
      const result = await subscribe(name.trim(), url.trim(), { api, actions, tasks })
      setName('')
      setUrl('')
      return result
    })
  }
  const refresh = (s: DocItem<SubscriptionBody>): void => void run(s.name, () => syncSubscription(s, { api, actions, tasks }))

  return (
    <Dialog open={open} onClose={onClose} title="Calendar subscriptions" width={520}>
      <div className="flex flex-col gap-4 px-6 pb-6 pt-2">
        {items.length === 0 ? (
          <p className="text-grey">No subscriptions yet. Paste an https:// or webcal:// calendar link below.</p>
        ) : (
          <ul aria-label="Subscriptions" className="flex flex-col gap-2">
            {items.map((s) => (
              <li key={s.id} className="flex items-center gap-2 rounded-row border border-line px-3 py-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate font-semibold">{s.name}</div>
                  <div className="truncate text-s text-grey">{s.syncedMs ? `Updated ${new Date(s.syncedMs).toLocaleString()}` : 'Not synced yet'}</div>
                </div>
                <button type="button" aria-label={`Refresh ${s.name}`} disabled={busy !== null} onClick={() => refresh(s)} className="rounded-row p-1.5 text-grey hover:bg-hover disabled:opacity-40">
                  <RefreshCw size={16} className={busy === s.name ? 'animate-spin' : ''} />
                </button>
                <button type="button" aria-label={`Unsubscribe from ${s.name}`} title="Removes the subscription and its list" disabled={busy !== null} onClick={() => void run(s.name, () => unsubscribe(s, actions))} className="rounded-row p-1.5 text-grey hover:bg-hover disabled:opacity-40">
                  <Trash2 size={16} />
                </button>
              </li>
            ))}
          </ul>
        )}
        <form onSubmit={add} className="flex flex-col gap-3 border-t border-line pt-4">
          <label className="flex flex-col gap-1.5">
            <span className="text-s text-grey">Name</span>
            <input data-autofocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} placeholder="Team calendar" className="h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" />
          </label>
          <label className="flex flex-col gap-1.5">
            <span className="text-s text-grey">Calendar link</span>
            <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://example.com/calendar.ics" inputMode="url" className="h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" />
          </label>
          {error && <p role="alert" className="text-danger">{error}</p>}
          <div className="flex items-center justify-end gap-2">
            {busy && <span className="mr-auto text-s text-grey">Fetching {busy}…</span>}
            <button type="button" onClick={onClose} className="h-8 rounded-row px-4 hover:bg-hover">
              Close
            </button>
            <button type="submit" disabled={!valid || busy !== null} className="h-8 rounded-row bg-primary px-4 text-white disabled:opacity-40">
              Subscribe
            </button>
          </div>
        </form>
      </div>
    </Dialog>
  )
}
