import { Plus, X } from 'lucide-react'
import { useState } from 'react'

interface Props {
  value: string[]
  onChange: (next: string[]) => void
}

/** The minimum standard of care as lines: edit in place, remove, add. */
export function StandardsEditor({ value, onChange }: Props) {
  const [draft, setDraft] = useState('')
  const add = (): void => {
    const line = draft.trim()
    if (!line) return
    onChange([...value, line])
    setDraft('')
  }
  return (
    <div className="flex flex-col gap-1.5">
      {value.length === 0 && <p className="text-s text-grey">No standards yet.</p>}
      <ul className="flex list-none flex-col gap-1.5 p-0" aria-label="Standards">
        {value.map((line, i) => (
          <li key={i} className="flex items-center gap-1.5">
            <input aria-label={`Standard ${i + 1}`} value={line} onChange={(e) => onChange(value.map((l, j) => (j === i ? e.target.value : l)))} className="field h-8" />
            <button type="button" aria-label={`Remove standard ${i + 1}`} onClick={() => onChange(value.filter((_, j) => j !== i))} className="rounded-row p-1.5 text-grey hover:bg-hover hover:text-danger">
              <X size={15} />
            </button>
          </li>
        ))}
      </ul>
      <div className="flex items-center gap-1.5">
        <input
          aria-label="New standard"
          placeholder="Add a standard…"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              add()
            }
          }}
          className="field h-8"
        />
        <button type="button" aria-label="Add standard" onClick={add} disabled={!draft.trim()} className="btn h-8 w-8 px-0">
          <Plus size={15} />
        </button>
      </div>
    </div>
  )
}
