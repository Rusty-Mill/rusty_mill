import { createContext, useContext, useMemo, type ReactNode } from 'react'
import { useStore } from 'zustand'
import type { ApiClient } from '@/api/client'
import { HttpAdapter } from '@/api/http'
import { MemoryAdapter } from '@/api/memory'
import { seedData } from '@/api/seed'
import { prefixStorage } from '@/lib/storage'
import { createDataStore, type DataActions, type DataState, type DataStore } from '@/store/data'
import { getToken, type Mode } from './env'

export interface Services {
  mode: Mode
  api: ApiClient
  store: DataStore
}

/** Wire an API and a data store for `mode`. */
export function createServices(mode: Mode): Services {
  if (mode === 'demo') {
    const api = new MemoryAdapter({ storage: localStorage, seed: seedData })
    return { mode, api, store: createDataStore({ api }) } // the adapter already persists; no queue needed
  }
  const api = new HttpAdapter({ getToken })
  return { mode, api, store: createDataStore({ api, storage: prefixStorage(localStorage, 'server:') }) }
}

const Ctx = createContext<Services | null>(null)

export function ServicesProvider({ services, children }: { services: Services; children: ReactNode }) {
  return <Ctx.Provider value={services}>{children}</Ctx.Provider>
}

export function useServices(): Services {
  const s = useContext(Ctx)
  if (!s) throw new Error('useServices outside ServicesProvider')
  return s
}

/** Select from the data store (state and actions). */
export function useData<T>(selector: (s: DataState & DataActions) => T): T {
  return useStore(useServices().store, selector)
}

/** The store's actions. They are created once, so the object is stable. */
export function useActions(): DataActions {
  const { store } = useServices()
  return useMemo(() => store.getState(), [store])
}
