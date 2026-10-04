import { Dialog } from './Dialog'

interface Props {
  open: boolean
  title: string
  message: string
  confirmLabel?: string
  danger?: boolean
  onConfirm: () => void
  onCancel: () => void
}

/** A yes/no question. Enter confirms; Escape or Cancel backs out. */
export function Confirm({ open, title, message, confirmLabel = 'OK', danger = false, onConfirm, onCancel }: Props) {
  return (
    <Dialog open={open} onClose={onCancel} title={title} width={400}>
      <p className="px-6 pb-4 text-base text-grey">{message}</p>
      <div className="flex justify-end gap-2 px-6 pb-5">
        <button type="button" onClick={onCancel} className="btn border-transparent">
          Cancel
        </button>
        <button type="button" data-autofocus onClick={onConfirm} className={`h-8 rounded-row px-4 text-white ${danger ? 'bg-danger' : 'bg-primary'}`}>
          {confirmLabel}
        </button>
      </div>
    </Dialog>
  )
}
