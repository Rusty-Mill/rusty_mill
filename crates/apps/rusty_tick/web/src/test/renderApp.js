import { jsx as _jsx } from "react/jsx-runtime";
import { render } from '@testing-library/react';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import { MemoryAdapter } from '@/api/memory';
import { routes } from '@/app/router';
import { ServicesProvider } from '@/app/services';
import { createDataStore } from '@/store/data';
/**
 * Render the whole app at `route` over an in-memory backend that `seed` may
 * fill first. The real routes, store and components; only the network is faked.
 */
export async function renderApp(route = '/q/all/tasks', seed) {
    const api = new MemoryAdapter();
    await seed?.(api);
    const store = createDataStore({ api });
    await store.getState().boot();
    const services = { mode: 'demo', api, store };
    const router = createMemoryRouter(routes, { initialEntries: [route] });
    render(_jsx(ServicesProvider, { services: services, children: _jsx(RouterProvider, { router: router }) }));
    return { api, services, router };
}
