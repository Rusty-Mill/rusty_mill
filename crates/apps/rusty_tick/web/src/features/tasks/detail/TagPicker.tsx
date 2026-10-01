import { Check, Plus, Tag as TagIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { Tag } from '@/api/types'
import { Popover } from '@/components/Popover'

interface Props {
  anchor: HTMLElement | null
  open: boolean
  onClose: () => void
  /** Every tag that exists. */
  tags: Tag[]
  /** Names on the task. */
  selected: string[]
  onToggle: (name: string) => void
  /** A label that matches no tag: create it and put it on the task. */
  onCreate: (label: string) => void
}

/** Search-as-you-type tag picker. Enter takes the first match, or creates the typed tag. */
export function TagPicker({ anchor, open, onClose, tags, selected, onToggle, onCreate }: Props) {
  return (
    <Popover anchor={anchor} open={open} onClose={onClose} ariaLabel="Tags" role="dialog" className="w-64 p-2">
      {open && <Body tags={tags} selected={selected} onToggle={onToggle} onCreate={onCreate} />}
    </Popover>
  )
}

function Body({ tags, selected, onToggle, onCreate }: Pick<Props, 'tags' | 'selected' | 'onToggle' | 'onCreate'>) {
  const [query, setQuery] = useState('')
  const q = query.trim().toLowerCase()
  const matches = useMemo(() => tags.filter((t) => t.label.toLowerCase().includes(q)), [tags, q])
  const exact = tags.some((t) => t.name === q)
  const canCreate = q !== '' && !exact

  const onEnter = (): void => {
    const first = matches[0]
    if (exact || (first && !canCreate)) onToggle((exact ? tags.find((t) => t.name === q)! : first!).name)
    else if (canCreate) onCreate(query.trim())
    setQuery('')
  }

  return (
    <div>
      <input
        data-autofocus
        value={query}
        maxLength={64}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            onEnter()
          } else if (e.key === 'ArrowDown') {
            e.preventDefault()
            e.currentTarget.parentElement?.querySelector<HTMLElement>('[role="option"]')?.focus()
          }
        }}
        placeholder="Search or create a tag"
        aria-label="Search or create a tag"
        className="mb-1.5 h-8 w-full rounded-row border border-line bg-surface px-2 text-base outline-none focus:border-primary"
      />
      <ul role="listbox" aria-label="Tags" aria-multiselectable className="scroll-thin max-h-56 overflow-y-auto">
        {matches.map((t) => {
          const on = selected.includes(t.name)
          return (
            <li key={t.name} role="none">
              <button
                type="button"
                role="option"
                aria-selected={on}
                onClick={() => onToggle(t.name)}
                onKeyDown={(e) => {
                  const li = e.currentTarget.closest('li')
                  if (e.key === 'ArrowDown') li?.nextElementSibling?.querySelector<HTMLElement>('button')?.focus()
                  if (e.key === 'ArrowUp') (li?.previousElementSibling?.querySelector<HTMLElement>('button') ?? e.currentTarget.closest('div')?.querySelector<HTMLElement>('input'))?.focus()
                }}
                className="flex h-8 w-full items-center gap-2 rounded-row px-2 text-left hover:bg-hover"
              >
                <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: t.color ?? 'rgb(var(--grey))' }} />
                <span className="flex-1 truncate">{t.label}</span>
                {on && <Check size={16} className="text-primary" aria-hidden />}
              </button>
            </li>
          )
        })}
      </ul>
      {canCreate && (
        <button type="button" onClick={onEnter} className="mt-1 flex h-8 w-full items-center gap-2 rounded-row px-2 text-left text-primary hover:bg-hover">
          <Plus size={16} />
          <span className="truncate">Create tag “{query.trim()}”</span>
        </button>
      )}
      {matches.length === 0 && !canCreate && (
        <p className="flex items-center gap-2 px-2 py-2 text-s text-grey">
          <TagIcon size={14} /> No tags yet. Type a name to make one.
        </p>
      )}
    </div>
  )
}
