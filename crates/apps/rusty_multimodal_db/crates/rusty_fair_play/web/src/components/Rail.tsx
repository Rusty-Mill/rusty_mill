import { LayoutGrid, Monitor, Moon, Scale, Sun, Users } from 'lucide-react'
import type { ReactNode } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { PATHS, sectionOf, type Section } from '@/app/paths'
import { useTheme, type Theme } from '@/app/theme'
import { Tooltip } from './Tooltip'

const ICON = 20
const STROKE = 1.5

function RailButton({ label, active = false, onClick, children }: { label: string; active?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <Tooltip label={label}>
      <button
        type="button"
        aria-label={label}
        aria-current={active ? 'page' : undefined}
        onClick={onClick}
        className={`flex h-[50px] w-[50px] items-center justify-center transition-colors ${active ? 'text-primary' : 'text-grey hover:text-text'}`}
      >
        {children}
      </button>
    </Tooltip>
  )
}

const NEXT: Record<Theme, Theme> = { system: 'light', light: 'dark', dark: 'system' }

/** The slim left column: the app mark, the three sections, and the theme toggle. */
export function Rail() {
  const navigate = useNavigate()
  const section = sectionOf(useLocation().pathname)
  const theme = useTheme((s) => s.theme)
  const setTheme = useTheme((s) => s.set)
  const item = (s: Section, label: string, path: string, icon: ReactNode) => (
    <RailButton label={label} active={section === s} onClick={() => navigate(path)}>
      {icon}
    </RailButton>
  )
  return (
    <nav aria-label="Sections" className="flex h-full w-[50px] shrink-0 flex-col items-center border-r border-line bg-rail">
      <div aria-hidden className="mb-1 mt-3 flex h-8 w-8 items-center justify-center rounded-row bg-suit-home text-s font-bold text-white" title="Fair Play">
        FP
      </div>
      {item('deck', 'Deck', PATHS.deck, <LayoutGrid size={ICON} strokeWidth={STROKE} />)}
      {item('players', 'Players', PATHS.players, <Users size={ICON} strokeWidth={STROKE} />)}
      {item('balance', 'Balance', PATHS.balance, <Scale size={ICON} strokeWidth={STROKE} />)}
      <div className="mt-auto pb-2">
        <RailButton label={`Theme: ${theme} (click to change)`} onClick={() => setTheme(NEXT[theme])}>
          {theme === 'light' ? <Sun size={ICON} strokeWidth={STROKE} /> : theme === 'dark' ? <Moon size={ICON} strokeWidth={STROKE} /> : <Monitor size={ICON} strokeWidth={STROKE} />}
        </RailButton>
      </div>
    </nav>
  )
}
