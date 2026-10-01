/**
 * Simple original SVG illustrations for empty states. Drawn with theme colours
 * so they read in light and dark; nothing here is taken from another product.
 */
const stroke = 'rgb(var(--grey) / 0.55)'
const fill = 'rgb(var(--grey) / 0.12)'
const accent = 'rgb(var(--primary) / 0.7)'

const Frame = ({ children, size = 120 }: { children: React.ReactNode; size?: number }) => (
  <svg role="img" width={size} height={size} viewBox="0 0 120 120" fill="none" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {children}
  </svg>
)

export const NoTasksArt = () => (
  <Frame>
    <rect x="30" y="20" width="60" height="80" rx="8" fill={fill} stroke={stroke} />
    <rect x="46" y="14" width="28" height="12" rx="6" fill="rgb(var(--surface))" stroke={stroke} />
    <path d="M42 50l5 5 9-10" stroke={accent} />
    <path d="M62 50h16M62 68h16M42 68h.01M42 84h.01M62 84h16" stroke={stroke} />
  </Frame>
)

export const DetailArt = () => (
  <Frame size={140}>
    <rect x="24" y="24" width="72" height="72" rx="12" fill={fill} stroke={stroke} />
    <path d="M38 46h44M38 60h44M38 74h26" stroke={stroke} />
    <circle cx="90" cy="90" r="16" fill="rgb(var(--surface))" stroke={accent} />
    <path d="M83 90l5 5 9-10" stroke={accent} />
  </Frame>
)

export const HabitArt = () => (
  <Frame>
    <path d="M60 96V56" stroke={stroke} />
    <path d="M60 68c0-14-10-22-24-22 0 14 10 22 24 22z" fill={fill} stroke={accent} />
    <path d="M60 58c0-12 9-20 22-20 0 12-9 20-22 20z" fill={fill} stroke={accent} />
    <path d="M40 98h40" stroke={stroke} />
  </Frame>
)

export const FocusArt = () => (
  <Frame>
    <circle cx="60" cy="60" r="34" fill={fill} stroke={stroke} />
    <circle cx="60" cy="60" r="20" stroke={stroke} />
    <circle cx="60" cy="60" r="6" fill={accent} stroke="none" />
  </Frame>
)

export const FunnelArt = () => (
  <Frame size={72}>
    <path d="M22 28h76L70 62v28l-20 8V62L22 28z" fill={fill} stroke={stroke} transform="scale(.7) translate(20 4)" />
  </Frame>
)

export const TagArt = () => (
  <Frame size={72}>
    <path d="M24 60V34a8 8 0 0 1 8-8h26l38 38-32 32-38-36z" fill={fill} stroke={stroke} transform="scale(.7) translate(12 8)" />
    <circle cx="36" cy="38" r="4" fill={accent} stroke="none" />
  </Frame>
)
