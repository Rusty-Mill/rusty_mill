import { useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ApiError } from '@/api/errors'
import { SUITS, type Suit } from '@/api/types'
import { PATHS } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { Dialog } from '@/components/Dialog'
import { StandardsEditor } from './StandardsEditor'

/** A custom (family-made) top-level card: name and suit, optionally an owner and its text. Opens the new card's pane. */
export function NewCardDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const people = useData((s) => s.people)
  const { createCard } = useActions()
  const navigate = useNavigate()
  const [name, setName] = useState('')
  const [suit, setSuit] = useState<Suit>('Home')
  const [ownerId, setOwnerId] = useState('')
  const [conception, setConception] = useState('')
  const [planning, setPlanning] = useState('')
  const [execution, setExecution] = useState('')
  const [standards, setStandards] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!name.trim()) return setError('A name is needed.')
    setBusy(true)
    setError(null)
    try {
      const card = await createCard({ name: name.trim(), suit, ownerId: ownerId || null, conception, planning, execution, minimumStandardOfCare: standards })
      onClose()
      navigate(PATHS.card(card.id))
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'The card could not be created.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={onClose} title="New card" width={520}>
      <form onSubmit={(e) => void submit(e)} className="scroll-thin flex flex-col gap-3 overflow-y-auto px-6 pb-5">
        <label className="flex flex-col gap-1">
          <span className="text-s font-semibold">Name</span>
          <input data-autofocus aria-label="Card name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Dog walking" className="field h-8" />
        </label>
        <div className="flex gap-3">
          <label className="flex flex-1 flex-col gap-1">
            <span className="text-s font-semibold">Suit</span>
            <select aria-label="Suit" value={suit} onChange={(e) => setSuit(e.target.value as Suit)} className="field h-8">
              {SUITS.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-1 flex-col gap-1">
            <span className="text-s font-semibold">Owner</span>
            <select aria-label="Owner" value={ownerId} onChange={(e) => setOwnerId(e.target.value)} className="field h-8">
              <option value="">Unassigned</option>
              {people.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        {(
          [
            ['Conception', conception, setConception],
            ['Planning', planning, setPlanning],
            ['Execution', execution, setExecution],
          ] as const
        ).map(([label, value, set]) => (
          <label key={label} className="flex flex-col gap-1">
            <span className="text-s font-semibold">{label}</span>
            <textarea aria-label={label} value={value} onChange={(e) => set(e.target.value)} rows={2} className="field resize-y" />
          </label>
        ))}
        <div className="flex flex-col gap-1">
          <span className="text-s font-semibold">Minimum standard of care</span>
          <StandardsEditor value={standards} onChange={setStandards} />
        </div>
        {error && (
          <p role="alert" className="text-s text-danger">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button type="button" onClick={onClose} className="btn border-transparent">
            Cancel
          </button>
          <button type="submit" disabled={busy} className="btn-primary">
            Create
          </button>
        </div>
      </form>
    </Dialog>
  )
}
