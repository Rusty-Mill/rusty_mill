/** A calendar outline with a short label inside: the day of the month for Today, the weekday for Next 7 Days. */
export function DateIcon({ label }: { label: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden fill="none" stroke="currentColor">
      <rect x="1.5" y="2.5" width="15" height="14" rx="3.5" strokeWidth="1.4" />
      <path d="M1.8 6.6h14.4" strokeWidth="1.4" />
      <text x="9" y="14.2" textAnchor="middle" fontSize="6.6" fontWeight="700" fill="currentColor" stroke="none">
        {label}
      </text>
    </svg>
  )
}
