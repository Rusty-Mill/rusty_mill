import type { ReactNode } from 'react'

/**
 * A dark pill that appears beside its child on hover or keyboard focus. CSS
 * only; the child must carry its own `aria-label` (the tooltip is decoration).
 */
export function Tooltip({ label, side = 'right', children }: { label: string; side?: 'right' | 'bottom'; children: ReactNode }) {
  const pos = side === 'right' ? 'left-full top-1/2 ml-2 -translate-y-1/2' : 'left-1/2 top-full mt-2 -translate-x-1/2'
  return (
    <span className="group relative inline-flex">
      {children}
      <span aria-hidden className={`pointer-events-none absolute ${pos} z-40 hidden whitespace-nowrap rounded-md bg-[#333] px-2 py-1 text-s text-white group-focus-within:block group-hover:block`}>
        {label}
      </span>
    </span>
  )
}
