import { useSearchParams } from 'react-router-dom'
import { SETTINGS_TABS, type SettingsTab } from '@/app/paths'
import { Dialog } from '@/components/Dialog'
import { SettingsNav, panelId, tabId } from './SettingsNav'
import { AboutTab } from './tabs/AboutTab'
import { AccountTab } from './tabs/AccountTab'
import { AppearanceTab } from './tabs/AppearanceTab'
import { DateTimeTab } from './tabs/DateTimeTab'
import { NotificationsTab } from './tabs/NotificationsTab'
import { ShortcutsTab } from './tabs/ShortcutsTab'

const isTab = (v: string | null): v is SettingsTab => SETTINGS_TABS.some((t) => t === v)

function Pane({ tab }: { tab: SettingsTab }) {
  switch (tab) {
    case 'account':
      return <AccountTab />
    case 'notifications':
      return <NotificationsTab />
    case 'date-time':
      return <DateTimeTab />
    case 'appearance':
      return <AppearanceTab />
    case 'shortcuts':
      return <ShortcutsTab />
    case 'about':
      return <AboutTab />
  }
}

/** `?modalType=settings&tabs=…` over the current route: the URL says whether it is open and which tab shows. */
export function SettingsModal() {
  const [params, setParams] = useSearchParams()
  const open = params.get('modalType') === 'settings'
  const requested = params.get('tabs')
  const tab: SettingsTab = isTab(requested) ? requested : 'account'

  const edit = (fn: (p: URLSearchParams) => void): void =>
    setParams(
      (prev) => {
        const p = new URLSearchParams(prev)
        fn(p)
        return p
      },
      { replace: true }, // switching tabs is not a place to go back to
    )

  return (
    <Dialog open={open} onClose={() => edit((p) => (p.delete('modalType'), p.delete('tabs')))} labelledBy="settings-title" width={780} height={680}>
      <div className="flex min-h-0 flex-1">
        <div className="flex w-[228px] shrink-0 flex-col border-r border-line bg-side">
          <h2 id="settings-title" className="px-6 pb-3 pt-5 text-title font-semibold">
            Settings
          </h2>
          <SettingsNav selected={tab} onSelect={(t) => edit((p) => p.set('tabs', t))} />
        </div>
        <div role="tabpanel" id={panelId(tab)} aria-labelledby={tabId(tab)} tabIndex={-1} className="min-w-0 flex-1 overflow-y-auto px-8 pb-6 pt-6 outline-none">
          <Pane tab={tab} />
        </div>
      </div>
    </Dialog>
  )
}
