import { createContext, useContext, useMemo, type ReactNode } from 'react'
import { useStore } from 'zustand'
import type { ApiClient } from '@/api/client'
import { HttpAdapter } from '@/api/http'
import { MemoryAdapter } from '@/api/memory'
import { createDataStore, type DataActions, type DataState, type DataStore } from '@/store/data'
import { getToken, type Mode } from './env'

export interface Services {
  mode: Mode
  api: ApiClient
  store: DataStore
}

/** Wire an API and a data store for `mode`. */
export function createServices(mode: Mode): Services {
  const api: ApiClient = mode === 'demo' ? new MemoryAdapter({ storage: localStorage }) : new HttpAdapter({ getToken })
  return { mode, api, store: createDataStore({ api }) }
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
