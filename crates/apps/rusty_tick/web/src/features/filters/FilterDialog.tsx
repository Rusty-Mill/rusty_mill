import { useState, type FormEvent } from 'react'
import type { List, Priority, Tag } from '@/api/types'
import { Dialog } from '@/components/Dialog'
import { UNASSIGNED } from '../assignees/logic'
import { DATE_BUCKETS, PRIORITIES, emptyRule, type DateBucket, type Filter, type FilterBody, type FilterRule } from './logic'

interface Props {
  open: boolean
  onClose: () => void
  /** The filter being edited; `null` creates one. */
  filter: Filter | null
  lists: List[]
  tags: Tag[]
  /** Assignee names in use, offered as filter choices. */
  assigneeNames: string[]
  onSubmit: (input: FilterBody) => void
}

export function FilterDialog({ open, onClose, filter, lists, tags, assigneeNames, onSubmit }: Props) {
  return (
    <Dialog open={open} onClose={onClose} title={filter ? 'Edit Filter' : 'Add Filter'} width={460}>
      {/* Remount per open so the fields start from the filter being edited. */}
      {open && <FilterForm key={filter?.id ?? 'new'} filter={filter} lists={lists} tags={tags} assigneeNames={assigneeNames} onCancel={onClose} onSubmit={onSubmit} />}
    </Dialog>
  )
}

const toggle = <T,>(xs: T[], x: T): T[] => (xs.includes(x) ? xs.filter((y) => y !== x) : [...xs, x])

function FilterForm({ filter, lists, tags, assigneeNames, onCancel, onSubmit }: Omit<Props, 'open' | 'onClose'> & { onCancel: () => void }) {
  const [name, setName] = useState(filter?.name ?? '')
  const [rule, setRule] = useState<FilterRule>(filter?.rule ?? emptyRule())
  const valid = name.trim().length > 0

  const submit = (e: FormEvent): void => {
    e.preventDefault()
    if (valid) onSubmit({ name: name.trim(), rule })
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4 px-6 pb-6 pt-2">
      <label className="flex flex-col gap-1.5">
        <span className="text-s text-grey">Name</span>
        <input data-autofocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} placeholder="Filter name" className="h-9 rounded-row border border-line bg-surface px-3 outline-none focus:border-primary" />
      </label>
      <Chips legend="Lists" options={lists.filter((l) => !l.archived).map((l) => [l.id, l.name])} selected={rule.lists} onToggle={(v) => setRule({ ...rule, lists: toggle(rule.lists, v) })} />
      <Chips legend="Tags" options={tags.map((t) => [t.name, t.label])} selected={rule.tags} onToggle={(v) => setRule({ ...rule, tags: toggle(rule.tags, v) })} />
      <Chips legend="Priority" options={PRIORITIES.map(([p, l]) => [String(p), l])} selected={rule.priorities.map(String)} onToggle={(v) => setRule({ ...rule, priorities: toggle(rule.priorities, Number(v) as Priority) })} />
      <Chips legend="Assignee" options={assigneeNames.length ? [...assigneeNames, UNASSIGNED].map((n) => [n, n === UNASSIGNED ? 'Unassigned' : n] as const) : []} selected={rule.assignees} onToggle={(v) => setRule({ ...rule, assignees: toggle(rule.assignees, v) })} />
      <Chips legend="Date" options={DATE_BUCKETS} selected={rule.dates} onToggle={(v) => setRule({ ...rule, dates: toggle(rule.dates, v as DateBucket) })} />
      <p className="text-s text-grey">Each group narrows the result; leave a group empty to ignore it.</p>
      <div className="flex justify-end gap-2 pt-1">
        <button type="button" onClick={onCancel} className="h-8 rounded-row px-4 hover:bg-hover">
          Cancel
        </button>
        <button type="submit" disabled={!valid} className="h-8 rounded-row bg-primary px-4 text-white disabled:opacity-40">
          {filter ? 'Save' : 'Add'}
        </button>
      </div>
    </form>
  )
}

function Chips({ legend, options, selected, onToggle }: { legend: string; options: readonly (readonly [string, string])[]; selected: string[]; onToggle: (value: string) => void }) {
  if (options.length === 0) return null
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="mb-1.5 text-s text-grey">{legend}</legend>
      <div className="flex flex-wrap gap-2">
        {options.map(([value, label]) => (
          <button
            key={value}
            type="button"
            aria-pressed={selected.includes(value)}
            onClick={() => onToggle(value)}
            className={`h-7 rounded-full border px-3 text-s ${selected.includes(value) ? 'border-primary bg-primary/10 text-primary' : 'border-line text-grey hover:bg-hover'}`}
          >
            {label}
          </button>
        ))}
      </div>
    </fieldset>
  )
}
