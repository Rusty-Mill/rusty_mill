import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { UnauthorizedError } from '@/api/errors'
import { HttpAdapter } from '@/api/http'
import { MemoryAdapter } from '@/api/memory'
import { App } from './App'
import { getToken } from './env'
import { useTheme } from './theme'

afterEach(() => {
  vi.restoreAllMocks()
  history.replaceState(null, '', '/')
  useTheme.setState({ theme: 'system' })
})

describe('App', () => {
  it('boots straight into the deck in demo mode (?adapter=memory)', async () => {
    history.replaceState(null, '', '/?adapter=memory')
    render(<App />)
    expect(await screen.findByRole('button', { name: 'Load the Fair Play deck' })).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Sections' })).toBeInTheDocument()
  })

  it('shows the token prompt on a 401, then retries with the token entered', async () => {
    const user = userEvent.setup()
    const tokens: (string | null)[] = []
    const backing = new MemoryAdapter()
    await backing.seed()
    vi.spyOn(HttpAdapter.prototype, 'snapshot').mockImplementation(function (this: HttpAdapter) {
      const token = (this as unknown as { options: { getToken: () => string | null } }).options.getToken()
      tokens.push(token)
      return token === 'secret-token-0123456789' ? backing.snapshot() : Promise.reject(new UnauthorizedError())
    })
    render(<App />)
    expect(await screen.findByLabelText('API token')).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('missing or invalid bearer token')
    await user.type(screen.getByLabelText('API token'), 'secret-token-0123456789')
    await user.click(screen.getByRole('checkbox', { name: 'Remember on this device' }))
    await user.click(screen.getByRole('button', { name: 'Connect' }))
    await waitFor(() => expect(screen.getAllByTestId('card-tile')).toHaveLength(100))
    expect(tokens).toEqual([null, 'secret-token-0123456789'])
    expect(getToken()).toBe('secret-token-0123456789')
    expect(localStorage.getItem('fair-play:token')).toBe('secret-token-0123456789')
  })

  it('shows an error with a retry when the server is unreachable', async () => {
    const user = userEvent.setup()
    const spy = vi.spyOn(HttpAdapter.prototype, 'snapshot').mockRejectedValue(new Error('offline'))
    render(<App />)
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not reach the server: offline')
    spy.mockResolvedValue({ people: [], cards: [] })
    await user.click(screen.getByRole('button', { name: 'Try again' }))
    expect(await screen.findByRole('button', { name: 'Load the Fair Play deck' })).toBeInTheDocument()
  })

  it('cycles the theme and writes data-theme', async () => {
    const user = userEvent.setup()
    history.replaceState(null, '', '/?adapter=memory')
    render(<App />)
    const button = await screen.findByRole('button', { name: /Theme: system/ })
    await user.click(button)
    expect(document.documentElement.dataset.theme).toBe('light')
    await user.click(screen.getByRole('button', { name: /Theme: light/ }))
    expect(document.documentElement.dataset.theme).toBe('dark')
    expect(localStorage.getItem('fair-play:theme')).toBe('dark')
  })
})
