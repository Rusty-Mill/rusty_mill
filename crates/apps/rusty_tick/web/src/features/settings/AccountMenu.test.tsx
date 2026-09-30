import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { MemoryRouter, useLocation } from 'react-router-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { STORAGE_KEY } from '@/api/memory'
import { getMode, getToken, setMode, setToken, type Mode } from '@/app/env'
import { createServices, ServicesProvider, type Services } from '@/app/services'
import { atTime } from '@/lib/date'
import { AccountMenu } from './AccountMenu'
import { page } from './session'

function Host() {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const [open, setOpen] = useState(false)
  return (
    <>
      <button ref={setAnchor} aria-label="Account" onClick={() => setOpen(true)} />
      <AccountMenu anchor={anchor} open={open} onClose={() => setOpen(false)} />
      <output data-testid="where">{useLocation().search}</output>
    </>
  )
}

async function setup(mode: Mode = 'demo', prepare?: (s: Services) => Promise<void>) {
  const services = createServices(mode)
  await prepare?.(services)
  const user = userEvent.setup()
  render(
    <ServicesProvider services={services}>
      <MemoryRouter>
        <Host />
      </MemoryRouter>
    </ServicesProvider>,
  )
  await user.click(screen.getByRole('button', { name: 'Account' }))
  return user
}

afterEach(() => vi.restoreAllMocks())

describe('AccountMenu', () => {
  it('lists Settings, Statistics and Sign Out', async () => {
    await setup('server')
    const menu = screen.getByRole('menu', { name: 'Account' })
    expect(within(menu).getAllByRole('menuitem').map((i) => i.textContent)).toEqual(['Settings', 'Statistics', 'Sign Out'])
  })

  it('Settings goes to the account tab', async () => {
    const user = await setup()
    await user.click(screen.getByRole('menuitem', { name: 'Settings' }))
    expect(screen.getByTestId('where')).toHaveTextContent('?modalType=settings&tabs=account')
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('Statistics opens a dialog of real numbers', async () => {
    const user = await setup('demo', async (s) => {
      await s.store.getState().boot()
      const a = s.store.getState()
      for (const id of Object.keys(a.tasks)) await a.trashTask(id) // start from a clean slate, not the sample data
      const t = await a.createTask({ listId: a.inboxId, title: 'one', dueMs: atTime(Date.now(), 12) })
      await a.toggleDone(t.id)
      await a.createTask({ listId: a.inboxId, title: 'two', dueMs: Date.now() - 3 * 86_400_000, isAllDay: true })
    })
    await user.click(screen.getByRole('menuitem', { name: 'Statistics' }))
    const dialog = screen.getByRole('dialog', { name: 'Statistics' })
    const figure = (label: string): string => within(dialog).getByText(label).previousElementSibling!.textContent ?? ''
    expect(figure('Completed today')).toBe('1')
    expect(figure('Completed this week')).toBe('1')
    expect(figure('Completed all time')).toBe('1')
    expect(figure('Overdue')).toBe('1')
    expect(figure('Open tasks')).toBe('1')
    expect(figure('Day streak')).toBe('1')
    expect(within(dialog).getByText('Inbox')).toBeInTheDocument()
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('Sign Out (server) clears the token and reloads', async () => {
    setToken('tok-1234567890', true)
    const reload = vi.spyOn(page, 'reload').mockImplementation(() => undefined)
    const user = await setup('server')
    await user.click(screen.getByRole('menuitem', { name: 'Sign Out' }))
    expect(getToken()).toBeNull()
    expect(reload).toHaveBeenCalledOnce()
  })

  it('Leave demo forgets the mode so the welcome screen returns, and reloads without ?adapter', async () => {
    setMode('demo')
    const replace = vi.spyOn(page, 'replace').mockImplementation(() => undefined)
    const user = await setup('demo')
    localStorage.setItem(STORAGE_KEY, '{}')
    expect(screen.queryByRole('menuitem', { name: 'Sign Out' })).toBeNull()
    await user.click(screen.getByRole('menuitem', { name: 'Leave demo' }))
    expect(getMode()).toBeNull()
    expect(replace).toHaveBeenCalledOnce()
    expect(localStorage.getItem(STORAGE_KEY)).not.toBeNull() // leaving is not deleting
  })
})
