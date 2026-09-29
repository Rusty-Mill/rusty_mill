import { DetailArt } from '@/components/Illustrations'
import type { ViewSpec } from './organize'

export function DetailPane({ taskId }: { spec: ViewSpec; taskId: string | null }) {
  return (
    <aside aria-label="Task details" className="flex w-[500px] shrink-0 items-center justify-center border-l border-line">
      {taskId ? <span className="text-grey">Task {taskId}</span> : <DetailArt />}
    </aside>
  )
}
