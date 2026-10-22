import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { usePrefs } from '../prefs';
import { PaneTitle, Row, Switch } from './controls';
export function NotificationsTab() {
    const supported = typeof Notification !== 'undefined';
    const on = usePrefs((s) => s.prefs.notifications);
    const update = usePrefs((s) => s.update);
    const [denied, setDenied] = useState(supported && Notification.permission === 'denied');
    const toggle = async (next) => {
        if (next && Notification.permission === 'default')
            await Notification.requestPermission();
        const blocked = next && Notification.permission !== 'granted';
        setDenied(blocked);
        update({ notifications: next && !blocked });
    };
    if (!supported)
        return _jsxs(_Fragment, { children: [_jsx(PaneTitle, { children: "Notifications" }), _jsx("p", { className: "text-base text-grey", children: "This browser does not support notifications." })] });
    return (_jsxs(_Fragment, { children: [_jsx(PaneTitle, { children: "Notifications" }), _jsx(Row, { label: "Reminders", hint: "Notify when a task or habit reminder comes due. Works while Tick Local is open in a tab.", htmlFor: "notify", children: _jsx(Switch, { id: "notify", checked: on, onChange: (v) => void toggle(v), label: "Reminders" }) }), denied && _jsx("p", { role: "status", className: "mt-3 text-s text-grey", children: "Notifications are blocked for this site. Allow them in the browser's site settings, then turn this on." })] }));
}
