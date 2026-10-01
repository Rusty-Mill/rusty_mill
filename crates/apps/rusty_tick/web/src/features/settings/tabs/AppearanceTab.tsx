import { useUi } from '@/store/ui'
import { usePrefs, type Theme } from '../prefs'
import { PaneTitle, Row, Switch } from './controls'

const THEMES: { value: Theme; label: string }[] = [
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
  { value: 'system', label: 'System' },
]

/** A miniature window in a theme's own colours (fixed values: it must show that theme, whichever one is active). */
function Preview({ theme }: { theme: 'light' | 'dark' }) {
  const c = theme === 'dark' ? { bg: '#1a1a1c', side: '#1e1e20', line: '#303032', bar: '#8c8c8c' } : { bg: '#ffffff', side: '#f7f7f7', line: '#f0f1f2', bar: '#c8c8c8' }
  return (
    <span aria-hidden className="flex h-[54px] w-full overflow-hidden rounded-md border" style={{ background: c.bg, borderColor: c.line }}>
      <span className="w-[28%]" style={{ background: c.side }} />
      <span className="flex flex-1 flex-col gap-1.5 p-2">
        <span className="h-1.5 w-3/4 rounded-full" style={{ background: c.bar }} />
        <span className="h-1.5 w-1/2 rounded-full" style={{ background: c.bar }} />
        <span className="h-1.5 w-2/3 rounded-full" style={{ background: '#4772FA' }} />
      </span>
    </span>
  )
}

export function AppearanceTab() {
  const theme = usePrefs((s) => s.prefs.theme)
  const update = usePrefs((s) => s.update)
  const collapsed = useUi((s) => s.sidebarCollapsed)
  const toggleSidebar = useUi((s) => s.toggleSidebar)
  return (
    <>
      <PaneTitle>Appearance</PaneTitle>
      <div role="radiogroup" aria-label="Theme" className="grid grid-cols-3 gap-3 border-b border-line pb-5">
        {THEMES.map((t) => (
          <label key={t.value} className="cursor-pointer">
            <input type="radio" name="theme" checked={theme === t.value} onChange={() => update({ theme: t.value })} className="peer sr-only" />
            <span className="flex flex-col gap-2 rounded-row border border-line p-2 peer-checked:border-primary peer-checked:ring-1 peer-checked:ring-primary peer-focus-visible:ring-2 peer-focus-visible:ring-primary">
              {t.value === 'system' ? (
                <span aria-hidden className="flex gap-1">
                  <Preview theme="light" />
                  <Preview theme="dark" />
                </span>
              ) : (
                <Preview theme={t.value} />
              )}
              <span className="text-center text-base">{t.label}</span>
            </span>
          </label>
        ))}
      </div>
      <Row label="Show sidebar" hint="The list of smart lists, lists and tags beside your tasks." htmlFor="show-sidebar">
        <Switch id="show-sidebar" label="Show sidebar" checked={!collapsed} onChange={() => toggleSidebar()} />
      </Row>
    </>
  )
}
