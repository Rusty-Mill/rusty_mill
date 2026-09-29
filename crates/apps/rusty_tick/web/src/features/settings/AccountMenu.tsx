import { BarChart3, Crown, LogOut, Settings } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { Menu } from '@/components/Menu'
import { settingsHref } from '@/app/paths'

interface Props {
  anchor: HTMLElement | null
  open: boolean
  onClose: () => void
}

/** The avatar menu: Settings, Statistics, Premium, Sign Out. (Placeholder until the settings feature lands.) */
export function AccountMenu({ anchor, open, onClose }: Props) {
  const navigate = useNavigate()
  return (
    <Menu
      anchor={anchor}
      open={open}
      onClose={onClose}
      label="Account"
      placement="right-start"
      items={[
        { id: 'settings', label: 'Settings', icon: <Settings size={16} />, onSelect: () => navigate(settingsHref('account')) },
        { id: 'stats', label: 'Statistics', icon: <BarChart3 size={16} />, disabled: true },
        { id: 'premium', label: 'Premium', icon: <Crown size={16} />, disabled: true },
        'separator',
        { id: 'signout', label: 'Sign Out', icon: <LogOut size={16} />, disabled: true },
      ]}
    />
  )
}
