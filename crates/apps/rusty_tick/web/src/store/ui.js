/**
 * View state that is not data: what is selected, what is collapsed, how each
 * view is sorted. Small, global, and partly remembered across reloads.
 */
import { create } from 'zustand';
import { defaultOptions, viewKey } from '@/features/tasks/organize';
const KEY = 'tick-local:ui:v1';
const load = () => {
    try {
        const raw = localStorage.getItem(KEY);
        if (raw)
            return { sidebarCollapsed: false, sections: {}, options: {}, ...JSON.parse(raw) };
    }
    catch {
        /* unreadable: start fresh */
    }
    return { sidebarCollapsed: false, sections: {}, options: {} };
};
export const useUi = create()((set, get) => {
    const persist = () => {
        const { sidebarCollapsed, sections, options } = get();
        try {
            localStorage.setItem(KEY, JSON.stringify({ sidebarCollapsed, sections, options }));
        }
        catch {
            /* storage unavailable: the choice lasts until reload */
        }
    };
    return {
        ...load(),
        drawerOpen: false,
        toggleDrawer: () => set((s) => ({ drawerOpen: !s.drawerOpen })),
        closeDrawer: () => set({ drawerOpen: false }),
        quickAddFocus: 0,
        searchOpen: false,
        assistantOpen: false,
        toggleAssistant: () => set((s) => ({ assistantOpen: !s.assistantOpen })),
        closeAssistant: () => set({ assistantOpen: false }),
        collapsedGroups: {},
        toggleSidebar: () => {
            set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed }));
            persist();
        },
        toggleSection: (id) => {
            set((s) => ({ sections: { ...s.sections, [id]: !s.sections[id] } }));
            persist();
        },
        isSectionCollapsed: (id) => !!get().sections[id],
        focusQuickAdd: () => set((s) => ({ quickAddFocus: s.quickAddFocus + 1 })),
        setSearchOpen: (searchOpen) => set({ searchOpen }),
        toggleGroup: (key) => set((s) => ({ collapsedGroups: { ...s.collapsedGroups, [key]: !s.collapsedGroups[key] } })),
        optionsFor: (spec) => ({ ...defaultOptions(spec), ...get().options[viewKey(spec)] }),
        setOptions: (spec, patch) => {
            const key = viewKey(spec);
            set((s) => ({ options: { ...s.options, [key]: { ...s.options[key], ...patch } } }));
            persist();
        },
    };
});
