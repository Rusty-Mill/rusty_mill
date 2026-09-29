import { PaneTitle } from './controls'

/** Panes for things Tick Local does not have. Honest about it; nothing to buy or enable. */
export function PlaceholderTab({ title, note }: { title: string; note?: string }) {
  return (
    <>
      <PaneTitle>{title}</PaneTitle>
      <p className="text-base text-grey">{note ?? `${title} is not available in Tick Local.`}</p>
    </>
  )
}
