import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom'
import { describe, expect, it } from 'vitest'
import { createServices, ServicesProvider, type Services } from '@/app/services'
import { startOfDay } from '@/lib/date'
import { CalendarPage } from './CalendarPage'
import { rangeTitle, visibleRange } from './layout'

function Where() {
  const l = useLocation()
  return <output data-testid="where">{l.pathname}</output>
}

/** A demo store with the sample tasks cleared, so only what a test adds is on the calendar. */
async function setup(path = '/c/all/calendar/m') {
  const services: Services = createServices('demo')
  await act(async () => {
    await services.store.getState().boot()
  })
  const s = services.store.getState()
  await act(async () => {
    for (const t of Object.values(s.tasks)) await s.trashTask(t.id)
  })
  const user = userEvent.setup()
  render(
    <ServicesProvider services={services}>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="c/all/calendar/:mode?" element={<CalendarPage />} />
          <Route path="*" element={null} />
        </Routes>
        <Where />
      </MemoryRouter>
    </ServicesProvider>,
  )
  return { services, user }
}

const now = () => Date.now()
const title = (mode: 'm' | 'w' | 'd' | 'a', anchor = now(), ws: 0 | 1 | 6 = 1) => rangeTitle(mode, anchor, visibleRange(mode, anchor, ws))

describe('CalendarPage', () => {
  it('shows the month with a weekday header and today marked', async () => {
    await setup()
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(title('m'))
    expect(screen.getAllByRole('columnheader')).toHaveLength(7)
    expect(screen.getAllByRole('gridcell')).toHaveLength(42)
    expect(screen.getByRole('gridcell', { selected: true })).toHaveAttribute('aria-label', expect.stringMatching(/, 0 tasks$/))
  })

  it('treats an unknown mode as month', async () => {
    await setup('/c/all/calendar/zzz')
    expect(screen.getByRole('grid', { name: 'Month' })).toBeInTheDocument()
  })

  it('switches views from the dropdown by navigating', async () => {
    const { user } = await setup()
    await user.click(screen.getByRole('button', { name: 'View: Month' }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Week' }))
    expect(screen.getByTestId('where')).toHaveTextContent('/c/all/calendar/w')
    expect(screen.getByRole('grid', { name: 'Week' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(title('w'))

    await user.click(screen.getByRole('button', { name: 'View: Week' }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Day' }))
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(title('d'))
    expect(screen.getByRole('grid', { name: 'Day' })).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'View: Day' }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Agenda' }))
    expect(screen.getByText('No tasks in this period')).toBeInTheDocument()
  })

  it('moves with prev, next and Today', async () => {
    const { user } = await setup()
    const h1 = screen.getByRole('heading', { level: 1 })
    const here = h1.textContent
    await user.click(screen.getByRole('button', { name: 'Next month' }))
    expect(h1.textContent).not.toBe(here)
    await user.click(screen.getByRole('button', { name: 'Next month' }))
    await user.click(screen.getByRole('button', { name: 'Previous month' }))
    await user.click(screen.getByRole('button', { name: 'Previous month' }))
    expect(h1).toHaveTextContent(here!)
    await user.click(screen.getByRole('button', { name: 'Previous month' }))
    await user.click(screen.getByRole('button', { name: 'Today' }))
    expect(h1).toHaveTextContent(here!)
  })

  it('steps by week in the week view', async () => {
    const { user } = await setup('/c/all/calendar/w')
    const h1 = screen.getByRole('heading', { level: 1 })
    const here = h1.textContent
    await user.click(screen.getByRole('button', { name: 'Next week' }))
    expect(h1.textContent).not.toBe(here)
    await user.click(screen.getByRole('button', { name: 'Today' }))
    expect(h1).toHaveTextContent(here!)
  })

  async function withTask(title = 'Zebra review') {
    const ctx = await setup()
    const { store } = ctx.services
    let id = ''
    await act(async () => {
      id = (await store.getState().createTask({ listId: store.getState().inboxId, title, dueMs: startOfDay(now()), isAllDay: true })).id
    })
    return { ...ctx, id, inboxId: store.getState().inboxId }
  }

  it('opens a task popover with an Open button that goes to the task', async () => {
    const { user, id, inboxId } = await withTask()
    await user.click(screen.getByRole('button', { name: 'Zebra review' }))
    const pop = screen.getByRole('dialog', { name: 'Zebra review' })
    expect(within(pop).getByText('Inbox')).toBeInTheDocument()
    await user.click(within(pop).getByRole('button', { name: 'Open' }))
    expect(screen.getByTestId('where')).toHaveTextContent(`/p/${inboxId}/tasks/${id}`)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('completes a task from its popover', async () => {
    const { services, user, id } = await withTask()
    await user.click(screen.getByRole('button', { name: 'Zebra review' }))
    await user.click(screen.getByRole('checkbox', { name: 'Complete Zebra review' }))
    expect(services.store.getState().tasks[id]!.status).toBe('done')
    expect(screen.queryByRole('button', { name: 'Zebra review' })).toBeNull() // done tasks are hidden by default
  })

  it('shows completed tasks greyed when asked', async () => {
    const { services, user } = await setup()
    await act(async () => {
      const t = await services.store.getState().createTask({ listId: services.store.getState().inboxId, title: 'Old news', dueMs: startOfDay(now()), isAllDay: true })
      await services.store.getState().toggleDone(t.id)
    })
    expect(screen.queryByRole('button', { name: 'Old news' })).toBeNull()
    await user.click(screen.getByRole('button', { name: 'View: Month' }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Show completed' }))
    expect(screen.getByRole('button', { name: 'Old news' })).toHaveClass('line-through')
  })

  it('adds a task for the day through the add popover', async () => {
    const { services, user } = await setup()
    await user.click(screen.getByRole('button', { name: 'Add task' }))
    const pop = screen.getByRole('dialog', { name: 'Add task' })
    const input = within(pop).getByRole('textbox', { name: 'Task title' })
    expect(input).toHaveFocus()
    await user.type(input, 'Water plants{Enter}')
    expect(screen.queryByRole('dialog')).toBeNull()
    const created = Object.values(services.store.getState().tasks).find((t) => t.title === 'Water plants')!
    expect(created.listId).toBe(services.store.getState().inboxId)
    expect(created.isAllDay).toBe(true)
    expect(startOfDay(created.dueMs!)).toBe(created.dueMs)
    expect(await screen.findByRole('button', { name: 'Water plants' })).toBeInTheDocument()
  })

  it('adds to the clicked day, and Escape cancels', async () => {
    const { services, user } = await setup()
    const cell = screen.getAllByRole('gridcell')[10]!
    await user.click(cell)
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('dialog')).toBeNull()
    await user.click(cell)
    await user.type(screen.getByRole('textbox', { name: 'Task title' }), 'Pay rent{Enter}')
    const created = Object.values(services.store.getState().tasks).find((t) => t.title === 'Pay rent')!
    expect(cell.getAttribute('aria-label')).toContain(new Date(created.dueMs!).getFullYear().toString())
    expect(cell.dataset.day).toBe(String(created.dueMs))
  })

  it('adds a timed task from a week-view day column via the keyboard', async () => {
    const { services, user } = await setup('/c/all/calendar/w')
    const col = screen.getAllByRole('gridcell').find((c) => c.tabIndex === 0 && !/^All-day/.test(c.getAttribute('aria-label') ?? ''))!
    col.focus()
    await user.keyboard('{Enter}')
    await user.type(screen.getByRole('textbox', { name: 'Task title' }), 'Standup{Enter}')
    const created = Object.values(services.store.getState().tasks).find((t) => t.title === 'Standup')!
    expect(created.isAllDay).toBe(false)
    expect(created.dueMs).not.toBeNull()
  })

  it('lists the rest of a busy day under "+N more"', async () => {
    const { services, user } = await setup()
    const inboxId = services.store.getState().inboxId
    await act(async () => {
      for (const n of [1, 2, 3, 4, 5]) await services.store.getState().createTask({ listId: inboxId, title: `Busy ${n}`, dueMs: startOfDay(now()), isAllDay: true })
    })
    await user.click(screen.getByRole('button', { name: '+2 more' }))
    const pop = screen.getByRole('dialog')
    expect(within(pop).getAllByRole('button')).toHaveLength(5)
  })
})
