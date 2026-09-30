/** Filled section icons for the rail, in the real app's style: solid glyphs with the shape knocked out. */
import type { ReactNode } from 'react'

const KNOCK = 'rgb(var(--rail))'

function Svg({ children }: { children: ReactNode }) {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden fill="currentColor">
      {children}
    </svg>
  )
}

export const TasksIcon = () => (
  <Svg>
    <rect x="3" y="3" width="18" height="18" rx="4.5" />
    <path d="M7.5 12.4l3 3 6-6.4" fill="none" stroke={KNOCK} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
  </Svg>
)

export const CalendarIcon = () => (
  <Svg>
    <rect x="3" y="4" width="18" height="17" rx="4" />
    {[8, 12, 16].flatMap((x) => [10.5, 14.5, 18].map((y) => <circle key={`${x}-${y}`} cx={x} cy={y} r="1" fill={KNOCK} />))}
    <rect x="7.5" y="2" width="1.8" height="4" rx=".9" />
    <rect x="14.7" y="2" width="1.8" height="4" rx=".9" />
  </Svg>
)

export const FocusIcon = () => (
  <Svg>
    <circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" strokeWidth="3" />
    <circle cx="12" cy="12" r="2.6" />
  </Svg>
)

export const HabitIcon = () => (
  <Svg>
    <circle cx="12" cy="12" r="9.5" />
    <path d="M12 6.5V12l3.5 2" fill="none" stroke={KNOCK} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
  </Svg>
)

export const SearchIcon = () => (
  <Svg>
    <circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="currentColor" strokeWidth="3" />
    <path d="M15.5 15.5L21 21" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
  </Svg>
)
