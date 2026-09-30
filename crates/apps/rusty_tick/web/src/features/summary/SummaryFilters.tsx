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
  { value: 'all', label: 'All' },
  { value: 'completed', label: 'Completed' },
  { value: 'open', label: 'Not completed' },
]

/** Full names do not fit three abreast; the accessible name stays the full one. */
const TEMPLATE_SHORT: Record<(typeof TEMPLATES)[number], string> = { daily: 'Daily', weekly: 'Weekly', simple: 'Simple list' }

const toggle = <T,>(xs: T[], x: T): T[] => (xs.includes(x) ? xs.filter((y) => y !== x) : [...xs, x])

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2 px-5 py-3">
      <h3 className="text-base font-semibold">{title}</h3>
      <div className="flex flex-col gap-2 rounded-[10px] bg-side px-3 py-2.5">{children}</div>
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

function Radio({ name, label, checked, onChange }: { name: string; label: string; checked: boolean; onChange: () => void }) {
  return (
    <label className="flex min-h-7 cursor-pointer items-center gap-2 text-base">
      <input type="radio" name={name} checked={checked} onChange={onChange} className="h-4 w-4 accent-primary" />
      {label}
    </label>
  )
}

const field = 'h-8 w-full rounded-row border border-line bg-surface px-2 text-base outline-none focus:border-primary'

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
              className={`h-7 flex-1 rounded-[6px] px-1 text-s ${o.template === t ? 'bg-surface font-semibold text-text shadow-sm' : 'text-grey hover:text-text'}`}
            >
              {TEMPLATE_SHORT[t]}
            </button>
          ))}
        </div>
      </Section>

      <Section title="Filter">
        <label className="flex items-center justify-between gap-3 text-base">
          Date range
          <select value={o.range} onChange={(e) => onChange({ range: e.target.value as SummaryOptions['range'] })} className="h-8 min-w-0 max-w-[180px] rounded-row bg-transparent text-right text-grey outline-none">
            {RANGE_KEYS.map((k) => (
              <option key={k} value={k}>
                {RANGE_LABELS[k]}
              </option>
            ))}
          </select>
        </label>
        {o.range === 'custom' && (
          <div className="flex items-center gap-2">
            <input type="date" aria-label="From date" value={o.custom.from} onChange={(e) => onChange({ custom: { ...o.custom, from: e.target.value } })} className={field} />
            <span className="text-grey" aria-hidden>
              –
            </span>
            <input type="date" aria-label="To date" value={o.custom.to} onChange={(e) => onChange({ custom: { ...o.custom, to: e.target.value } })} className={field} />
          </div>
        )}

        <fieldset className="mt-1 flex flex-col">
          <legend className="mb-1 text-s text-grey">Lists</legend>
          <div className="max-h-[132px] overflow-y-auto rounded-row border border-line px-2 py-1">
            <Check label="All lists" checked={o.listIds.length === 0} onChange={() => onChange({ listIds: [] })} />
            {shownLists.map((l) => (
              <Check key={l.id} label={l.name} checked={o.listIds.includes(l.id)} onChange={() => onChange({ listIds: toggle(o.listIds, l.id) })} />
            ))}
          </div>
        </fieldset>

        <fieldset className="mt-1 flex flex-col">
          <legend className="mb-1 text-s text-grey">Status</legend>
          {STATUSES.map((s) => (
            <Radio key={s.value} name="summary-status" label={s.label} checked={o.status === s.value} onChange={() => onChange({ status: s.value })} />
          ))}
        </fieldset>

        <details className="mt-1">
          <summary className="cursor-pointer select-none text-s text-grey hover:text-text">More</summary>
          <div className="mt-2 flex flex-col gap-2">
            <fieldset className="flex flex-col">
              <legend className="mb-1 text-s text-grey">Priority</legend>
              {PRIORITIES.map((p) => (
                <Check key={p.value} label={p.label} checked={o.priorities.includes(p.value)} onChange={() => onChange({ priorities: toggle(o.priorities, p.value) })} />
              ))}
            </fieldset>
            <fieldset className="flex flex-col">
              <legend className="mb-1 text-s text-grey">Tags</legend>
              {tags.length === 0 && <p className="text-s text-grey">No tags yet.</p>}
              <div className="max-h-[110px] overflow-y-auto">
                {tags.map((g) => (
                  <Check key={g.name} label={`#${g.label}`} checked={o.tags.includes(g.name)} onChange={() => onChange({ tags: toggle(o.tags, g.name) })} />
                ))}
              </div>
            </fieldset>
          </div>
        </details>
      </Section>

      <Section title="Display Options">
        <Check label="Show list name" checked={o.showList} onChange={(v) => onChange({ showList: v })} />
        <Check label="Show due date" checked={o.showDue} onChange={(v) => onChange({ showDue: v })} />
        <Check label="Show priority" checked={o.showPriority} onChange={(v) => onChange({ showPriority: v })} />
        <Check label="Show tags" checked={o.showTags} onChange={(v) => onChange({ showTags: v })} />
        <Check label="Show completed time" checked={o.showCompletedTime} onChange={(v) => onChange({ showCompletedTime: v })} />
        <fieldset className="mt-1 flex flex-col">
          <legend className="mb-1 text-s text-grey">Group by</legend>
          <div className="flex gap-4">
            <Radio name="summary-group" label="None" checked={o.groupBy === 'none'} onChange={() => onChange({ groupBy: 'none' })} />
            <Radio name="summary-group" label="List" checked={o.groupBy === 'list'} onChange={() => onChange({ groupBy: 'list' })} />
          </div>
        </fieldset>
      </Section>
    </div>
  )
}
