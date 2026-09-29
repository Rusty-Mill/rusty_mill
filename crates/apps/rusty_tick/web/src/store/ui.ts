/**
 * View state that is not data: what is selected, what is collapsed, how each
 * view is sorted. Small, global, and partly remembered across reloads.
 */
import { create } from 'zustand'
import { defaultOptions, viewKey, type ViewOptions, type ViewSpec } from '@/features/tasks/organize'

const KEY = 'tick-local:ui:v1'

interface Persisted {
  sidebarCollapsed: boolean
  sections: Record<string, boolean> // section id -> collapsed
  options: Record<string, Partial<ViewOptions>>
}

const load = (): Persisted => {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return { sidebarCollapsed: false, sections: {}, options: {}, ...(JSON.parse(raw) as Partial<Persisted>) }
  } catch {
    /* unreadable: start fresh */
  }
  return { sidebarCollapsed: false, sections: {}, options: {} }
}

export interface UiState extends Persisted {
  /** The sidebar as a drawer over the content, on narrow screens. Never remembered. */
  drawerOpen: boolean
  toggleDrawer(): void
  closeDrawer(): void
  /** Bumped to ask the quick-add box to take focus (the `N` shortcut). */
  quickAddFocus: number
  searchOpen: boolean
  /** Group collapse state within the task list, keyed `${view}:${group}`. */
  collapsedGroups: Record<string, boolean>
  toggleSidebar(): void
  toggleSection(id: string): void
  isSectionCollapsed(id: string): boolean
  focusQuickAdd(): void
  setSearchOpen(open: boolean): void
  toggleGroup(key: string): void
  optionsFor(spec: ViewSpec): ViewOptions
  setOptions(spec: ViewSpec, patch: Partial<ViewOptions>): void
}

export const useUi = create<UiState>()((set, get) => {
  const persist = (): void => {
    const { sidebarCollapsed, sections, options } = get()
    try {
      localStorage.setItem(KEY, JSON.stringify({ sidebarCollapsed, sections, options }))
    } catch {
      /* storage unavailable: the choice lasts until reload */
    }
  }
  return {
    ...load(),
    drawerOpen: false,
    toggleDrawer: () => set((s) => ({ drawerOpen: !s.drawerOpen })),
    closeDrawer: () => set({ drawerOpen: false }),
    quickAddFocus: 0,
    searchOpen: false,
    collapsedGroups: {},
    toggleSidebar: () => {
      set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed }))
      persist()
    },
    toggleSection: (id) => {
      set((s) => ({ sections: { ...s.sections, [id]: !s.sections[id] } }))
      persist()
    },
    isSectionCollapsed: (id) => !!get().sections[id],
    focusQuickAdd: () => set((s) => ({ quickAddFocus: s.quickAddFocus + 1 })),
    setSearchOpen: (searchOpen) => set({ searchOpen }),
    toggleGroup: (key) => set((s) => ({ collapsedGroups: { ...s.collapsedGroups, [key]: !s.collapsedGroups[key] } })),
    optionsFor: (spec) => ({ ...defaultOptions(spec), ...get().options[viewKey(spec)] }),
    setOptions: (spec, patch) => {
      const key = viewKey(spec)
      set((s) => ({ options: { ...s.options, [key]: { ...s.options[key], ...patch } } }))
      persist()
    },
  }
})
