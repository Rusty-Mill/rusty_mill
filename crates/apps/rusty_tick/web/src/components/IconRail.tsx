import { Bell, HelpCircle, Hourglass, RefreshCw, Sparkles, WifiOff } from 'lucide-react'
import { useRef, useState, type ReactNode } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { calendarPath, HOME, PATHS, railSection } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { AccountMenu } from '@/features/settings/AccountMenu'
import { useUi } from '@/store/ui'
import { Popover } from './Popover'
import { CalendarIcon, FocusIcon, HabitIcon, SearchIcon, TasksIcon } from './RailIcons'
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
        className={`flex h-[50px] w-[50px] items-center justify-center transition-colors duration-150 ${active ? 'text-primary' : 'text-grey hover:text-text'}`}
      >
        {children}
      </button>
    </Tooltip>
  )
}

/** The 50px column of section icons. The active one is filled blue with no background. */
export function IconRail() {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const section = railSection(pathname)
  const setSearchOpen = useUi((s) => s.setSearchOpen)
  const assistantOpen = useUi((s) => s.assistantOpen)
  const toggleAssistant = useUi((s) => s.toggleAssistant)
  const [account, setAccount] = useState(false)
  const avatarRef = useRef<HTMLButtonElement>(null)

  return (
    <nav aria-label="Sections" className="flex h-full w-[50px] shrink-0 flex-col items-center bg-rail">
      <button
        ref={avatarRef}
        type="button"
        aria-label="Account"
        aria-haspopup="menu"
        onClick={() => setAccount((o) => !o)}
        className="mb-1 mt-3 flex h-8 w-8 items-center justify-center rounded-full bg-primary text-base font-semibold text-white"
      >
        T
      </button>
      <AccountMenu anchor={avatarRef.current} open={account} onClose={() => setAccount(false)} />

      <RailButton label="Tasks" active={section === 'tasks'} onClick={() => navigate(HOME)}>
        <TasksIcon />
      </RailButton>
      <RailButton label="Calendar" active={section === 'calendar'} onClick={() => navigate(calendarPath())}>
        <CalendarIcon />
      </RailButton>
      <RailButton label="Pomodoro" active={section === 'focus'} onClick={() => navigate(PATHS.focus)}>
        <FocusIcon />
      </RailButton>
      <RailButton label="Habit Tracker" active={section === 'habit'} onClick={() => navigate(PATHS.habit)}>
        <HabitIcon />
      </RailButton>
      <RailButton label="Countdown" active={section === 'countdown'} onClick={() => navigate(PATHS.countdown)}>
        <Hourglass size={ICON} strokeWidth={STROKE} />
      </RailButton>
      <RailButton label="Search" onClick={() => setSearchOpen(true)}>
        <SearchIcon />
      </RailButton>
      <RailButton label="Assistant" active={assistantOpen} onClick={toggleAssistant}>
        <Sparkles size={ICON} strokeWidth={STROKE} />
      </RailButton>

      <div className="mt-auto flex flex-col items-center pb-2">
        <SyncButton />
        <NotificationsButton />
        <HelpButton />
      </div>
    </nav>
  )
}

/** Sync status: spins while edits are pending, shows a slash when the server is unreachable. */
function SyncButton() {
  const online = useData((s) => s.online)
  const pending = useData((s) => s.pending)
  const { retryNow, refresh } = useActions()
  const label = !online ? 'Offline: changes are saved here and will sync when the server is back' : pending > 0 ? `Syncing ${pending} change${pending === 1 ? '' : 's'}` : 'Synced'
  return (
    <RailButton label={label} onClick={() => (online ? void refresh() : retryNow())}>
      {online ? <RefreshCw size={ICON} strokeWidth={STROKE} className={pending > 0 ? 'animate-spin' : ''} /> : <WifiOff size={ICON} strokeWidth={STROKE} className="text-danger" />}
    </RailButton>
  )
}

function NotificationsButton() {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLSpanElement>(null)
  return (
    <span ref={ref}>
      <RailButton label="Notifications" onClick={() => setOpen((o) => !o)}>
        <Bell size={ICON} strokeWidth={STROKE} />
      </RailButton>
      <Popover anchor={ref.current} open={open} onClose={() => setOpen(false)} placement="right-start" ariaLabel="Notifications" className="w-64 p-4">
        <p className="text-base font-semibold">Notifications</p>
        <p className="mt-2 text-s text-grey">Nothing new. Reminders will appear here.</p>
      </Popover>
    </span>
  )
}

const SHORTCUTS: [string, string][] = [
  ['N', 'Add a task'],
  ['J / K', 'Next / previous task'],
  ['Space', 'Complete or reopen'],
  ['Delete', 'Move to Trash'],
  ['1 2 3 4', 'Priority: high, medium, low, none'],
  ['Ctrl/⌘ K', 'Search'],
  ['Esc', 'Close'],
]

function HelpButton() {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLSpanElement>(null)
  return (
    <span ref={ref}>
      <RailButton label="Help" onClick={() => setOpen((o) => !o)}>
        <HelpCircle size={ICON} strokeWidth={STROKE} />
      </RailButton>
      <Popover anchor={ref.current} open={open} onClose={() => setOpen(false)} placement="right-start" ariaLabel="Keyboard shortcuts" className="w-72 p-4">
        <p className="mb-2 text-base font-semibold">Keyboard shortcuts</p>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-base">
          {SHORTCUTS.map(([keys, what]) => (
            <div key={keys} className="contents">
              <dt className="font-mono text-s text-grey">{keys}</dt>
              <dd>{what}</dd>
            </div>
          ))}
        </dl>
      </Popover>
    </span>
  )
}
