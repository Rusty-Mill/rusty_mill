import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { Dialog } from './Dialog'
import { Menu, type MenuEntry } from './Menu'
import { computePosition } from './Popover'
import { TaskCheck } from './TaskCheck'

function MenuHost({ items, onClose = () => undefined }: { items: MenuEntry[]; onClose?: () => void }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const [open, setOpen] = useState(false)
  return (
    <>
      <button ref={setAnchor} onClick={() => setOpen(true)}>open</button>
      <Menu anchor={anchor} open={open} onClose={() => { setOpen(false); onClose() }} items={items} label="Actions" />
    </>
  )
}

describe('Menu', () => {
  it('opens, moves with the arrow keys, and activates with Enter', async () => {
    const user = userEvent.setup()
    const a = vi.fn(), b = vi.fn()
    render(<MenuHost items={[{ id: 'a', label: 'Alpha', onSelect: a }, { id: 'b', label: 'Beta', onSelect: b }]} />)
    await user.click(screen.getByText('open'))
    expect(screen.getByRole('menu', { name: 'Actions' })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: 'Alpha' })).toHaveFocus()
    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('menuitem', { name: 'Beta' })).toHaveFocus()
    await user.keyboard('{ArrowDown}') // wraps
    expect(screen.getByRole('menuitem', { name: 'Alpha' })).toHaveFocus()
    await user.keyboard('{ArrowUp}{Enter}')
    expect(b).toHaveBeenCalledOnce()
    expect(a).not.toHaveBeenCalled()
    expect(screen.queryByRole('menu')).toBeNull() // choosing closes it
  })

  it('skips disabled items and separators', async () => {
    const user = userEvent.setup()
    render(<MenuHost items={[{ id: 'a', label: 'A' }, 'separator', { id: 'b', label: 'B', disabled: true }, { id: 'c', label: 'C' }]} />)
    await user.click(screen.getByText('open'))
    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('menuitem', { name: 'C' })).toHaveFocus()
  })

  it('closes on Escape and on a press outside, giving focus back', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(<><MenuHost items={[{ id: 'a', label: 'A' }]} onClose={onClose} /><p>elsewhere</p></>)
    await user.click(screen.getByText('open'))
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('menu')).toBeNull()
    expect(screen.getByText('open')).toHaveFocus()
    await user.click(screen.getByText('open'))
    await user.click(screen.getByText('elsewhere'))
    expect(screen.queryByRole('menu')).toBeNull()
    expect(onClose).toHaveBeenCalledTimes(2)
  })

  it('opens a submenu with the right arrow and closes everything when something is chosen in it', async () => {
    const user = userEvent.setup()
    const pick = vi.fn()
    render(<MenuHost items={[{ id: 'sort', label: 'Sort by', hint: 'Date', submenu: [{ id: 'title', label: 'Title', onSelect: pick }] }]} />)
    await user.click(screen.getByText('open'))
    await user.keyboard('{ArrowRight}')
    expect(screen.getByRole('menuitem', { name: 'Title' })).toBeInTheDocument()
    await user.click(screen.getByRole('menuitem', { name: 'Title' }))
    expect(pick).toHaveBeenCalledOnce()
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('a click inside a submenu does not count as outside the parent', async () => {
    const user = userEvent.setup()
    render(<MenuHost items={[{ id: 'sort', label: 'Sort by', submenu: [{ id: 'x', label: 'Inert', disabled: true }] }]} />)
    await user.click(screen.getByText('open'))
    await user.click(screen.getByRole('menuitem', { name: /Sort by/ }))
    await user.click(screen.getByRole('menuitem', { name: 'Inert' }))
    expect(screen.getByRole('menuitem', { name: /Sort by/ })).toBeInTheDocument()
  })

  it('Escape and the left arrow close only the innermost menu', async () => {
    const user = userEvent.setup()
    render(<MenuHost items={[{ id: 'sort', label: 'Sort by', submenu: [{ id: 'title', label: 'Title' }] }]} />)
    await user.click(screen.getByText('open'))
    await user.keyboard('{ArrowRight}')
    expect(screen.getByRole('menuitem', { name: 'Title' })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('menuitem', { name: 'Title' })).toBeNull()
    expect(screen.getByRole('menuitem', { name: /Sort by/ })).toBeInTheDocument() // the parent survives
    await user.keyboard('{ArrowRight}')
    expect(screen.getByRole('menuitem', { name: 'Title' })).toHaveFocus()
    await user.keyboard('{ArrowLeft}')
    expect(screen.queryByRole('menuitem', { name: 'Title' })).toBeNull()
    expect(screen.getByRole('menu', { name: 'Actions' })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('shows checked items as checkboxes', async () => {
    const user = userEvent.setup()
    render(<MenuHost items={[{ id: 'a', label: 'Hide Completed', checked: true }, { id: 'b', label: 'Show Details', checked: false }]} />)
    await user.click(screen.getByText('open'))
    expect(screen.getByRole('menuitemcheckbox', { name: 'Hide Completed' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('menuitemcheckbox', { name: 'Show Details' })).toHaveAttribute('aria-checked', 'false')
  })
})

describe('Dialog', () => {
  function DialogHost({ onClose = () => undefined }: { onClose?: () => void }) {
    const [open, setOpen] = useState(false)
    return (
      <>
        <button onClick={() => setOpen(true)}>launch</button>
        <Dialog open={open} onClose={() => { setOpen(false); onClose() }} title="Rename list">
          <input aria-label="Name" />
          <button>Save</button>
        </Dialog>
      </>
    )
  }

  it('is a labelled modal that takes focus and gives it back', async () => {
    const user = userEvent.setup()
    render(<DialogHost />)
    await user.click(screen.getByText('launch'))
    const dialog = screen.getByRole('dialog', { name: 'Rename list' })
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog.contains(document.activeElement)).toBe(true)
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(screen.getByText('launch')).toHaveFocus()
  })

  it('keeps Tab inside', async () => {
    const user = userEvent.setup()
    render(<DialogHost />)
    await user.click(screen.getByText('launch'))
    const dialog = screen.getByRole('dialog')
    for (let i = 0; i < 6; i++) {
      await user.tab()
      expect(dialog.contains(document.activeElement)).toBe(true)
    }
    for (let i = 0; i < 6; i++) {
      await user.tab({ shift: true })
      expect(dialog.contains(document.activeElement)).toBe(true)
    }
  })

  it('closes on a press on the backdrop but not inside', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(<DialogHost onClose={onClose} />)
    await user.click(screen.getByText('launch'))
    await user.click(screen.getByLabelText('Name'))
    expect(onClose).not.toHaveBeenCalled()
    await user.pointer({ target: screen.getByRole('dialog').parentElement!, keys: '[MouseLeft]' })
    expect(onClose).toHaveBeenCalledOnce()
  })
})

describe('TaskCheck', () => {
  it('is a labelled checkbox that toggles without triggering its row', async () => {
    const user = userEvent.setup()
    const change = vi.fn(), row = vi.fn()
    render(<div onClick={row}><TaskCheck checked={false} label="Buy milk" onChange={change} priority={5} /></div>)
    const box = screen.getByRole('checkbox', { name: 'Buy milk' })
    expect(box).toHaveAttribute('aria-checked', 'false')
    await user.click(box)
    expect(change).toHaveBeenCalledOnce()
    expect(row).not.toHaveBeenCalled()
    await user.keyboard('{Enter}')
    expect(change).toHaveBeenCalledTimes(2)
  })

  it('shows the priority colour and a tick once done', () => {
    const { rerender } = render(<TaskCheck checked={false} label="t" onChange={() => undefined} priority={5} />)
    expect(screen.getByRole('checkbox')).toHaveStyle({ borderColor: 'rgb(var(--prio-high))' })
    rerender(<TaskCheck checked label="t" onChange={() => undefined} priority={5} />)
    expect(screen.getByRole('checkbox')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('checkbox').querySelector('svg')).not.toBeNull()
  })
})

describe('computePosition', () => {
  const rect = (l: number, t: number, w = 100, h = 30) => ({ left: l, top: t, right: l + w, bottom: t + h, width: w, height: h }) as DOMRect
  const vp = { width: 1000, height: 800 }

  it('places below and aligned to the start', () => {
    expect(computePosition(rect(100, 100), { width: 200, height: 150 }, 'bottom-start', 6, vp)).toEqual({ left: 100, top: 136 })
  })
  it('aligns to the end', () => {
    expect(computePosition(rect(500, 100), { width: 200, height: 150 }, 'bottom-end', 6, vp)).toEqual({ left: 400, top: 136 })
  })
  it('flips above when there is no room below', () => {
    const p = computePosition(rect(100, 700), { width: 200, height: 150 }, 'bottom-start', 6, vp)
    expect(p.top).toBe(700 - 6 - 150)
  })
  it('stays inside the viewport', () => {
    const p = computePosition(rect(950, 100), { width: 200, height: 150 }, 'bottom-start', 6, vp)
    expect(p.left).toBe(1000 - 200 - 8)
    expect(computePosition(rect(-50, -50), { width: 10, height: 10 }, 'bottom-start', 0, vp).left).toBe(8)
  })
  it('opens submenus to the right, or the left at the edge', () => {
    expect(computePosition(rect(100, 100), { width: 180, height: 100 }, 'right-start', 4, vp)).toEqual({ left: 204, top: 100 })
    expect(computePosition(rect(880, 100), { width: 180, height: 100 }, 'right-start', 4, vp).left).toBe(880 - 4 - 180)
  })
})
