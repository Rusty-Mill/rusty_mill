import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter, useLocation } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { STORAGE_KEY } from '@/api/memory'
import { SETTINGS_TABS } from '@/app/paths'
import { getToken, setToken } from '@/app/env'
import { createServices, ServicesProvider } from '@/app/services'
import type { Mode } from '@/app/env'
import { useUi } from '@/store/ui'
import { DEFAULT_PREFS, usePrefs } from './prefs'
import { page } from './session'
import { SettingsModal } from './SettingsModal'

const Where = () => <output data-testid="where">{useLocation().search}</output>

function setup(entry = '/?modalType=settings&tabs=account', mode: Mode = 'demo') {
  const user = userEvent.setup()
  render(
    <ServicesProvider services={createServices(mode)}>
      <MemoryRouter initialEntries={[entry]}>
        <SettingsModal />
        <Where />
      </MemoryRouter>
    </ServicesProvider>,
  )
  return user
}
const search = (): string => screen.getByTestId('where').textContent ?? ''

beforeEach(() => {
  usePrefs.setState({ prefs: DEFAULT_PREFS })
  useUi.setState({ sidebarCollapsed: false })
  delete document.documentElement.dataset.theme
})
afterEach(() => vi.restoreAllMocks())

describe('SettingsModal', () => {
  it('is closed unless the URL asks for it', () => {
    setup('/')
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('opens from the URL on the tab it names, sized 780x680', () => {
    setup('/?modalType=settings&tabs=date-time')
    const dialog = screen.getByRole('dialog', { name: 'Settings' })
    expect(dialog).toHaveStyle({ width: '780px', height: '680px' })
    expect(screen.getByRole('tab', { name: 'Date & Time' })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByRole('tabpanel')).toHaveTextContent('Week starts on')
  })

  it('falls back to Account for a missing or unknown tab', () => {
    setup('/?modalType=settings&tabs=nonsense')
    expect(screen.getByRole('tab', { name: 'Account', selected: true })).toBeInTheDocument()
  })

  it('has the tabs in order, as a vertical tablist with a single tab stop', () => {
    setup()
    const list = screen.getByRole('tablist', { name: 'Settings' })
    expect(list).toHaveAttribute('aria-orientation', 'vertical')
    const tabs = within(list).getAllByRole('tab')
    expect(tabs.map((t) => t.textContent)).toEqual(['Account', 'Premium', 'Features', 'Smart List', 'Notifications', 'Date & Time', 'Appearance', 'AI Features', 'More', 'Integrations & Import', 'Collaborate', 'Shortcuts', 'About'])
    expect(tabs).toHaveLength(SETTINGS_TABS.length)
    expect(tabs.filter((t) => t.tabIndex === 0)).toHaveLength(1)
  })

  it('switches tab on click and puts it in the URL', async () => {
    const user = setup()
    await user.click(screen.getByRole('tab', { name: 'Appearance' }))
    expect(search()).toBe('?modalType=settings&tabs=appearance')
    expect(screen.getByRole('tab', { name: 'Appearance', selected: true })).toBeInTheDocument()
    expect(screen.getByRole('tabpanel')).toHaveAccessibleName('Appearance')
  })

  it('moves with the arrow keys, wrapping, and Home/End', async () => {
    const user = setup()
    screen.getByRole('tab', { name: 'Account' }).focus()
    await user.keyboard('{ArrowDown}')
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Premium' })).toHaveFocus())
    expect(search()).toContain('tabs=premium')
    await user.keyboard('{ArrowUp}{ArrowUp}')
    await waitFor(() => expect(screen.getByRole('tab', { name: 'About' })).toHaveFocus())
    expect(search()).toContain('tabs=about')
    await user.keyboard('{Home}')
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Account' })).toHaveFocus())
    await user.keyboard('{End}')
    await waitFor(() => expect(screen.getByRole('tab', { name: 'About' })).toHaveFocus())
  })

  it('closes with Escape or the button, removing its parameters but keeping others', async () => {
    const user = setup('/?x=1&modalType=settings&tabs=about')
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(search()).toBe('?x=1')
  })

  it.each(['premium', 'features', 'smart-list', 'notifications', 'ai', 'more', 'integrations', 'collaborate'])('%s is an honest placeholder', (tab) => {
    setup(`/?modalType=settings&tabs=${tab}`)
    const panel = screen.getByRole('tabpanel')
    expect(within(panel).getByRole('heading')).toBeInTheDocument()
    expect(panel).toHaveTextContent('is not available in Tick Local.')
    expect(within(panel).queryByRole('button')).toBeNull() // nothing to click, nothing to buy
  })

  it('About shows the app name, a description and the version', () => {
    setup('/?modalType=settings&tabs=about')
    const panel = screen.getByRole('tabpanel')
    expect(panel).toHaveTextContent('Tick Local')
    expect(panel).toHaveTextContent(/Version \d+\.\d+\.\d+/)
  })

  describe('Date & Time', () => {
    it('sets the week start, time format and default reminder in the prefs', async () => {
      const user = setup('/?modalType=settings&tabs=date-time')
      expect(screen.getByRole('radio', { name: 'Monday' })).toBeChecked()
      await user.click(screen.getByRole('radio', { name: 'Sunday' }))
      expect(usePrefs.getState().prefs.weekStart).toBe(0)
      await user.click(screen.getByRole('radio', { name: 'Saturday' }))
      expect(usePrefs.getState().prefs.weekStart).toBe(6)
      await user.click(screen.getByRole('radio', { name: '12-hour' }))
      expect(usePrefs.getState().prefs.hour12).toBe(true)
      await user.click(screen.getByRole('radio', { name: '24-hour' }))
      expect(usePrefs.getState().prefs.hour12).toBe(false)
      await user.selectOptions(screen.getByLabelText('Default reminder'), 'TRIGGER:-PT1H')
      expect(usePrefs.getState().prefs.defaultReminder).toBe('TRIGGER:-PT1H')
      await user.selectOptions(screen.getByLabelText('Default reminder'), '')
      expect(usePrefs.getState().prefs.defaultReminder).toBe('')
    })
  })

  describe('Appearance', () => {
    it('applies the theme at once', async () => {
      const user = setup('/?modalType=settings&tabs=appearance')
      await user.click(screen.getByRole('radio', { name: /Dark/ }))
      expect(usePrefs.getState().prefs.theme).toBe('dark')
      expect(document.documentElement.dataset.theme).toBe('dark')
      await user.click(screen.getByRole('radio', { name: /Light/ }))
      expect(document.documentElement.dataset.theme).toBe('light')
      await user.click(screen.getByRole('radio', { name: /System/ }))
      expect(usePrefs.getState().prefs.theme).toBe('system')
    })

    it('shows and hides the sidebar', async () => {
      const user = setup('/?modalType=settings&tabs=appearance')
      const toggle = screen.getByRole('switch', { name: 'Show sidebar' })
      expect(toggle).toBeChecked()
      await user.click(toggle)
      expect(useUi.getState().sidebarCollapsed).toBe(true)
      expect(toggle).not.toBeChecked()
    })
  })

  describe('Shortcuts', () => {
    it('lists the keys in a two-column table', () => {
      setup('/?modalType=settings&tabs=shortcuts')
      const tables = screen.getAllByRole('table')
      const rows = tables.flatMap((t) => within(t).getAllByRole('row'))
      const text = rows.map((r) => r.textContent).join('|')
      for (const expected of ['N', 'J', 'K', 'Space', 'Delete', 'Ctrl', '⌘', 'Esc', '1–4', 'Set priority']) expect(text).toContain(expected)
      expect(within(tables[0]!).getAllByRole('columnheader')).toHaveLength(2)
    })
  })

  describe('Account', () => {
    it('server mode: shows the masked token and signs out', async () => {
      setToken('secret-token-abcd1234', false)
      const reload = vi.spyOn(page, 'reload').mockImplementation(() => undefined)
      const user = setup('/?modalType=settings&tabs=account', 'server')
      const panel = screen.getByRole('tabpanel')
      expect(panel).toHaveTextContent('Connected to')
      expect(panel).toHaveTextContent('••••••••1234')
      expect(panel).not.toHaveTextContent('secret-token')
      await user.click(screen.getByRole('button', { name: 'Sign out' }))
      expect(getToken()).toBeNull()
      expect(reload).toHaveBeenCalledOnce()
    })

    it('demo mode: explains where the data lives and resets it after confirming', async () => {
      localStorage.setItem(STORAGE_KEY, '{"lists":[]}')
      localStorage.setItem('tick-local:prefs:v1', '{}')
      const reload = vi.spyOn(page, 'reload').mockImplementation(() => undefined)
      const user = setup()
      expect(screen.getByRole('tabpanel')).toHaveTextContent('stored in this browser')
      await user.click(screen.getByRole('button', { name: 'Reset sample data' }))
      expect(localStorage.getItem(STORAGE_KEY)).not.toBeNull() // not until confirmed
      await user.click(screen.getByRole('button', { name: 'Cancel' }))
      expect(localStorage.getItem(STORAGE_KEY)).not.toBeNull()
      await user.click(screen.getByRole('button', { name: 'Reset sample data' }))
      await user.click(screen.getByRole('button', { name: 'Reset' }))
      expect(localStorage.getItem(STORAGE_KEY)).toBeNull()
      expect(localStorage.getItem('tick-local:prefs:v1')).not.toBeNull() // preferences are not sample data
      expect(reload).toHaveBeenCalledOnce()
    })

    it('does not show a sign-out in demo mode', () => {
      setup()
      expect(screen.queryByRole('button', { name: 'Sign out' })).toBeNull()
    })
  })
})
