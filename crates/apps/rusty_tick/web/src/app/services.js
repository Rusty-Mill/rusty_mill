import { jsx as _jsx } from "react/jsx-runtime";
import { createContext, useContext, useMemo } from 'react';
import { useStore } from 'zustand';
import { HttpAdapter } from '@/api/http';
import { MemoryAdapter } from '@/api/memory';
import { seedData } from '@/api/seed';
import { prefixStorage } from '@/lib/storage';
import { createDataStore } from '@/store/data';
import { getToken } from './env';
/** Wire an API and a data store for `mode`. */
export function createServices(mode) {
    if (mode === 'demo') {
        const api = new MemoryAdapter({ storage: localStorage, seed: seedData });
        return { mode, api, store: createDataStore({ api }) }; // the adapter already persists; no queue needed
    }
    const api = new HttpAdapter({ getToken });
    return { mode, api, store: createDataStore({ api, storage: prefixStorage(localStorage, 'server:') }) };
}
const Ctx = createContext(null);
export function ServicesProvider({ services, children }) {
    return _jsx(Ctx.Provider, { value: services, children: children });
}
export function useServices() {
    const s = useContext(Ctx);
    if (!s)
        throw new Error('useServices outside ServicesProvider');
    return s;
}
/** Select from the data store (state and actions). */
export function useData(selector) {
    return useStore(useServices().store, selector);
}
/** The store's actions. They are created once, so the object is stable. */
export function useActions() {
    const { store } = useServices();
    return useMemo(() => store.getState(), [store]);
}
