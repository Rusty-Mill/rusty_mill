import { useSearchParams } from 'react-router-dom'
import { Dialog } from '@/components/Dialog'

/** `?modalType=settings&tabs=…` over the current route. (Placeholder until the settings feature lands.) */
export function SettingsModal() {
  const [params, setParams] = useSearchParams()
  const open = params.get('modalType') === 'settings'
  const close = (): void =>
    setParams((p) => {
      p.delete('modalType')
      p.delete('tabs')
      return p
    }, { replace: true })
  return (
    <Dialog open={open} onClose={close} title="Settings" width={780} height={680}>
      <p className="px-6 text-grey">Settings are coming.</p>
    </Dialog>
  )
}
