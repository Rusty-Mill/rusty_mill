import { jsx as _jsx } from "react/jsx-runtime";
import { createHashRouter, Navigate } from 'react-router-dom';
import { CompletedPage, TrashPage } from '@/features/tasks/HistoryPage';
import { TasksPage } from '@/features/tasks/TasksPage';
import { HOME } from './paths';
import { Shell, WithSidebar } from './Shell';
/** A route whose page is fetched on first visit, so the task list ships without calendar, summary, etc. */
const page = (load, name) => async () => ({ Component: (await load())[name] });
const home = _jsx(Navigate, { to: HOME, replace: true });
/** Unknown hashes go to All, as in the reference app. */
export const routes = [
    {
        element: _jsx(Shell, {}),
        children: [
            {
                element: _jsx(WithSidebar, {}),
                children: [
                    { path: 'q/:smart/tasks/:taskId?', element: _jsx(TasksPage, {}) },
                    { path: 'p/:listId/tasks/:taskId?', element: _jsx(TasksPage, {}) },
                    { path: 't/:tag/tasks/:taskId?', element: _jsx(TasksPage, {}) },
                    { path: 'f/:filterId/tasks/:taskId?', element: _jsx(TasksPage, {}) },
                    { path: 'q/all/completed/:taskId?', element: _jsx(CompletedPage, {}) },
                    { path: 'q/all/trash/:taskId?', element: _jsx(TrashPage, {}) },
                    { path: 'q/all/summary', lazy: page(() => import('@/features/summary/SummaryPage'), 'SummaryPage') },
                ],
            },
            { path: 'c/all/calendar/:mode?', lazy: page(() => import('@/features/calendar/CalendarPage'), 'CalendarPage') },
            { path: 'focus', lazy: page(() => import('@/features/focus/FocusPage'), 'FocusPage') },
            { path: 'countdown', lazy: page(() => import('@/features/countdown/CountdownPage'), 'CountdownPage') },
            { path: 'q/all/habit', lazy: page(() => import('@/features/habits/HabitsPage'), 'HabitsPage') },
            { path: '*', element: home },
        ],
    },
];
/** The real router: routes live in the URL hash so they match the reference app's. */
export const createRouter = () => createHashRouter(routes);
