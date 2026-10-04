import { Plus, X } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import type { Card, SplitInput } from '@/api/types'
import { useActions, useData } from '@/app/services'
import { Dialog } from '@/components/Dialog'

interface Row {
  name: string
  ownerId: string
}

const KEEP = '__keep'
const NONE = '__none'

/** Split `card` into child cards with their own owners; the parent may keep, hand over or drop its owner. */
export function SplitDialog({ card, open, onClose }: { card: Card; open: boolean; onClose: () => void }) {
  const people = useData((s) => s.people)
  const { split } = useActions()
  const [rows, setRows] = useState<Row[]>([
    { name: '', ownerId: '' },
    { name: '', ownerId: '' },
  ])
  const [parentTo, setParentTo] = useState(KEEP)
  const [busy, setBusy] = useState(false)

  const update = (i: number, patch: Partial<Row>): void => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)))
  const valid = rows.some((r) => r.name.trim()) && rows.every((r) => r.name.trim() || !r.ownerId)

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    const children = rows.filter((r) => r.name.trim()).map((r) => ({ name: r.name.trim(), ownerId: r.ownerId || null }))
    if (children.length === 0) return
    const input: SplitInput = { children }
    if (parentTo === NONE) input.ownerId = null
    else if (parentTo !== KEEP) input.ownerId = parentTo
    setBusy(true)
    try {
      await split(card.id, input)
      onClose()
    } catch {
      /* toasted by the store */
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={onClose} title={`Split “${card.name}”`} width={520}>
      <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-4 px-6 pb-5">
        <p className="text-s text-grey">Each child is its own card of the {card.suit} suit, held by whoever you choose. The parent holds what is left.</p>
        <div className="flex flex-col gap-2" role="group" aria-label="Children">
          {rows.map((row, i) => (
            <div key={i} className="flex items-center gap-2">
              <input data-autofocus={i === 0 ? '' : undefined} aria-label={`Child ${i + 1} name`} placeholder="Child card name" value={row.name} onChange={(e) => update(i, { name: e.target.value })} className="field h-8 flex-1" />
              <select aria-label={`Child ${i + 1} owner`} value={row.ownerId} onChange={(e) => update(i, { ownerId: e.target.value })} className="field h-8 w-[150px]">
                <option value="">Unassigned</option>
                {people.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
              <button type="button" aria-label={`Remove child ${i + 1}`} disabled={rows.length === 1} onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))} className="rounded-row p-1.5 text-grey hover:bg-hover disabled:opacity-30">
                <X size={15} />
              </button>
            </div>
          ))}
          <div>
            <button type="button" onClick={() => setRows((rs) => [...rs, { name: '', ownerId: '' }])} className="btn">
              <Plus size={14} /> Add row
            </button>
          </div>
        </div>
        <label className="flex items-center gap-2">
          <span className="text-s text-grey">Hand the parent to</span>
          <select aria-label="Hand the parent to" value={parentTo} onChange={(e) => setParentTo(e.target.value)} className="field h-8 w-auto">
            <option value={KEEP}>Keep as is</option>
            <option value={NONE}>Unassigned</option>
            {people.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <div className="flex justify-end gap-2">
          <button type="button" onClick={onClose} className="btn border-transparent">
            Cancel
          </button>
          <button type="submit" disabled={!valid || busy} className="btn-primary">
            Split
          </button>
        </div>
      </form>
    </Dialog>
  )
}
