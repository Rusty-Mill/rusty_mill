import { Plus } from 'lucide-react'
import { useMemo, useRef, useState, type FormEvent } from 'react'
import { ConflictError } from '@/api/errors'
import type { Person } from '@/api/types'
import { useActions, useData } from '@/app/services'
import { initial } from '@/components/badges'
import { balance } from '@/store/derive'

/** The people cards are dealt to: rename in place, add new ones. */
export function PlayersPage() {
  const people = useData((s) => s.people)
  const cards = useData((s) => s.cards)
  const index = useData((s) => s.index)
  const { createPerson } = useActions()
  const rows = useMemo(() => balance(index, cards, people), [index, cards, people])
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const add = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!name.trim()) return
    setBusy(true)
    setError(null)
    const submitted = name.trim()
    try {
      await createPerson(submitted)
      // Clear the box only if nothing new was typed while the server answered.
      setName((current) => (current.trim() === submitted ? '' : current))
    } catch (err) {
      setError(err instanceof ConflictError ? `${name.trim()} already exists` : (err as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="flex min-w-0 flex-1 flex-col" aria-label="Players">
      <header className="px-5 pt-4">
        <h1 className="text-h1 font-semibold">Players</h1>
        <p className="text-s text-grey">Whoever holds a card owns its conception, planning and execution.</p>
      </header>
      <div className="scroll-thin flex-1 overflow-y-auto px-5 pb-8 pt-4">
        <div className="max-w-xl">
          {people.length === 0 && <p className="mb-4 text-grey">Nobody yet. Add the people you deal cards to.</p>}
          <ul className="flex list-none flex-col gap-2 p-0" aria-label="People">
            {rows.map((row) => (
              <li key={row.person.id} className="flex items-center gap-3 rounded-card border border-line px-3 py-2">
                <span aria-hidden className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary text-base font-semibold text-white">
                  {initial(row.person.name)}
                </span>
                <div className="flex min-w-0 flex-1 flex-col">
                  <RenameField person={row.person} />
                  <span className="text-s text-grey">
                    Player {row.person.player} · holds {row.all} {row.all === 1 ? 'card' : 'cards'} ({row.leaves} {row.leaves === 1 ? 'leaf' : 'leaves'})
                  </span>
                </div>
              </li>
            ))}
          </ul>
          <form onSubmit={(e) => void add(e)} className="mt-4 flex flex-col gap-1.5" aria-label="Add person">
            <div className="flex items-center gap-2">
              <input aria-label="New person's name" placeholder="Add a person…" value={name} onChange={(e) => setName(e.target.value)} className="field h-9 flex-1" />
              <button type="submit" disabled={!name.trim() || busy} className="btn-primary h-9">
                <Plus size={15} /> Add person
              </button>
            </div>
            {error && (
              <p role="alert" className="text-s text-danger">
                {error}
              </p>
            )}
          </form>
        </div>
      </div>
    </main>
  )
}

function RenameField({ person }: { person: Person }) {
  const { renamePerson } = useActions()
  const [value, setValue] = useState(person.name)
  const [editing, setEditing] = useState(false)
  const cancelled = useRef(false)
  const commit = (): void => {
    setEditing(false)
    if (cancelled.current) {
      cancelled.current = false
      return setValue(person.name)
    }
    const next = value.trim()
    if (!next || next === person.name) return setValue(person.name)
    void renamePerson(person.id, next).catch(() => setValue(person.name))
  }
  return (
    <input
      aria-label={`Name of player ${person.player}`}
      value={editing ? value : person.name}
      onFocus={() => {
        setValue(person.name)
        setEditing(true)
      }}
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
        if (e.key === 'Escape') {
          cancelled.current = true
          ;(e.target as HTMLInputElement).blur()
        }
      }}
      className="w-full rounded-row border border-transparent bg-transparent px-1 font-medium outline-none hover:border-line focus:border-primary"
    />
  )
}
