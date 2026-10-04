import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { Confirm } from './Confirm'
import { Dialog } from './Dialog'

describe('Dialog', () => {
  it('labels itself, focuses the first field, closes on Escape and gives focus back', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <>
        <button>opener</button>
        <Dialog open onClose={onClose} title="Hello">
          <input aria-label="field" data-autofocus />
        </Dialog>
      </>,
    )
    expect(screen.getByRole('dialog', { name: 'Hello' })).toBeInTheDocument()
    expect(screen.getByLabelText('field')).toHaveFocus()
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('renders nothing when closed', () => {
    render(
      <Dialog open={false} onClose={() => undefined} title="Hidden">
        x
      </Dialog>,
    )
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('Confirm', () => {
  it('confirms with Enter on the focused button and cancels with Cancel', async () => {
    const user = userEvent.setup()
    const onConfirm = vi.fn()
    const onCancel = vi.fn()
    render(<Confirm open title="Sure?" message="Really." confirmLabel="Yes" onConfirm={onConfirm} onCancel={onCancel} />)
    expect(screen.getByRole('button', { name: 'Yes' })).toHaveFocus()
    await user.keyboard('{Enter}')
    expect(onConfirm).toHaveBeenCalledOnce()
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(onCancel).toHaveBeenCalledOnce()
  })
})
