import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createServices, ServicesProvider } from '@/app/services'
import { HabitsPage } from './HabitsPage'
import { checkinId } from './logic'
import { resetHabitsStore } from './store'

// Tuesday, Sep 29 2026.
const NOW = new Date(2026, 8, 29, 10, 0, 0)

async function setup() {
  const services = createServices('demo')
  await act(async () => {
    render(
      <ServicesProvider services={services}>
        <HabitsPage />
      </ServicesProvider>,
    )
  })
  return { services, user: userEvent.setup() }
}

async function addHabit(user: ReturnType<typeof userEvent.setup>, name: string) {
  const dialog = await screen.findByRole('dialog', { name: 'Create Habit' })
  await user.type(within(dialog).getByPlaceholderText('Habit name'), name)
  await user.click(within(dialog).getByRole('button', { name: 'Save' }))
}

describe('HabitsPage', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(NOW)
    resetHabitsStore()
  })
  afterEach(() => {
    cleanup()
    resetHabitsStore()
    vi.useRealTimers()
  })

  it('shows the empty state with a way to add a habit', async () => {
    const { user } = await setup()
    expect(screen.getByText('Develop a habit')).toBeInTheDocument()
    expect(screen.getByText('Every little bit counts')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    expect(screen.getByRole('dialog', { name: 'Create Habit' })).toBeInTheDocument()
  })

  it('adds a habit, checks in and unchecks, persisting one doc per day', async () => {
    const { user, services } = await setup()
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    await addHabit(user, 'Read')
    expect(screen.queryByText('Develop a habit')).toBeNull()
    expect(screen.getByText('Read')).toBeInTheDocument()

    const cell = screen.getByRole('button', { name: 'Read, Tuesday Sep 29, not done' })
    await user.click(cell)
    expect(screen.getByRole('button', { name: 'Read, Tuesday Sep 29, done' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByText('1 day')).toBeInTheDocument()

    const habitId = (await services.api.listDocs('habit'))[0]!.id
    await waitFor(async () => expect(await services.api.listDocs('habit_checkin')).toHaveLength(1))
    const docs = await services.api.listDocs('habit_checkin')
    expect(docs[0]!.id).toBe(checkinId(habitId, '2026-09-29'))
    expect(docs[0]!.body).toEqual({ habitId, day: '2026-09-29', count: 1 })

    await user.click(screen.getByRole('button', { name: 'Read, Tuesday Sep 29, done' }))
    expect(screen.getByRole('button', { name: 'Read, Tuesday Sep 29, not done' })).toBeInTheDocument()
    await waitFor(async () => expect(await services.api.listDocs('habit_checkin')).toHaveLength(0))
  })

  it('cannot check in a future day', async () => {
    const { user } = await setup()
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    await addHabit(user, 'Read')
    expect(screen.getByRole('button', { name: 'Read, Wednesday Sep 30, not done' })).toBeDisabled()
  })

  it('navigates weeks', async () => {
    const { user } = await setup()
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    await addHabit(user, 'Read')
    await user.click(screen.getByRole('button', { name: 'Previous week' }))
    expect(screen.getByRole('button', { name: 'Read, Tuesday Sep 22, not done' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'This week' }))
    expect(screen.getByRole('button', { name: 'Read, Tuesday Sep 29, not done' })).toBeInTheDocument()
  })

  it('edits a habit from the row menu', async () => {
    const { user, services } = await setup()
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    await addHabit(user, 'Read')
    await user.click(screen.getByRole('button', { name: 'More actions for Read' }))
    await user.click(screen.getByRole('menuitem', { name: 'Edit' }))
    const dialog = screen.getByRole('dialog', { name: 'Edit Habit' })
    const name = within(dialog).getByPlaceholderText('Habit name')
    await user.clear(name)
    await user.type(name, 'Write')
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Frequency' }), 'perWeek')
    await user.click(within(dialog).getByRole('button', { name: 'Save' }))
    expect(screen.getByText('Write')).toBeInTheDocument()
    expect(screen.queryByText('Read')).toBeNull()
    const body = (await services.api.listDocs<{ name: string; frequency: unknown }>('habit'))[0]!.body
    expect(body.name).toBe('Write')
    expect(body.frequency).toEqual({ kind: 'perWeek', times: 3 })
  })

  it('deletes a habit after confirming, with its check-ins', async () => {
    const { user, services } = await setup()
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    await addHabit(user, 'Read')
    await user.click(screen.getByRole('button', { name: 'Read, Tuesday Sep 29, not done' }))
    await user.click(screen.getByRole('button', { name: 'More actions for Read' }))
    await user.click(screen.getByRole('menuitem', { name: 'Delete' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Delete' }))
    expect(screen.getByText('Develop a habit')).toBeInTheDocument()
    await waitFor(async () => {
      expect(await services.api.listDocs('habit')).toHaveLength(0)
      expect(await services.api.listDocs('habit_checkin')).toHaveLength(0)
    })
  })

  it('requires a name and, for specific days, at least one day', async () => {
    const { user } = await setup()
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    const dialog = screen.getByRole('dialog', { name: 'Create Habit' })
    expect(within(dialog).getByRole('button', { name: 'Save' })).toBeDisabled()
    fireEvent.change(within(dialog).getByPlaceholderText('Habit name'), { target: { value: 'Run' } })
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Frequency' }), 'weekdays')
    for (const d of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri']) await user.click(within(dialog).getByRole('button', { name: d }))
    expect(within(dialog).getByRole('button', { name: 'Save' })).toBeDisabled()
  })

  it('keeps the change and warns when saving fails', async () => {
    const { user, services } = await setup()
    vi.spyOn(services.api, 'putDoc').mockRejectedValue(new Error('offline'))
    await user.click(screen.getByRole('button', { name: 'Add habit' }))
    await addHabit(user, 'Read')
    expect(screen.getByText('Read')).toBeInTheDocument()
    await waitFor(() => expect(Object.values(services.store.getState().toasts).some((t) => t.kind === 'error')).toBe(true))
  })
})
