import type { ReactNode } from 'react'

/** A pane's heading. Named by the tab, so the tabpanel and its heading agree. */
export function PaneTitle({ children }: { children: ReactNode }) {
  return <h3 className="mb-4 text-title font-semibold">{children}</h3>
}

/** A setting: label and optional hint on the left, the control on the right. */
export function Row({ label, hint, htmlFor, children }: { label: string; hint?: string; htmlFor?: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-line py-3.5 last:border-b-0">
      <div className="min-w-0">
        <label htmlFor={htmlFor} className="text-base">
          {label}
        </label>
        {hint && <p className="text-s text-grey">{hint}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  )
}

interface SegmentedProps<T extends string | number> {
  name: string
  label: string
  value: T
  options: { value: T; label: string }[]
  onChange: (value: T) => void
}

/** A row of radio buttons drawn as one control; native radios give the arrow keys and the labels. */
export function Segmented<T extends string | number>({ name, label, value, options, onChange }: SegmentedProps<T>) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex rounded-row bg-selected p-0.5">
      {options.map((o) => (
        <label key={String(o.value)} className="cursor-pointer">
          <input type="radio" name={name} checked={value === o.value} onChange={() => onChange(o.value)} className="peer sr-only" />
          <span className="flex h-7 items-center rounded-[6px] px-3 text-base text-grey peer-checked:bg-surface peer-checked:font-semibold peer-checked:text-text peer-checked:shadow-xs peer-focus-visible:ring-2 peer-focus-visible:ring-primary">
            {o.label}
          </span>
        </label>
      ))}
    </div>
  )
}

/** An on/off switch. */
export function Switch({ id, checked, onChange, label }: { id: string; checked: boolean; onChange: (on: boolean) => void; label: string }) {
  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-6 w-10 rounded-full transition-colors ${checked ? 'bg-primary' : 'bg-selected'}`}
    >
      <span className={`absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-all ${checked ? 'left-[18px]' : 'left-0.5'}`} />
    </button>
  )
}

export const selectClass = 'h-8 rounded-row border border-line bg-surface px-2 text-base text-text outline-hidden focus:border-primary'
