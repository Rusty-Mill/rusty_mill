import { createHashRouter, Navigate } from 'react-router-dom'
import { CalendarPage } from '@/features/calendar/CalendarPage'
import { FocusPage } from '@/features/focus/FocusPage'
import { HabitsPage } from '@/features/habits/HabitsPage'
import { SummaryPage } from '@/features/summary/SummaryPage'
import { CompletedPage } from '@/features/tasks/CompletedPage'
import { TasksPage } from '@/features/tasks/TasksPage'
import { TrashPage } from '@/features/tasks/TrashPage'
import { HOME } from './paths'
import { Shell, WithSidebar } from './Shell'

const home = <Navigate to={HOME} replace />

/** Unknown hashes go to All, as in the reference app. */
export const createRouter = () =>
  createHashRouter([
    {
      element: <Shell />,
      children: [
        {
          element: <WithSidebar />,
          children: [
            { path: 'q/:smart/tasks/:taskId?', element: <TasksPage /> },
            { path: 'p/:listId/tasks/:taskId?', element: <TasksPage /> },
            { path: 't/:tag/tasks/:taskId?', element: <TasksPage /> },
            { path: 'q/all/completed', element: <CompletedPage /> },
            { path: 'q/all/trash', element: <TrashPage /> },
            { path: 'q/all/summary', element: <SummaryPage /> },
          ],
        },
        { path: 'c/all/calendar/:mode?', element: <CalendarPage /> },
        { path: 'focus', element: <FocusPage /> },
        { path: 'q/all/habit', element: <HabitsPage /> },
        { path: '*', element: home },
      ],
    },
  ])
