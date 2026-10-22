import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useSearchParams } from 'react-router-dom';
import { SETTINGS_TABS } from '@/app/paths';
import { Dialog } from '@/components/Dialog';
import { SettingsNav, TAB_META, panelId, tabId } from './SettingsNav';
import { AboutTab } from './tabs/AboutTab';
import { AccountTab } from './tabs/AccountTab';
import { AppearanceTab } from './tabs/AppearanceTab';
import { DateTimeTab } from './tabs/DateTimeTab';
import { NotificationsTab } from './tabs/NotificationsTab';
import { PlaceholderTab } from './tabs/PlaceholderTab';
import { ShortcutsTab } from './tabs/ShortcutsTab';
const isTab = (v) => SETTINGS_TABS.some((t) => t === v);
function Pane({ tab }) {
    switch (tab) {
        case 'account':
            return _jsx(AccountTab, {});
        case 'notifications':
            return _jsx(NotificationsTab, {});
        case 'date-time':
            return _jsx(DateTimeTab, {});
        case 'appearance':
            return _jsx(AppearanceTab, {});
        case 'shortcuts':
            return _jsx(ShortcutsTab, {});
        case 'about':
            return _jsx(AboutTab, {});
        default:
            return _jsx(PlaceholderTab, { title: TAB_META.find((t) => t.id === tab)?.label ?? '' });
    }
}
/** `?modalType=settings&tabs=…` over the current route: the URL says whether it is open and which tab shows. */
export function SettingsModal() {
    const [params, setParams] = useSearchParams();
    const open = params.get('modalType') === 'settings';
    const requested = params.get('tabs');
    const tab = isTab(requested) ? requested : 'account';
    const edit = (fn) => setParams((prev) => {
        const p = new URLSearchParams(prev);
        fn(p);
        return p;
    }, { replace: true });
    return (_jsx(Dialog, { open: open, onClose: () => edit((p) => (p.delete('modalType'), p.delete('tabs'))), labelledBy: "settings-title", width: 780, height: 680, children: _jsxs("div", { className: "flex min-h-0 flex-1", children: [_jsxs("div", { className: "flex w-[228px] shrink-0 flex-col border-r border-line bg-side", children: [_jsx("h2", { id: "settings-title", className: "px-6 pb-3 pt-5 text-title font-semibold", children: "Settings" }), _jsx(SettingsNav, { selected: tab, onSelect: (t) => edit((p) => p.set('tabs', t)) })] }), _jsx("div", { role: "tabpanel", id: panelId(tab), "aria-labelledby": tabId(tab), tabIndex: -1, className: "min-w-0 flex-1 overflow-y-auto px-8 pb-6 pt-6 outline-hidden", children: _jsx(Pane, { tab: tab }) })] }) }));
}
