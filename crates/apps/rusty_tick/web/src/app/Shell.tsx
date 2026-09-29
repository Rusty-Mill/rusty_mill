import { Outlet } from 'react-router-dom'
import { IconRail } from '@/components/IconRail'
import { Toasts } from '@/components/Toasts'
import { SearchModal } from '@/features/search/SearchModal'
import { SettingsModal } from '@/features/settings/SettingsModal'
import { Sidebar } from '@/features/lists/Sidebar'

/** Rail + whatever the route shows, with the app-wide overlays. */
export function Shell() {
  return (
    <div className="flex h-full w-full">
      <IconRail />
      <Outlet />
      <SearchModal />
      <SettingsModal />
      <Toasts />
    </div>
  )
}

/** The layout for task-ish routes: the sidebar, then the route's own columns. */
export function WithSidebar() {
  return (
    <>
      <Sidebar />
      <div className="flex min-w-0 flex-1 bg-surface">
        <Outlet />
      </div>
    </>
  )
}
