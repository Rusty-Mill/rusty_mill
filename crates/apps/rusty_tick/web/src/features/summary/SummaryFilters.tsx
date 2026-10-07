import type { ReactNode } from 'react'
import type { List, Priority, Tag } from '@/api/types'
import { RANGE_KEYS, RANGE_LABELS } from './range'
import { TEMPLATES, TEMPLATE_LABELS, type StatusFilter, type SummaryOptions } from './options'

interface Props {
  options: SummaryOptions
  onChange: (patch: Partial<SummaryOptions>) => void
  lists: List[]
  tags: Tag[]
}

const PRIORITIES: { value: Priority; label: string }[] = [
  { value: 5, label: 'High' },
  { value: 3, label: 'Medium' },
  { value: 1, label: 'Low' },
  { value: 0, label: 'None' },
]
const STATUSES: { value: StatusFilter; label: string }[] = [
  { value: 'all', label: 'All Status' },
  { value: 'completed', label: 'Completed' },
  { value: 'open', label: 'Not completed' },
]

/** Full names do not fit three abreast; the accessible name stays the full one. */
const TEMPLATE_SHORT: Record<(typeof TEMPLATES)[number], string> = { daily: 'Daily', weekly: 'Weekly', simple: 'Simple list' }

const FIELDS = [
  { key: 'showList', label: 'Show list name' },
  { key: 'showDue', label: 'Show due date' },
  { key: 'showPriority', label: 'Show priority' },
  { key: 'showTags', label: 'Show tags' },
  { key: 'showCompletedTime', label: 'Show completed time' },
] as const

const toggle = <T,>(xs: T[], x: T): T[] => (xs.includes(x) ? xs.filter((y) => y !== x) : [...xs, x])

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2 px-5 py-3">
      <h3 className="text-base font-semibold">{title}</h3>
      <div className="flex flex-col rounded-[10px] bg-side px-4 py-1">{children}</div>
    </section>
  )
}

function Check({ label, checked, onChange }: { label: string; checked: boolean; onChange: (on: boolean) => void }) {
  return (
    <label className="flex min-h-7 cursor-pointer items-center gap-2 text-base">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="h-4 w-4 accent-primary" />
      <span className="min-w-0 flex-1 truncate">{label}</span>
    </label>
  )
}

const rowClass = 'flex min-h-10 items-center justify-between gap-3 text-base'
const valueClass = 'h-8 min-w-0 max-w-[190px] cursor-pointer bg-transparent text-right text-grey outline-hidden'

/** A label on the left and a select on the right, as the real app lays out its filters. */
function SelectRow({ label, value, onChange, children }: { label: string; value: string; onChange: (v: string) => void; children: ReactNode }) {
  return (
    <label className={rowClass}>
      {label}
      <select value={value} onChange={(e) => onChange(e.target.value)} className={valueClass}>
        {children}
      </select>
    </label>
  )
}

/** A row whose value ("All Lists", "3 selected") opens its choices underneath. */
function OpenRow({ label, value, children }: { label: string; value: string; children: ReactNode }) {
  return (
    <details className="group">
      <summary className={`${rowClass} cursor-pointer list-none [&::-webkit-details-marker]:hidden`}>
        {label}
        <span className="truncate text-grey">{value} ▾</span>
      </summary>
      <div className="mb-2 flex flex-col">{children}</div>
    </details>
  )
}

const field = 'h-8 w-full rounded-row border border-line bg-surface px-2 text-base outline-hidden focus:border-primary'

/** Template, Filter (range, lists, status, more) and Display Options: the summary's right-hand panel. */
export function SummaryFilters({ options: o, onChange, lists, tags }: Props) {
  const shownLists = lists.filter((l) => !l.archived)
  return (
    <div>
      <Section title="Template">
        <div role="radiogroup" aria-label="Template" className="flex gap-1 rounded-row bg-selected p-0.5">
          {TEMPLATES.map((t) => (
            <button
              key={t}
              type="button"
              role="radio"
              aria-label={TEMPLATE_LABELS[t]}
              aria-checked={o.template === t}
              onClick={() => onChange({ template: t })}
              className={`h-7 flex-1 rounded-[6px] px-1 text-s ${o.template === t ? 'bg-surface font-semibold text-text shadow-xs' : 'text-grey hover:text-text'}`}
            >
              {TEMPLATE_SHORT[t]}
            </button>
          ))}
        </div>
      </Section>

      <Section title="Filter">
        <SelectRow label="Date" value={o.range} onChange={(v) => onChange({ range: v as SummaryOptions['range'] })}>
          {RANGE_KEYS.map((k) => (
            <option key={k} value={k}>
              {RANGE_LABELS[k]}
            </option>
          ))}
        </SelectRow>
        {o.range === 'custom' && (
          <div className="flex items-center gap-2 pb-2">
            <input type="date" aria-label="From date" value={o.custom.from} onChange={(e) => onChange({ custom: { ...o.custom, from: e.target.value } })} className={field} />
            <span className="text-grey" aria-hidden>
              –
            </span>
            <input type="date" aria-label="To date" value={o.custom.to} onChange={(e) => onChange({ custom: { ...o.custom, to: e.target.value } })} className={field} />
          </div>
        )}

        <OpenRow label="Lists" value={o.listIds.length === 0 ? 'All Lists' : `${o.listIds.length} selected`}>
          <div className="max-h-[132px] overflow-y-auto">
            <Check label="All lists" checked={o.listIds.length === 0} onChange={() => onChange({ listIds: [] })} />
            {shownLists.map((l) => (
              <Check key={l.id} label={l.name} checked={o.listIds.includes(l.id)} onChange={() => onChange({ listIds: toggle(o.listIds, l.id) })} />
            ))}
          </div>
        </OpenRow>

        <SelectRow label="Status" value={o.status} onChange={(v) => onChange({ status: v as StatusFilter })}>
          {STATUSES.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </SelectRow>

        <OpenRow label="More" value={o.priorities.length + o.tags.length === 0 ? 'None' : `${o.priorities.length + o.tags.length} selected`}>
          <fieldset className="flex flex-col">
            <legend className="mb-1 text-s text-grey">Priority</legend>
            {PRIORITIES.map((p) => (
              <Check key={p.value} label={p.label} checked={o.priorities.includes(p.value)} onChange={() => onChange({ priorities: toggle(o.priorities, p.value) })} />
            ))}
          </fieldset>
          <fieldset className="mt-2 flex flex-col">
            <legend className="mb-1 text-s text-grey">Tags</legend>
            {tags.length === 0 && <p className="text-s text-grey">No tags yet.</p>}
            <div className="max-h-[110px] overflow-y-auto">
              {tags.map((g) => (
                <Check key={g.name} label={`#${g.label}`} checked={o.tags.includes(g.name)} onChange={() => onChange({ tags: toggle(o.tags, g.name) })} />
              ))}
            </div>
          </fieldset>
        </OpenRow>
      </Section>

      <Section title="Display Options">
        <SelectRow label="Grouping" value={o.groupBy} onChange={(v) => onChange({ groupBy: v as SummaryOptions['groupBy'] })}>
          <option value="none">No Grouping</option>
          <option value="list">By List</option>
        </SelectRow>
        <OpenRow label="Fields" value={`${FIELDS.filter((f) => o[f.key]).length} selected`}>
          {FIELDS.map((f) => (
            <Check key={f.key} label={f.label} checked={o[f.key]} onChange={(v) => onChange({ [f.key]: v })} />
          ))}
        </OpenRow>
      </Section>
    </div>
  )
}
