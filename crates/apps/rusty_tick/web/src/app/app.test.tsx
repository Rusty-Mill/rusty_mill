import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { INBOX_ID } from '@/api/memory'
import { addDays, atTime, startOfDay } from '@/lib/date'
import { renderApp } from '@/test/renderApp'

const opts = { advanceTimers: undefined }
void opts
const tasksList = () => screen.getByRole('list', { name: 'Tasks' })
const pane = () => within(screen.getByRole('complementary', { name: 'Task details' }))
const rowFor = (title: string) => within(tasksList()).getByText(title).closest('li') as HTMLElement
const titles = () => within(tasksList()).queryAllByRole('listitem').map((li) => li.textContent)

describe('the shell', () => {
  it('shows the smart lists with counts, lists, tags and the pinned footer', async () => {
    await renderApp('/q/all/tasks', async (api) => {
      const work = await api.createList({ name: 'Work' })
      await api.createTask({ listId: work.id, title: 'A', tags: ['q4'] })
      await api.createTask({ listId: INBOX_ID, title: 'B' })
    })
    const side = screen.getByRole('complementary', { name: 'Lists' })
    for (const name of ['All', 'Today', 'Next 7 Days', 'Inbox', 'Summary', 'Completed', 'Trash']) expect(within(side).getByText(name)).toBeInTheDocument()
    expect(within(side).getByRole('link', { name: /^All\s*2$/ })).toBeInTheDocument()
    expect(within(side).getByRole('link', { name: /^Work\s*1$/ })).toBeInTheDocument()
    expect(within(side).getByRole('link', { name: /^q4\s*1$/i })).toBeInTheDocument()
    expect(within(side).getByText(/Used: 1\/9/)).toBeInTheDocument()
    expect(within(side).getByRole('button', { name: 'Upgrade to Premium' })).toBeDisabled()
  })

  it('shows the empty-state hints when there are no tags', async () => {
    await renderApp()
    expect(screen.getByText(/Display tasks filtered by list, date, priority, tag, and more/)).toBeInTheDocument()
    expect(screen.getByText(/Categorize your tasks with tags/)).toBeInTheDocument()
  })

  it('redirects an unknown hash to All', async () => {
    const { router } = await renderApp('/nonsense/route')
    await waitFor(() => expect(router.state.location.pathname).toBe('/q/all/tasks'))
  })

  it('names the empty list as the copy says', async () => {
    await renderApp()
    expect(screen.getByText('No tasks')).toBeInTheDocument()
    expect(screen.getByText('Click the input box to add')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Add task to "Inbox"')).toBeInTheDocument()
  })
})

describe('quick add', () => {
  it('adds a task on Enter, reading date, priority and tag from the text', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks')
    await user.type(screen.getByLabelText('Add task'), 'call mum tomorrow !high #family{Enter}')
    await waitFor(() => expect(within(tasksList()).getByText('call mum')).toBeInTheDocument())
    expect(screen.getByLabelText('Add task')).toHaveValue('')
    const [t] = (await api.snapshot()).tasks
    expect(t).toMatchObject({ title: 'call mum', priority: 5, tags: ['family'], isAllDay: true, listId: INBOX_ID })
    expect(t!.dueMs).toBe(startOfDay(addDays(Date.now(), 1)))
  })

  it('highlights recognised tokens while typing', async () => {
    const user = userEvent.setup()
    await renderApp()
    await user.type(screen.getByLabelText('Add task'), 'pay rent tomorrow !high')
    const marks = [...document.querySelectorAll('mark')].map((m) => m.textContent)
    expect(marks).toEqual(['tomorrow', '!high'])
  })

  it('does not add a line that is only tokens', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp()
    await user.type(screen.getByLabelText('Add task'), 'tomorrow #a{Enter}')
    expect((await api.snapshot()).tasks).toHaveLength(0)
  })

  it('adds to the list being viewed, and ~list overrides it', async () => {
    const user = userEvent.setup()
    let workId = ''
    const { api } = await renderApp('/q/all/tasks', async (a) => {
      workId = (await a.createList({ name: 'Work' })).id
      await a.createList({ name: 'Home' })
    })
    await user.type(screen.getByLabelText('Add task'), 'to inbox{Enter}')
    await user.type(screen.getByLabelText('Add task'), 'to home ~home{Enter}')
    await waitFor(async () => expect((await api.snapshot()).tasks).toHaveLength(2))
    const byTitle = Object.fromEntries((await api.snapshot()).tasks.map((t) => [t.title, t.listId]))
    expect(byTitle['to inbox']).toBe(INBOX_ID)
    expect(byTitle['to home']).not.toBe(INBOX_ID)
    expect(byTitle['to home']).not.toBe(workId)
  })

  it('a task added while viewing Today is due today', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/q/today/tasks')
    await user.type(screen.getByLabelText('Add task'), 'water plants{Enter}')
    await waitFor(async () => expect((await api.snapshot()).tasks[0]?.dueMs).toBe(startOfDay(Date.now())))
  })
})

describe('completing and opening tasks', () => {
  const seed = async (api: import('@/api/memory').MemoryAdapter) => {
    await api.createTask({ listId: INBOX_ID, title: 'first', sortOrder: 0 })
    await api.createTask({ listId: INBOX_ID, title: 'second', sortOrder: 1 })
  }

  it('ticking a task takes it off the list and puts it in Completed', async () => {
    const user = userEvent.setup()
    const { router } = await renderApp('/p/inbox/tasks', seed)
    await user.click(within(rowFor('first')).getByRole('checkbox', { name: 'first' }))
    await waitFor(() => expect(screen.queryByText('first')).toBeNull())
    expect(screen.getByText('second')).toBeInTheDocument()
    await router.navigate('/q/all/completed')
    expect(await screen.findByText('first')).toBeInTheDocument()
    expect(screen.getAllByText('Today').length).toBeGreaterThan(1) // sidebar + group header; grouped by completion day
  })

  it('unticking in Completed reopens it', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/q/all/completed', async (a) => {
      const t = await a.createTask({ listId: INBOX_ID, title: 'done one' })
      await a.updateTask(t.id, { status: 'done' })
    })
    await user.click(await screen.findByRole('checkbox', { name: 'Reopen: done one' }))
    await waitFor(async () => expect((await api.snapshot()).tasks[0]?.status).toBe('open'))
  })

  it('clicking a task opens it in the detail pane, and the URL says which', async () => {
    const user = userEvent.setup()
    const { router } = await renderApp('/p/inbox/tasks', seed)
    expect(screen.getByRole('complementary', { name: 'Task details' })).toBeInTheDocument()
    await user.click(within(tasksList()).getByText('second'))
    expect(router.state.location.pathname).toMatch(/^\/p\/inbox\/tasks\/[0-9a-f-]{36}$/)
    expect(await screen.findByLabelText('Title')).toHaveValue('second')
    expect(rowFor('second')).toHaveAttribute('aria-current', 'true')
  })

  it('editing the title in the pane updates the list', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', seed)
    await user.click(screen.getByText('first'))
    const title = await screen.findByLabelText('Title')
    await user.clear(title)
    await user.type(title, 'renamed')
    await user.tab() // blur saves
    await waitFor(() => expect(within(tasksList()).getByText('renamed')).toBeInTheDocument())
    expect((await api.snapshot()).tasks.map((t) => t.title).sort()).toEqual(['renamed', 'second'])
  })

  it('an emptied title is put back, not saved', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', seed)
    await user.click(screen.getByText('first'))
    const title = await screen.findByLabelText('Title')
    await user.clear(title)
    await user.tab()
    await waitFor(() => expect(title).toHaveValue('first'))
    expect((await api.snapshot()).tasks.map((t) => t.title).sort()).toEqual(['first', 'second'])
  })

  it('the priority flag sets priority', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', seed)
    await user.click(screen.getByText('first'))
    await user.click(await screen.findByRole('button', { name: /Priority: None/ }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'High priority' }))
    await waitFor(async () => expect((await api.snapshot()).tasks.find((t) => t.title === 'first')?.priority).toBe(5))
  })

  it('tags: add an existing one, create a new one, remove one', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', async (a) => {
      await seed(a)
      await a.createTag('Home')
    })
    await user.click(screen.getByText('first'))
    await user.click(await screen.findByLabelText('Task details').then(() => pane().getByRole('button', { name: 'Add tag' })))
    await user.click(screen.getByRole('option', { name: /Home/ }))
    await user.type(screen.getByLabelText('Search or create a tag'), 'Errands{Enter}')
    await user.keyboard('{Escape}')
    await waitFor(async () => expect((await api.snapshot()).tasks.find((t) => t.title === 'first')?.tags).toEqual(['home', 'errands']))
    await user.click(screen.getByRole('button', { name: 'Remove tag Home' }))
    await waitFor(async () => expect((await api.snapshot()).tasks.find((t) => t.title === 'first')?.tags).toEqual(['errands']))
  })

  it('the checklist toggle turns lines into items and back', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'list me', notes: '- [x] one\n- [ ] two' })
    })
    await user.click(screen.getByText('list me'))
    await user.click(await screen.findByRole('button', { name: 'Switch to checklist' }))
    await waitFor(async () => {
      const t = (await api.snapshot()).tasks[0]!
      expect(t.kind).toBe('checklist')
      expect(t.items.map((i) => [i.title, i.done])).toEqual([['one', true], ['two', false]])
      expect(t.notes).toBe('')
    })
    expect(await screen.findByDisplayValue('one')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Switch to text' }))
    await waitFor(async () => expect((await api.snapshot()).tasks[0]?.notes).toBe('- [x] one\n- [ ] two'))
  })
})

describe('the due-date popover', () => {
  it('a quick date and OK sets the date; the chip reads it back', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'dated' })
    })
    await user.click(screen.getByText('dated'))
    await user.click(await screen.findByRole('button', { name: /Due Date/ }))
    const dialog = screen.getByRole('dialog', { name: 'Due date' })
    await user.click(within(dialog).getByRole('button', { name: 'Tomorrow' }))
    await user.click(within(dialog).getByRole('button', { name: 'OK' }))
    const t = () => api.snapshot().then((s) => s.tasks[0]!)
    await waitFor(async () => expect((await t()).dueMs).toBe(startOfDay(addDays(Date.now(), 1))))
    expect((await t()).isAllDay).toBe(true)
    expect(await screen.findByRole('button', { name: /^Tomorrow, / })).toBeInTheDocument()
  })

  it('a time, a reminder and a repeat are saved together', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'meeting' })
    })
    await user.click(screen.getByText('meeting'))
    await user.click(await screen.findByRole('button', { name: /Due Date/ }))
    const dialog = screen.getByRole('dialog', { name: 'Due date' })
    await user.click(within(dialog).getByRole('button', { name: 'Today' }))
    const time = within(dialog).getByLabelText('Time')
    await user.clear(time)
    await user.type(time, '1730')
    await user.selectOptions(within(dialog).getByLabelText('Reminder'), 'TRIGGER:-PT30M')
    await user.selectOptions(within(dialog).getByLabelText('Repeat'), 'daily')
    await user.click(within(dialog).getByRole('button', { name: 'OK' }))
    await waitFor(async () => {
      const t = (await api.snapshot()).tasks[0]!
      expect(t.dueMs).toBe(atTime(startOfDay(Date.now()), 17, 30))
      expect(t).toMatchObject({ isAllDay: false, reminders: ['TRIGGER:-PT30M'], repeatFlag: 'RRULE:FREQ=DAILY' })
    })
  })

  it('Clear removes the date and everything hanging off it', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'clear me', dueMs: atTime(Date.now(), 17), reminders: ['TRIGGER:PT0S'], repeatFlag: 'RRULE:FREQ=DAILY' })
    })
    await user.click(screen.getByText('clear me'))
    await screen.findByLabelText('Title')
    await user.click(pane().getByRole('button', { name: /Today|Tomorrow|Yesterday|,/ }))
    await user.click(within(screen.getByRole('dialog', { name: 'Due date' })).getByRole('button', { name: 'Clear' }))
    await waitFor(async () => expect((await api.snapshot()).tasks[0]).toMatchObject({ dueMs: null, reminders: [], repeatFlag: '' }))
  })

  it('Escape closes it without saving', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'x' })
    })
    await user.click(screen.getByText('x'))
    await user.click(await screen.findByRole('button', { name: /Due Date/ }))
    await user.click(within(screen.getByRole('dialog', { name: 'Due date' })).getByRole('button', { name: 'Today' }))
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('dialog', { name: 'Due date' })).toBeNull()
    expect((await api.snapshot()).tasks[0]?.dueMs).toBeNull()
  })
})

describe('trash', () => {
  it('Move to Trash removes it from the list; Trash shows it; Restore brings it back', async () => {
    const user = userEvent.setup()
    const { api, router } = await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'bin me' })
    })
    await user.click(screen.getByText('bin me'))
    await screen.findByLabelText('Title')
    await user.click(pane().getByRole('button', { name: 'More' }))
    await user.click(screen.getByRole('menuitem', { name: 'Delete' }))
    await waitFor(() => expect(screen.queryByText('bin me')).toBeNull())
    expect((await api.snapshot()).tasks[0]?.deletedMs).not.toBeNull()

    await router.navigate('/q/all/trash')
    expect(await screen.findByText('bin me')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Restore bin me' }))
    await waitFor(async () => expect((await api.snapshot()).tasks[0]?.deletedMs).toBeNull())
    expect(await screen.findByText('Trash is empty')).toBeInTheDocument()
  })

  it('Delete forever and Empty Trash ask first, then remove for good', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/q/all/trash', async (a) => {
      for (const title of ['one', 'two', 'three']) {
        const t = await a.createTask({ listId: INBOX_ID, title })
        await a.trashTask(t.id)
      }
    })
    await user.click(await screen.findByRole('button', { name: 'Delete one forever' }))
    expect(screen.getByRole('dialog', { name: 'Delete forever?' })).toBeInTheDocument()
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Delete' }))
    await waitFor(async () => expect((await api.snapshot()).tasks).toHaveLength(2))

    await user.click(screen.getByRole('button', { name: 'Empty Trash' }))
    await user.click(within(screen.getByRole('dialog', { name: 'Empty Trash?' })).getByRole('button', { name: 'Empty Trash' }))
    await waitFor(async () => expect((await api.snapshot()).tasks).toHaveLength(0))
  })

  it('cancelling the confirmation keeps everything', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/q/all/trash', async (a) => {
      const t = await a.createTask({ listId: INBOX_ID, title: 'keep' })
      await a.trashTask(t.id)
    })
    await user.click(await screen.findByRole('button', { name: 'Empty Trash' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }))
    expect((await api.snapshot()).tasks).toHaveLength(1)
  })

  it('a trashed task opens read-only with a Restore banner', async () => {
    const user = userEvent.setup()
    await renderApp('/q/all/trash', async (a) => {
      const t = await a.createTask({ listId: INBOX_ID, title: 'gone' })
      await a.trashTask(t.id)
    })
    await user.click(await screen.findByText('gone'))
    expect(await screen.findByText('This task is in the Trash.')).toBeInTheDocument()
    expect(screen.getByLabelText('Title')).toBeDisabled()
  })
})

describe('lists and tags', () => {
  it('adds a list from the sidebar dialog and opens it', async () => {
    const user = userEvent.setup()
    const { api, router } = await renderApp()
    await user.click(screen.getByRole('button', { name: 'Add list' }))
    await user.type(within(screen.getByRole('dialog', { name: 'Add List' })).getByPlaceholderText('List name'), 'Errands')
    await user.click(within(screen.getByRole('dialog')).getByRole('radio', { name: '#4772fa' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Add' }))
    await waitFor(async () => expect((await api.snapshot()).lists.map((l) => l.name)).toContain('Errands'))
    expect((await api.snapshot()).lists.find((l) => l.name === 'Errands')?.color).toBe('#4772fa')
    await waitFor(() => expect(router.state.location.pathname).toMatch(/^\/p\/[0-9a-f-]{36}\/tasks$/))
    expect(screen.getByPlaceholderText('Add task to "Errands"')).toBeInTheDocument()
  })

  it('renames, archives and deletes a list', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/q/all/tasks', async (a) => {
      await a.createList({ name: 'Old name' })
    })
    await user.click(await screen.findByRole('button', { name: 'Old name options' }))
    await user.click(screen.getByRole('menuitem', { name: 'Edit' }))
    const name = within(screen.getByRole('dialog', { name: 'Edit List' })).getByPlaceholderText('List name')
    await user.clear(name)
    await user.type(name, 'New name')
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Save' }))
    await waitFor(async () => expect((await api.snapshot()).lists.map((l) => l.name)).toContain('New name'))

    await user.click(await screen.findByRole('button', { name: 'New name options' }))
    await user.click(screen.getByRole('menuitem', { name: 'Archive' }))
    await waitFor(async () => expect((await api.snapshot()).lists.find((l) => l.name === 'New name')?.archived).toBe(true))
    expect(await screen.findByText('Archived Lists')).toBeInTheDocument()

    await user.click(await screen.findByRole('button', { name: 'New name options' }))
    await user.click(screen.getByRole('menuitem', { name: 'Delete' }))
    await user.click(within(screen.getByRole('dialog', { name: 'Delete list?' })).getByRole('button', { name: 'Delete' }))
    await waitFor(async () => expect((await api.snapshot()).lists.map((l) => l.name)).toEqual(['Inbox']))
  })

  it('deleting a list sends its tasks to the Trash', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/q/all/tasks', async (a) => {
      const l = await a.createList({ name: 'Doomed' })
      await a.createTask({ listId: l.id, title: 'inside' })
    })
    await user.click(await screen.findByRole('button', { name: 'Doomed options' }))
    await user.click(screen.getByRole('menuitem', { name: 'Delete' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Delete' }))
    await waitFor(async () => expect((await api.snapshot()).tasks[0]?.deletedMs).not.toBeNull())
  })

  it('renames and recolours a tag from the sidebar, and deleting one takes it off its tasks', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/q/all/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 't', tags: ['old'] })
    })
    await user.click(await screen.findByRole('button', { name: 'old options' }))
    await user.click(screen.getByRole('menuitem', { name: 'Edit' }))
    const label = within(screen.getByRole('dialog', { name: 'Edit Tag' })).getByPlaceholderText('Tag name')
    await user.clear(label)
    await user.type(label, 'New')
    await user.click(within(screen.getByRole('dialog')).getByRole('radio', { name: '#e5312d' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Save' }))
    await waitFor(async () => {
      const s = await api.snapshot()
      expect(s.tags).toMatchObject([{ name: 'new', label: 'New', color: '#e5312d' }])
      expect(s.tasks[0]?.tags).toEqual(['new'])
    })

    await user.click(await screen.findByRole('button', { name: 'New options' }))
    await user.click(screen.getByRole('menuitem', { name: 'Delete' }))
    await user.click(within(screen.getByRole('dialog', { name: 'Delete tag?' })).getByRole('button', { name: 'Delete' }))
    await waitFor(async () => expect((await api.snapshot()).tags).toEqual([]))
    expect((await api.snapshot()).tasks[0]?.tags).toEqual([])
  })
})

describe('views', () => {
  it('Today shows what is due today or earlier, grouped, with Overdue first', async () => {
    await renderApp('/q/today/tasks', async (a) => {
      const now = Date.now()
      await a.createTask({ listId: INBOX_ID, title: 'late', dueMs: startOfDay(addDays(now, -2)), isAllDay: true })
      await a.createTask({ listId: INBOX_ID, title: 'now', dueMs: startOfDay(now), isAllDay: true })
      await a.createTask({ listId: INBOX_ID, title: 'later', dueMs: startOfDay(addDays(now, 3)), isAllDay: true })
    })
    expect(within(tasksList()).getByText('late')).toBeInTheDocument()
    expect(within(tasksList()).getByText('now')).toBeInTheDocument()
    expect(within(tasksList()).queryByText('later')).toBeNull()
    const headers = within(tasksList()).getAllByRole('button', { expanded: true }).map((b) => b.textContent)
    expect(headers).toEqual(['Overdue1', 'Today1'])
  })

  it('a group can be collapsed and expanded', async () => {
    const user = userEvent.setup()
    await renderApp('/q/all/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'undated' })
    })
    const header = screen.getByRole('button', { name: /No Date/ })
    await user.click(header)
    expect(screen.queryByText('undated')).toBeNull()
    await user.click(screen.getByRole('button', { name: /No Date/ }))
    expect(screen.getByText('undated')).toBeInTheDocument()
  })

  it('the sort menu regroups and reorders', async () => {
    const user = userEvent.setup()
    await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'banana', priority: 1, sortOrder: 0 })
      await a.createTask({ listId: INBOX_ID, title: 'apple', priority: 5, sortOrder: 1 })
      await a.createTask({ listId: INBOX_ID, title: 'cherry', priority: 0, sortOrder: 2 })
    })
    expect(titles().map((t) => t?.slice(0, 6))).toEqual(['banana', 'apple', 'cherry']) // custom order
    await user.click(screen.getByRole('button', { name: 'Sort' }))
    await user.click(screen.getByRole('menuitem', { name: /Sort by/ }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Title' }))
    await waitFor(() => expect(titles().map((t) => t?.slice(0, 6))).toEqual(['apple', 'banana', 'cherry']))

    await user.click(screen.getByRole('button', { name: 'Sort' }))
    await user.click(screen.getByRole('menuitem', { name: /Group by/ }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Priority' }))
    await waitFor(() => expect(screen.getByRole('button', { name: /High Priority/ })).toBeInTheDocument())
  })

  it('the More menu hides details and toggles completed tasks', async () => {
    const user = userEvent.setup()
    await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'open one', tags: ['t'] })
      const d = await a.createTask({ listId: INBOX_ID, title: 'finished' })
      await a.updateTask(d.id, { status: 'done' })
    })
    expect(screen.queryByText('finished')).toBeNull()
    await user.click(screen.getByRole('button', { name: 'More' }))
    await user.click(screen.getByRole('menuitem', { name: 'Show Completed' }))
    expect(await screen.findByText('finished')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'More' }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Show Details' }))
    await waitFor(() => expect(within(rowFor('open one')).queryByText('t')).toBeNull())
  })

  it('switches to the Kanban view and back', async () => {
    const user = userEvent.setup()
    await renderApp('/p/inbox/tasks', async (a) => {
      await a.createTask({ listId: INBOX_ID, title: 'card', priority: 5 })
    })
    await user.click(screen.getByRole('button', { name: 'More' }))
    await user.click(screen.getByRole('radio', { name: 'Kanban' }))
    expect(await screen.findByRole('list', { name: 'Board' })).toBeInTheDocument()
    expect(within(screen.getByRole('list', { name: 'Board' })).getByText('High Priority')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'More' }))
    await user.click(screen.getByRole('radio', { name: 'List' }))
    expect(await screen.findByRole('list', { name: 'Tasks' })).toBeInTheDocument()
  })
})

describe('search', () => {
  const seed = async (api: import('@/api/memory').MemoryAdapter) => {
    const w = await api.createList({ name: 'Work' })
    await api.createTask({ listId: INBOX_ID, title: 'Buy groceries' })
    await api.createTask({ listId: w.id, title: 'Quarterly report', notes: 'includes the groceries budget' })
    await api.createTask({ listId: w.id, title: 'Unrelated' })
  }

  it('Ctrl+K opens it; a prefix finds tasks, matches are highlighted, Enter opens the best one', async () => {
    const user = userEvent.setup()
    const { router } = await renderApp('/q/all/tasks', seed)
    await user.keyboard('{Control>}k{/Control}')
    const box = await screen.findByRole('combobox', { name: 'Search' })
    expect(box).toHaveFocus()
    await user.type(box, 'grocer')
    const results = within(screen.getByRole('listbox', { name: 'Results' }))
    expect(results.getAllByRole('option')).toHaveLength(2)
    expect(document.querySelector('mark')?.textContent).toBe('grocer')
    expect(results.getAllByRole('option')[0]).toHaveTextContent('Buy groceries') // a title match beats a notes match
    await user.keyboard('{Enter}')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(router.state.location.pathname).toMatch(/^\/p\/inbox\/tasks\/[0-9a-f-]{36}$/)
    expect(await screen.findByLabelText('Title')).toHaveValue('Buy groceries')
  })

  it('arrow keys move the selection', async () => {
    const user = userEvent.setup()
    await renderApp('/q/all/tasks', seed)
    await user.keyboard('{Control>}k{/Control}')
    await user.type(await screen.findByRole('combobox', { name: 'Search' }), 'grocer')
    const options = screen.getAllByRole('option')
    expect(options[0]).toHaveAttribute('aria-selected', 'true')
    await user.keyboard('{ArrowDown}')
    expect(options[1]).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByRole('combobox')).toHaveAttribute('aria-activedescendant', options[1]!.id)
  })

  it('the List chip searches lists', async () => {
    const user = userEvent.setup()
    const { router } = await renderApp('/q/all/tasks', seed)
    await user.click(screen.getByRole('button', { name: 'Search' }))
    await user.click(await screen.findByRole('radio', { name: 'List' }))
    await user.type(screen.getByRole('combobox', { name: 'Search' }), 'wor')
    await user.keyboard('{Enter}')
    expect(router.state.location.pathname).toMatch(/^\/p\/[0-9a-f-]{36}\/tasks$/)
  })

  it('says so when nothing matches, and the clear button empties the box', async () => {
    const user = userEvent.setup()
    await renderApp('/q/all/tasks', seed)
    await user.keyboard('{Control>}k{/Control}')
    const box = await screen.findByRole('combobox', { name: 'Search' })
    await user.type(box, 'zzzz')
    expect(screen.getByText('No results')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Clear search' }))
    expect(box).toHaveValue('')
  })

  it('Escape closes it', async () => {
    const user = userEvent.setup()
    await renderApp('/q/all/tasks', seed)
    await user.keyboard('{Control>}k{/Control}')
    await screen.findByRole('dialog')
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('keyboard shortcuts', () => {
  const seed = async (api: import('@/api/memory').MemoryAdapter) => {
    for (const [i, title] of ['one', 'two', 'three'].entries()) await api.createTask({ listId: INBOX_ID, title, sortOrder: i })
  }

  it('N focuses the quick-add box', async () => {
    const user = userEvent.setup()
    await renderApp('/p/inbox/tasks', seed)
    await user.keyboard('n')
    expect(screen.getByLabelText('Add task')).toHaveFocus()
  })

  it('J and K move the selection; Space completes; 1 sets high priority; Delete trashes', async () => {
    const user = userEvent.setup()
    const { api, router } = await renderApp('/p/inbox/tasks', seed)
    ;(document.activeElement as HTMLElement).blur() // the quick-add box may hold focus, and typing there is not a shortcut
    await user.keyboard('j')
    await waitFor(() => expect(router.state.location.pathname).toMatch(/tasks\/[0-9a-f-]{36}$/))
    expect(await screen.findByLabelText('Title')).toHaveValue('one')
    await user.keyboard('j')
    await waitFor(() => expect(screen.getByLabelText('Title')).toHaveValue('two'))
    await user.keyboard('k')
    await waitFor(() => expect(screen.getByLabelText('Title')).toHaveValue('one'))

    await user.keyboard('1')
    await waitFor(async () => expect((await api.snapshot()).tasks.find((t) => t.title === 'one')?.priority).toBe(5))
    await user.keyboard(' ')
    await waitFor(async () => expect((await api.snapshot()).tasks.find((t) => t.title === 'one')?.status).toBe('done'))
  })

  it('Delete moves the selected task to the Trash and lands on a neighbour', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', seed)
    await user.click(screen.getByText('two'))
    await screen.findByLabelText('Title')
    await user.keyboard('{Delete}')
    await waitFor(async () => expect((await api.snapshot()).tasks.find((t) => t.title === 'two')?.deletedMs).not.toBeNull())
    await waitFor(() => expect(screen.getByLabelText('Title')).toHaveValue('three'))
  })

  it('does not act on keys typed into a field', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks', seed)
    await user.click(screen.getByText('one'))
    const title = await screen.findByLabelText('Title')
    await user.click(title)
    await user.keyboard('{End} 1 j n')
    expect(title).toHaveValue('one 1 j n')
    expect((await api.snapshot()).tasks.find((t) => t.title === 'one')?.priority).toBe(0)
  })

  it('Escape closes the detail pane', async () => {
    const user = userEvent.setup()
    const { router } = await renderApp('/p/inbox/tasks', seed)
    await user.click(screen.getByText('one'))
    await screen.findByLabelText('Title')
    ;(document.activeElement as HTMLElement).blur()
    await user.keyboard('{Escape}')
    await waitFor(() => expect(router.state.location.pathname).toBe('/p/inbox/tasks'))
  })
})

describe('large lists', () => {
  it('draws only a window of rows once there are hundreds', async () => {
    await renderApp('/p/inbox/tasks', async (api) => {
      for (let i = 0; i < 260; i++) await api.createTask({ listId: INBOX_ID, title: `task ${i}`, sortOrder: i })
    })
    const drawn = within(tasksList()).getAllByRole('listitem').length
    expect(drawn).toBeGreaterThan(0)
    expect(drawn).toBeLessThan(80) // 260 rows exist; only those near the top are in the DOM
  })

  it('draws everything for a normal-sized list', async () => {
    await renderApp('/p/inbox/tasks', async (api) => {
      for (let i = 0; i < 30; i++) await api.createTask({ listId: INBOX_ID, title: `task ${i}`, sortOrder: i })
    })
    expect(within(tasksList()).getAllByRole('checkbox')).toHaveLength(30)
  })
})
