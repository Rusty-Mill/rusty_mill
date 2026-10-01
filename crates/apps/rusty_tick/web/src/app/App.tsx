import { useEffect, useMemo, useState } from 'react'
import { RouterProvider } from 'react-router-dom'
import { usePrefs, applyTheme } from '@/features/settings/prefs'
import { getMode, getToken, setMode, type Mode } from './env'
import { Splash, TokenPrompt, Welcome } from './Gate'
import { createRouter } from './router'
import { createServices, ServicesProvider, useData, type Services } from './services'

export function App() {
  const [mode, setModeState] = useState<Mode | null>(getMode)
  // Bumped after a token is entered so the services (and their store) are built afresh.
  const [epoch, setEpoch] = useState(0)

  useEffect(() => applyTheme(usePrefs.getState().prefs.theme), [])

  if (!mode) return <Welcome onChoose={setModeState} />
  if (mode === 'server' && !getToken()) {
    return <TokenPrompt onSubmit={() => setEpoch((e) => e + 1)} onDemo={() => setModeState('demo')} />
  }
  return <Connected key={`${mode}:${epoch}`} mode={mode} onRetry={() => setEpoch((e) => e + 1)} onDemo={() => { setMode('demo'); setModeState('demo') }} />
}

function Connected({ mode, onRetry, onDemo }: { mode: Mode; onRetry: () => void; onDemo: () => void }) {
  const services = useMemo(() => createServices(mode), [mode])
  useEffect(() => {
    const { store, api } = services
    usePrefs.getState().bind(api)
    void store.getState().boot().then(() => usePrefs.getState().load(api))
    const stop = store.getState().startBackgroundSync()
    return () => {
      stop()
      usePrefs.getState().bind(null)
    }
  }, [services])
  return (
    <ServicesProvider services={services}>
      <Ready services={services} onRetry={onRetry} onDemo={onDemo} />
    </ServicesProvider>
  )
}

function Ready({ services, onRetry, onDemo }: { services: Services; onRetry: () => void; onDemo: () => void }) {
  const status = useData((s) => s.status)
  const error = useData((s) => s.error)
  const router = useMemo(createRouter, [])
  if (status === 'loading') return <Splash>Loading…</Splash>
  if (status === 'unauthorized') {
    // The prompt stores the new token itself; retrying only rebuilds the services with it (clearing it here would discard it).
    return <TokenPrompt error={error ?? 'The server did not accept that token.'} onSubmit={onRetry} onDemo={onDemo} />
  }
  if (status === 'error') {
    return (
      <Splash>
        <div className="flex flex-col items-center gap-3">
          <p role="alert">Could not reach the server{error ? `: ${error}` : ''}</p>
          <button type="button" className="h-9 rounded-row bg-primary px-4 text-white" onClick={() => services.store.getState().retryNow()}>
            Try again
          </button>
          <button type="button" className="text-s underline" onClick={onDemo}>
            Use sample data instead
          </button>
        </div>
      </Splash>
    )
  }
  return <RouterProvider router={router} />
}
