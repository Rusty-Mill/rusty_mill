import { useState, type FormEvent } from 'react'
import type { Tag } from '@/api/types'
import { Dialog } from '@/components/Dialog'
import { SWATCHES } from '@/lib/colors'
import { Swatch } from '../lists/ListDialog'

interface Props {
  open: boolean
  onClose: () => void
  /** The tag being edited; `null` creates one. */
  tag: Tag | null
  onSubmit: (input: { label: string; color: string | null }) => void
}

export function TagDialog({ open, onClose, tag, onSubmit }: Props) {
  return (
    <Dialog open={open} onClose={onClose} title={tag ? 'Edit Tag' : 'Add Tag'} width={400}>
      {open && <TagForm key={tag?.name ?? 'new'} tag={tag} onCancel={onClose} onSubmit={onSubmit} />}
    </Dialog>
  )
}

function TagForm({ tag, onCancel, onSubmit }: { tag: Tag | null; onCancel: () => void; onSubmit: Props['onSubmit'] }) {
  const [label, setLabel] = useState(tag?.label ?? '')
  const [color, setColor] = useState<string | null>(tag?.color ?? null)
  const valid = label.trim().length > 0

  const submit = (e: FormEvent): void => {
    e.preventDefault()
    if (valid) onSubmit({ label: label.trim(), color })
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4 px-6 pb-6 pt-2">
      <label className="flex flex-col gap-1.5">
        <span className="text-s text-grey">Name</span>
        <input
          data-autofocus
          value={label}
          maxLength={64}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="Tag name"
          className="h-9 rounded-row border border-line bg-surface px-3 outline-none focus:border-primary"
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
          {tag ? 'Save' : 'Add'}
        </button>
      </div>
    </form>
  )
}
