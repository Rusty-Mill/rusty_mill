import { render } from '@testing-library/react'
import { createMemoryRouter, RouterProvider } from 'react-router-dom'
import { MemoryAdapter } from '@/api/memory'
import { routes } from '@/app/router'
import { ServicesProvider, type Services } from '@/app/services'
import { createDataStore } from '@/store/data'

export interface Harness {
  api: MemoryAdapter
  services: Services
  router: ReturnType<typeof createMemoryRouter>
}

/**
 * Render the whole app at `route` over an in-memory backend that `seed` may
 * fill first. The real routes, store and components; only the network is faked.
 */
export async function renderApp(route = '/q/all/tasks', seed?: (api: MemoryAdapter) => Promise<void>): Promise<Harness> {
  const api = new MemoryAdapter()
  await seed?.(api)
  const store = createDataStore({ api })
  await store.getState().boot()
  const services: Services = { mode: 'demo', api, store }
  const router = createMemoryRouter(routes, { initialEntries: [route] })
  render(
    <ServicesProvider services={services}>
      <RouterProvider router={router} />
    </ServicesProvider>,
  )
  return { api, services, router }
}
