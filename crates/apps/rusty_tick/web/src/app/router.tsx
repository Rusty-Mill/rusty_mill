import { createHashRouter, Navigate, type RouteObject } from 'react-router-dom'
import { CompletedPage, TrashPage } from '@/features/tasks/HistoryPage'
import { TasksPage } from '@/features/tasks/TasksPage'
import { HOME } from './paths'
import { Shell, WithSidebar } from './Shell'

/** A route whose page is fetched on first visit, so the task list ships without calendar, summary, etc. */
const page = <M,>(load: () => Promise<M>, name: keyof M) => async () => ({ Component: (await load())[name] as React.ComponentType })

const home = <Navigate to={HOME} replace />

/** Unknown hashes go to All, as in the reference app. */
export const routes: RouteObject[] = [
    {
      element: <Shell />,
      children: [
        {
          element: <WithSidebar />,
          children: [
            { path: 'q/:smart/tasks/:taskId?', element: <TasksPage /> },
            { path: 'p/:listId/tasks/:taskId?', element: <TasksPage /> },
            { path: 't/:tag/tasks/:taskId?', element: <TasksPage /> },
            { path: 'f/:filterId/tasks/:taskId?', element: <TasksPage /> },
            { path: 'q/all/completed/:taskId?', element: <CompletedPage /> },
            { path: 'q/all/trash/:taskId?', element: <TrashPage /> },
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
  ]

/** The real router: routes live in the URL hash so they match the reference app's. */
export const createRouter = () => createHashRouter(routes)
