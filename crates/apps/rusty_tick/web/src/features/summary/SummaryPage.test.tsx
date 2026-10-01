import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createServices, ServicesProvider, type Services } from '@/app/services'
import { atTime } from '@/lib/date'
import { SummaryPage } from './SummaryPage'

let services: Services

async function setup() {
  services = createServices('demo')
  await services.store.getState().boot()
  const s = services.store.getState()
  await s.createTask({ listId: s.inboxId, title: 'Write the <report>', dueMs: atTime(Date.now(), 12) })
  const user = userEvent.setup()
  render(
    <ServicesProvider services={services}>
      <MemoryRouter>
        <SummaryPage />
      </MemoryRouter>
    </ServicesProvider>,
  )
  await waitFor(() => expect(editor()).toHaveTextContent('Weekly report')) // the page fills itself once its options have loaded
  return user
}

const editor = (): HTMLElement => screen.getByRole('textbox', { name: 'Summary' })

/** Choose "Today" and generate, so the result does not depend on which day of the week the suite runs. */
async function generateToday(user: ReturnType<typeof userEvent.setup>) {
  await user.selectOptions(screen.getByLabelText('Date'), 'today')
  await user.click(screen.getByRole('button', { name: /generate/i }))
}

const readBlob = (b: Blob): Promise<string> =>
  new Promise((resolve) => {
    const r = new FileReader()
    r.onload = () => resolve(String(r.result))
    r.readAsText(b)
  })

const clipboard = { writeText: vi.fn<(t: string) => Promise<void>>() }

beforeEach(() => {
  clipboard.writeText = vi.fn().mockResolvedValue(undefined)
})
afterEach(() => {
  vi.restoreAllMocks()
  Reflect.deleteProperty(document, 'execCommand')
})

describe('SummaryPage', () => {
  it('has an accessible editor, a formatting toolbar and the settings panel', async () => {
    await setup()
    const box = editor()
    expect(box).toHaveAttribute('aria-multiline', 'true')
    const bar = screen.getByRole('toolbar', { name: 'Formatting' })
    for (const name of ['Heading', 'Bold', 'Highlight', 'Checklist', 'Bulleted list', 'Numbered list', 'Italic', 'Underline', 'Strikethrough', 'Divider', 'Undo', 'Redo', 'Link', 'Code', 'Quote']) {
      expect(within(bar).getByRole('button', { name })).toBeInTheDocument()
    }
    expect(within(bar).getByRole('button', { name: 'Bold' })).toHaveAttribute('aria-pressed', 'false')
    expect(within(bar).getByRole('button', { name: 'Undo' })).not.toHaveAttribute('aria-pressed')
    expect(screen.getByRole('radiogroup', { name: 'Template' })).toBeInTheDocument()
    expect(screen.getByLabelText('Date')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'Show list name' })).toBeChecked()
  })

  it('generates headings and checklist items from the tasks, escaping their text', async () => {
    const user = await setup()
    await generateToday(user)
    expect(within(editor()).getByRole('heading', { level: 1 })).toHaveTextContent('Weekly report')
    const item = within(editor()).getByText(/Write the/)
    expect(item.closest('li')).toHaveAttribute('data-checked', 'false')
    expect(item.closest('ul')).toHaveAttribute('data-checklist')
    expect(item.textContent).toContain('<report>') // shown as text, not parsed as a tag
    expect(editor().querySelector('report')).toBeNull()
  })

  it('reflects the chosen template and status', async () => {
    const user = await setup()
    await user.click(screen.getByRole('radio', { name: 'Daily report' }))
    await user.selectOptions(screen.getByRole('combobox', { name: 'Status' }), 'completed')
    await generateToday(user)
    expect(within(editor()).getByRole('heading', { level: 1 })).toHaveTextContent('Daily report')
    expect(editor()).toHaveTextContent('No tasks match these filters.')
  })

  it('shows two date inputs for a custom range', async () => {
    const user = await setup()
    expect(screen.queryByLabelText('From date')).toBeNull()
    await user.selectOptions(screen.getByLabelText('Date'), 'custom')
    expect(screen.getByLabelText('From date')).toBeInTheDocument()
    expect(screen.getByLabelText('To date')).toBeInTheDocument()
  })

  it('copies the summary as text and confirms with a toast', async () => {
    const user = await setup()
    Object.defineProperty(navigator, 'clipboard', { value: clipboard, configurable: true })
    await generateToday(user)
    await user.click(screen.getByRole('button', { name: 'Copy' }))
    await waitFor(() => expect(clipboard.writeText).toHaveBeenCalledOnce())
    const text = clipboard.writeText.mock.calls[0]![0]
    expect(text).toContain('Weekly report')
    expect(text).toContain('- [ ] Write the <report>')
    expect(services.store.getState().toasts.map((t) => t.message)).toContain('Copied')
  })

  it('says so when the clipboard is refused', async () => {
    const user = await setup()
    clipboard.writeText.mockRejectedValue(new Error('denied'))
    Object.defineProperty(navigator, 'clipboard', { value: clipboard, configurable: true })
    await generateToday(user)
    await user.click(screen.getByRole('button', { name: 'Copy' }))
    await waitFor(() => expect(services.store.getState().toasts.some((t) => t.kind === 'error')).toBe(true))
  })

  it.each([
    ['Markdown (.md)', /^summary-\d{4}-\d{2}-\d{2}\.md$/, /^# Weekly report/],
    ['Plain text (.txt)', /\.txt$/, /^Weekly report\n/],
    ['HTML (.html)', /\.html$/, /^<!doctype html>/],
  ])('saves as %s through a temporary link', async (label, filename, body) => {
    const user = await setup()
    const blobs: Blob[] = []
    URL.createObjectURL = vi.fn((b: Blob | MediaSource) => (blobs.push(b as Blob), 'blob:x'))
    URL.revokeObjectURL = vi.fn()
    const names: string[] = []
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      names.push(this.download)
    })
    await generateToday(user)
    await user.click(screen.getByRole('button', { name: /save as/i }))
    await user.click(screen.getByRole('menuitem', { name: label }))
    expect(names).toHaveLength(1)
    expect(names[0]).toMatch(filename)
    const text = await readBlob(blobs[0]!)
    expect(text).toMatch(body)
    expect(text).toContain('Write the')
    expect(document.querySelector('a[download]')).toBeNull() // the anchor is removed again
  })

  it('does not save or copy an empty editor', async () => {
    const user = await setup()
    Object.defineProperty(navigator, 'clipboard', { value: clipboard, configurable: true })
    editor().innerHTML = ''
    fireEvent.input(editor())
    await user.click(screen.getByRole('button', { name: 'Copy' }))
    expect(clipboard.writeText).not.toHaveBeenCalled()
  })

  it('runs toolbar commands on the editor and pastes plain text only', async () => {
    const user = await setup()
    const exec = vi.fn(() => true)
    Object.defineProperty(document, 'execCommand', { value: exec, configurable: true })
    await user.click(screen.getByRole('button', { name: 'Bold' }))
    expect(exec).toHaveBeenCalledWith('bold', false, undefined)
    await user.click(screen.getByRole('button', { name: 'Numbered list' }))
    expect(exec).toHaveBeenCalledWith('insertOrderedList', false, undefined)
    await user.click(screen.getByRole('button', { name: 'Heading' }))
    expect(exec).toHaveBeenCalledWith('formatBlock', false, 'h2')
    fireEvent.paste(editor(), { clipboardData: { getData: (t: string) => (t === 'text/plain' ? 'just text' : '<b>rich</b>') } })
    expect(exec).toHaveBeenCalledWith('insertText', false, 'just text')
  })

  it('ticks a checklist box when its left edge is pressed', async () => {
    const user = await setup()
    await generateToday(user)
    const li = editor().querySelector('li')!
    li.getBoundingClientRect = () => ({ left: 0, top: 0, right: 200, bottom: 24, width: 200, height: 24, x: 0, y: 0, toJSON: () => ({}) })
    fireEvent.mouseDown(li, { clientX: 8, clientY: 10 })
    expect(li).toHaveAttribute('data-checked', 'true')
    fireEvent.mouseDown(li, { clientX: 120, clientY: 10 }) // on the text: no toggle
    expect(li).toHaveAttribute('data-checked', 'true')
  })

  it('asks for a link address and rejects an unsafe one', async () => {
    const user = await setup()
    await user.click(screen.getByRole('button', { name: 'Link' }))
    const input = await screen.findByLabelText('Link address')
    await user.type(input, 'javascript:alert(1){Enter}')
    expect(screen.getByRole('alert')).toHaveTextContent(/web or email address/)
  })

  it('remembers the options in a summary_template document', async () => {
    const user = await setup()
    await user.click(screen.getByRole('radio', { name: 'Simple list' }))
    await waitFor(async () => expect(await services.api.listDocs('summary_template')).toHaveLength(1), { timeout: 3000 })
    const [doc] = await services.api.listDocs('summary_template')
    expect(doc!.body).toMatchObject({ options: { template: 'simple' } })
  })

  it('restores saved options on the next visit', async () => {
    const user = await setup()
    await user.selectOptions(screen.getByLabelText('Date'), 'lastMonth')
    await waitFor(async () => expect(await services.api.listDocs('summary_template')).toHaveLength(1), { timeout: 3000 })
    render(
      <ServicesProvider services={services}>
        <MemoryRouter>
          <SummaryPage />
        </MemoryRouter>
      </ServicesProvider>,
    )
    await waitFor(() => expect(screen.getAllByLabelText('Date')[1]).toHaveValue('lastMonth'))
    await waitFor(() => expect(screen.getAllByRole('textbox', { name: 'Summary' })[1]).toHaveTextContent('Weekly report'))
  })
})
