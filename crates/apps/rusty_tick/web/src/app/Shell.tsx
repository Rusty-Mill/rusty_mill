import { useEffect } from 'react'
import { Outlet, useLocation } from 'react-router-dom'
import { useMedia } from '@/lib/hooks'
import { useUi } from '@/store/ui'
import { IconRail } from '@/components/IconRail'
import { Toasts } from '@/components/Toasts'
import { SearchModal } from '@/features/search/SearchModal'
import { SettingsModal } from '@/features/settings/SettingsModal'
import { Sidebar } from '@/features/lists/Sidebar'

/** Rail + whatever the route shows, with the app-wide overlays. */
export function Shell() {
  // Search is available everywhere, not just on task pages.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        useUi.getState().setSearchOpen(true)
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])
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

/** Below this width the sidebar leaves the layout and opens as a drawer over it. */
export const NARROW = '(max-width: 1099px)'

/** The layout for task-ish routes: the sidebar, then the route's own columns. */
export function WithSidebar() {
  const narrow = useMedia(NARROW)
  const collapsed = useUi((s) => s.sidebarCollapsed)
  const drawerOpen = useUi((s) => s.drawerOpen)
  const closeDrawer = useUi((s) => s.closeDrawer)
  const { pathname } = useLocation()

  // Choosing something in the drawer (or anything else that navigates) puts it away.
  useEffect(() => closeDrawer(), [pathname, closeDrawer])
  useEffect(() => {
    if (!drawerOpen) return
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') closeDrawer()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [drawerOpen, closeDrawer])

  const inline = !narrow && !collapsed
  return (
    <>
      {inline && <Sidebar />}
      {narrow && drawerOpen && (
        <>
          <div aria-hidden className="fixed inset-0 z-30 bg-black/30" onClick={closeDrawer} />
          <div className="pop-shadow fixed inset-y-0 left-[50px] z-40">
            <Sidebar />
          </div>
        </>
      )}
      <div className="relative flex min-w-0 flex-1 bg-surface">
        <Outlet />
      </div>
    </>
  )
}
