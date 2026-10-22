import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { BarChart3, Crown, LogOut, Settings } from 'lucide-react';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { settingsHref } from '@/app/paths';
import { useServices } from '@/app/services';
import { Menu } from '@/components/Menu';
import { leaveDemo, signOut } from './session';
import { StatsDialog } from './StatsDialog';
/** The avatar menu: Settings, Statistics, Premium (not applicable here), and Sign Out / Leave demo. */
export function AccountMenu({ anchor, open, onClose }) {
    const navigate = useNavigate();
    const { mode } = useServices();
    const [stats, setStats] = useState(false);
    return (_jsxs(_Fragment, { children: [_jsx(Menu, { anchor: anchor, open: open, onClose: onClose, label: "Account", placement: "right-start", items: [
                    { id: 'settings', label: 'Settings', icon: _jsx(Settings, { size: 16 }), onSelect: () => navigate(settingsHref('account')) },
                    { id: 'stats', label: 'Statistics', icon: _jsx(BarChart3, { size: 16 }), onSelect: () => setStats(true) },
                    { id: 'premium', label: 'Premium', icon: _jsx(Crown, { size: 16 }), hint: 'Not applicable in Tick Local', disabled: true },
                    'separator',
                    mode === 'server'
                        ? { id: 'signout', label: 'Sign Out', icon: _jsx(LogOut, { size: 16 }), onSelect: signOut }
                        : { id: 'signout', label: 'Leave demo', icon: _jsx(LogOut, { size: 16 }), onSelect: leaveDemo },
                ] }), _jsx(StatsDialog, { open: stats, onClose: () => setStats(false) })] }));
}
