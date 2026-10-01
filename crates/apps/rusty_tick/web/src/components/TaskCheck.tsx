import { Check } from 'lucide-react'
import type { Priority } from '@/api/types'

/** Border (and fill, once done) colour by priority. Grey when there is none. */
export const PRIORITY_COLOR: Record<Priority, string> = {
  5: 'rgb(var(--prio-high))',
  3: 'rgb(var(--prio-medium))',
  1: 'rgb(var(--prio-low))',
  0: 'rgb(var(--grey))',
}

export const PRIORITY_LABEL: Record<Priority, string> = { 5: 'High', 3: 'Medium', 1: 'Low', 0: 'None' }

interface Props {
  checked: boolean
  priority?: Priority
  onChange: () => void
  /** Names the box for assistive tech: the task's title. */
  label: string
  size?: number
}

/** The round-cornered completion box: 16px, 1.5px border coloured by priority. */
export function TaskCheck({ checked, priority = 0, onChange, label, size = 16 }: Props) {
  const color = PRIORITY_COLOR[priority]
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-label={label}
      onClick={(e) => {
        e.stopPropagation()
        onChange()
      }}
      onKeyDown={(e) => e.stopPropagation()}
      style={{ width: size, height: size, borderColor: color, backgroundColor: checked ? color : 'transparent' }}
      className="flex shrink-0 items-center justify-center rounded-[5px] border-[1.5px] transition-colors duration-150"
    >
      {checked && <Check size={size - 4} strokeWidth={3} className="text-white" aria-hidden />}
    </button>
  )
}
