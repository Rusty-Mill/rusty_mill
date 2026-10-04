import { useEffect } from 'react'
import { Outlet } from 'react-router-dom'
import { Rail } from '@/components/Rail'
import { Toasts } from '@/components/Toasts'
import { useUi } from '@/store/ui'

/** The rail plus whatever the route shows, with the app-wide overlays and shortcuts. */
export function Shell() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return
      if (document.querySelector('[role="dialog"]')) return
      e.preventDefault()
      useUi.getState().focusSearch()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])
  return (
    <div className="flex h-full w-full">
      <Rail />
      <div className="relative flex min-w-0 flex-1 bg-surface">
        <Outlet />
      </div>
      <Toasts />
    </div>
  )
}
