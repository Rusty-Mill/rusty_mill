import { useState, type FormEvent } from 'react'
import { Dialog } from '@/components/Dialog'
import { SWATCHES } from '@/lib/colors'
import type { List } from '@/api/types'

interface Props {
  open: boolean
  onClose: () => void
  /** The list being edited; `null` creates one. */
  list: List | null
  onSubmit: (input: { name: string; color: string | null }) => void
}

export function ListDialog({ open, onClose, list, onSubmit }: Props) {
  return (
    <Dialog open={open} onClose={onClose} title={list ? 'Edit List' : 'Add List'} width={400}>
      {/* Remount per open so the fields start from the list being edited. */}
      {open && <ListForm key={list?.id ?? 'new'} list={list} onCancel={onClose} onSubmit={onSubmit} />}
    </Dialog>
  )
}

function ListForm({ list, onCancel, onSubmit }: { list: List | null; onCancel: () => void; onSubmit: Props['onSubmit'] }) {
  const [name, setName] = useState(list?.name ?? '')
  const [color, setColor] = useState<string | null>(list?.color ?? null)
  const valid = name.trim().length > 0

  const submit = (e: FormEvent): void => {
    e.preventDefault()
    if (valid) onSubmit({ name: name.trim(), color })
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4 px-6 pb-6 pt-2">
      <label className="flex flex-col gap-1.5">
        <span className="text-s text-grey">Name</span>
        <input
          data-autofocus
          value={name}
          maxLength={200}
          onChange={(e) => setName(e.target.value)}
          placeholder="List name"
          className="h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary"
        />
      </label>
      <fieldset className="flex flex-col gap-1.5">
        <legend className="mb-1.5 text-s text-grey">Colour</legend>
        <div className="flex flex-wrap gap-2" role="radiogroup" aria-label="Colour">
          <Swatch label="No colour" selected={color === null} color={null} onSelect={() => setColor(null)} />
          {SWATCHES.map((c) => (
            <Swatch key={c} label={c} selected={color === c} color={c} onSelect={() => setColor(c)} />
          ))}
        </div>
      </fieldset>
      <div className="flex justify-end gap-2 pt-1">
        <button type="button" onClick={onCancel} className="h-8 rounded-row px-4 hover:bg-hover">
          Cancel
        </button>
        <button type="submit" disabled={!valid} className="h-8 rounded-row bg-primary px-4 text-white disabled:opacity-40">
          {list ? 'Save' : 'Add'}
        </button>
      </div>
    </form>
  )
}

export function Swatch({ color, selected, onSelect, label }: { color: string | null; selected: boolean; onSelect: () => void; label: string }) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-label={label}
      onClick={onSelect}
      style={{ backgroundColor: color ?? 'transparent' }}
      className={`h-6 w-6 rounded-full border ${color ? 'border-transparent' : 'border-dashed border-grey'} ${selected ? 'ring-2 ring-primary ring-offset-2 ring-offset-surface' : ''}`}
    />
  )
}
