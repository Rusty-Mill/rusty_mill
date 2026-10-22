import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { STORAGE_KEY } from '@/api/memory';
import { getMode, getToken, setMode, setToken } from '@/app/env';
import { createServices, ServicesProvider } from '@/app/services';
import { atTime } from '@/lib/date';
import { AccountMenu } from './AccountMenu';
import { page } from './session';
function Host() {
    const [anchor, setAnchor] = useState(null);
    const [open, setOpen] = useState(false);
    return (_jsxs(_Fragment, { children: [_jsx("button", { ref: setAnchor, "aria-label": "Account", onClick: () => setOpen(true) }), _jsx(AccountMenu, { anchor: anchor, open: open, onClose: () => setOpen(false) }), _jsx("output", { "data-testid": "where", children: useLocation().search })] }));
}
async function setup(mode = 'demo', prepare) {
    const services = createServices(mode);
    await prepare?.(services);
    const user = userEvent.setup();
    render(_jsx(ServicesProvider, { services: services, children: _jsx(MemoryRouter, { children: _jsx(Host, {}) }) }));
    await user.click(screen.getByRole('button', { name: 'Account' }));
    return user;
}
afterEach(() => vi.restoreAllMocks());
describe('AccountMenu', () => {
    it('lists Settings, Statistics, a disabled Premium and Sign Out', async () => {
        await setup('server');
        const menu = screen.getByRole('menu', { name: 'Account' });
        expect(within(menu).getAllByRole('menuitem').map((i) => i.textContent)).toEqual(['Settings', 'Statistics', 'PremiumNot applicable in Tick Local', 'Sign Out']);
        expect(within(menu).getByRole('menuitem', { name: /Premium/ })).toHaveAttribute('aria-disabled', 'true');
    });
    it('Settings goes to the account tab', async () => {
        const user = await setup();
        await user.click(screen.getByRole('menuitem', { name: 'Settings' }));
        expect(screen.getByTestId('where')).toHaveTextContent('?modalType=settings&tabs=account');
        expect(screen.queryByRole('menu')).toBeNull();
    });
    it('Premium does nothing', async () => {
        const user = await setup();
        await user.click(screen.getByRole('menuitem', { name: /Premium/ }));
        expect(screen.getByRole('menu')).toBeInTheDocument();
        expect(screen.getByTestId('where')).toBeEmptyDOMElement();
    });
    it('Statistics opens a dialog of real numbers', async () => {
        const user = await setup('demo', async (s) => {
            await s.store.getState().boot();
            const a = s.store.getState();
            for (const id of Object.keys(a.tasks))
                await a.trashTask(id); // start from a clean slate, not the sample data
            const t = await a.createTask({ listId: a.inboxId, title: 'one', dueMs: atTime(Date.now(), 12) });
            await a.toggleDone(t.id);
            await a.createTask({ listId: a.inboxId, title: 'two', dueMs: Date.now() - 3 * 86_400_000, isAllDay: true });
        });
        await user.click(screen.getByRole('menuitem', { name: 'Statistics' }));
        const dialog = screen.getByRole('dialog', { name: 'Statistics' });
        const figure = (label) => within(dialog).getByText(label).previousElementSibling.textContent ?? '';
        expect(figure('Completed today')).toBe('1');
        expect(figure('Completed this week')).toBe('1');
        expect(figure('Completed all time')).toBe('1');
        expect(figure('Overdue')).toBe('1');
        expect(figure('Open tasks')).toBe('1');
        expect(figure('Day streak')).toBe('1');
        expect(within(dialog).getByText('Inbox')).toBeInTheDocument();
        await user.keyboard('{Escape}');
        expect(screen.queryByRole('dialog')).toBeNull();
    });
    it('Sign Out (server) clears the token and reloads', async () => {
        setToken('tok-1234567890', true);
        const reload = vi.spyOn(page, 'reload').mockImplementation(() => undefined);
        const user = await setup('server');
        await user.click(screen.getByRole('menuitem', { name: 'Sign Out' }));
        expect(getToken()).toBeNull();
        expect(reload).toHaveBeenCalledOnce();
    });
    it('Leave demo forgets the mode so the welcome screen returns, and reloads without ?adapter', async () => {
        setMode('demo');
        const replace = vi.spyOn(page, 'replace').mockImplementation(() => undefined);
        const user = await setup('demo');
        localStorage.setItem(STORAGE_KEY, '{}');
        expect(screen.queryByRole('menuitem', { name: 'Sign Out' })).toBeNull();
        await user.click(screen.getByRole('menuitem', { name: 'Leave demo' }));
        expect(getMode()).toBeNull();
        expect(replace).toHaveBeenCalledOnce();
        expect(localStorage.getItem(STORAGE_KEY)).not.toBeNull(); // leaving is not deleting
    });
});
