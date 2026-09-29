import { formatClock } from './logic'

interface Props {
  /** 0..1 of the ring still filled. */
  progress: number
  seconds: number
  caption: string
  /** Break time is drawn green; focus is blue. */
  tone: 'focus' | 'break'
  size?: number
}

const STROKE = 10

/** The big circular timer. The clock text is the accessible content; the SVG is decoration. */
export function TimerRing({ progress, seconds, caption, tone, size = 280 }: Props) {
  const r = (size - STROKE) / 2
  const c = 2 * Math.PI * r
  return (
    <div role="timer" aria-label={`${caption} timer`} className="relative flex items-center justify-center" style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden className="-rotate-90">
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="rgb(var(--line))" strokeWidth={STROKE} />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke={tone === 'focus' ? 'rgb(var(--primary))' : 'rgb(var(--green))'}
          strokeWidth={STROKE}
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - Math.min(1, Math.max(0, progress)))}
        />
      </svg>
      <div className="absolute flex flex-col items-center">
        <span className="text-[52px] font-semibold leading-none tabular-nums">{formatClock(seconds)}</span>
        <span className="mt-2 text-grey">{caption}</span>
      </div>
    </div>
  )
}
