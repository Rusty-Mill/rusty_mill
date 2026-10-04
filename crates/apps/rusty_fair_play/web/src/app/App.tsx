import { useEffect, useMemo, useState } from 'react'
import { RouterProvider } from 'react-router-dom'
import { getMode } from './env'
import { Splash, TokenPrompt } from './Gate'
import { createRouter } from './router'
import { createServices, ServicesProvider, useData, type Services } from './services'
import { startTheme } from './theme'

/** Boots straight into the app; a 401 from the server brings up the token prompt, and a new token rebuilds the services. */
export function App() {
  const mode = useMemo(getMode, [])
  const [epoch, setEpoch] = useState(0)
  useEffect(startTheme, [])
  return <Connected key={`${mode}:${epoch}`} mode={mode} onRetry={() => setEpoch((e) => e + 1)} />
}

function Connected({ mode, onRetry }: { mode: 'server' | 'demo'; onRetry: () => void }) {
  const services = useMemo(() => createServices(mode), [mode])
  useEffect(() => {
    const { store } = services
    void store.getState().boot()
    return store.getState().startBackgroundSync()
  }, [services])
  return (
    <ServicesProvider services={services}>
      <Ready services={services} onRetry={onRetry} />
    </ServicesProvider>
  )
}

function Ready({ services, onRetry }: { services: Services; onRetry: () => void }) {
  const status = useData((s) => s.status)
  const error = useData((s) => s.error)
  const router = useMemo(createRouter, [])
  if (status === 'loading') return <Splash>Loading…</Splash>
  if (status === 'unauthorized') {
    // The prompt stores the new token itself; retrying rebuilds the services with it.
    return <TokenPrompt error={error ?? 'The server did not accept that token.'} onSubmit={onRetry} />
  }
  if (status === 'error') {
    return (
      <Splash>
        <div className="flex flex-col items-center gap-3">
          <p role="alert">Could not reach the server{error ? `: ${error}` : ''}</p>
          <button type="button" className="h-9 rounded-row bg-primary px-4 text-white" onClick={() => void services.store.getState().boot()}>
            Try again
          </button>
        </div>
      </Splash>
    )
  }
  return <RouterProvider router={router} />
}
