import { createHashRouter, Navigate, type RouteObject } from 'react-router-dom'
import { CalendarPage } from '@/features/calendar/CalendarPage'
import { FocusPage } from '@/features/focus/FocusPage'
import { HabitsPage } from '@/features/habits/HabitsPage'
import { SummaryPage } from '@/features/summary/SummaryPage'
import { CompletedPage, TrashPage } from '@/features/tasks/HistoryPage'
import { TasksPage } from '@/features/tasks/TasksPage'
import { HOME } from './paths'
import { Shell, WithSidebar } from './Shell'

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
            { path: 'q/all/completed/:taskId?', element: <CompletedPage /> },
            { path: 'q/all/trash/:taskId?', element: <TrashPage /> },
            { path: 'q/all/summary', element: <SummaryPage /> },
          ],
        },
        { path: 'c/all/calendar/:mode?', element: <CalendarPage /> },
        { path: 'focus', element: <FocusPage /> },
        { path: 'q/all/habit', element: <HabitsPage /> },
        { path: '*', element: home },
      ],
    },
  ]

/** The real router: routes live in the URL hash so they match the reference app's. */
export const createRouter = () => createHashRouter(routes)
