import { Bell, Clock, Info, Keyboard, Palette, User, type LucideIcon } from 'lucide-react'
import type { SettingsTab } from '@/app/paths'

export const TAB_META: { id: SettingsTab; label: string; icon: LucideIcon }[] = [
  { id: 'account', label: 'Account', icon: User },
  { id: 'notifications', label: 'Notifications', icon: Bell },
  { id: 'date-time', label: 'Date & Time', icon: Clock },
  { id: 'appearance', label: 'Appearance', icon: Palette },
  { id: 'shortcuts', label: 'Shortcuts', icon: Keyboard },
  { id: 'about', label: 'About', icon: Info },
]

export const tabId = (id: SettingsTab): string => `settings-tab-${id}`
export const panelId = (id: SettingsTab): string => `settings-panel-${id}`

interface Props {
  selected: SettingsTab
  onSelect: (tab: SettingsTab) => void
}

/** A vertical tablist: one tab stop, arrows/Home/End move and select (the panel is cheap, so selection follows focus). */
export function SettingsNav({ selected, onSelect }: Props) {
  const move = (delta: number | 'first' | 'last'): void => {
    const at = TAB_META.findIndex((t) => t.id === selected)
    const next = delta === 'first' ? 0 : delta === 'last' ? TAB_META.length - 1 : (at + delta + TAB_META.length) % TAB_META.length
    const tab = TAB_META[next]
    if (!tab) return
    onSelect(tab.id)
    requestAnimationFrame(() => document.getElementById(tabId(tab.id))?.focus())
  }
  return (
    <div
      role="tablist"
      aria-label="Settings"
      aria-orientation="vertical"
      className="flex flex-1 flex-col gap-0.5 overflow-y-auto px-3 pb-3"
      onKeyDown={(e) => {
        const key: Record<string, number | 'first' | 'last'> = { ArrowDown: 1, ArrowUp: -1, Home: 'first', End: 'last' }
        const delta = key[e.key]
        if (delta === undefined) return
        e.preventDefault()
        move(delta)
      }}
    >
      {TAB_META.map((t) => {
        const on = t.id === selected
        return (
          <button
            key={t.id}
            id={tabId(t.id)}
            type="button"
            role="tab"
            aria-selected={on}
            aria-controls={panelId(t.id)}
            tabIndex={on ? 0 : -1}
            data-autofocus={on || undefined} // the dialog opens with the current tab focused, not its close button
            onClick={() => onSelect(t.id)}
            className={`flex h-9 shrink-0 items-center gap-2.5 rounded-row px-2.5 text-left text-base outline-none focus-visible:ring-2 focus-visible:ring-primary ${on ? 'bg-selected font-semibold' : 'hover:bg-hover'}`}
          >
            <t.icon size={16} strokeWidth={1.5} className="shrink-0 text-grey" aria-hidden />
            <span className="truncate">{t.label}</span>
          </button>
        )
      })}
    </div>
  )
}
