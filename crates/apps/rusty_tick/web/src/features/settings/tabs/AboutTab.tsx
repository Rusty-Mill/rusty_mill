import pkg from '../../../../package.json'
import { PaneTitle } from './controls'

export const APP_NAME = 'Tick Local'
export const APP_VERSION: string = pkg.version

export function AboutTab() {
  return (
    <>
      <PaneTitle>About</PaneTitle>
      <p className="text-base font-semibold">{APP_NAME}</p>
      <p className="mb-3 text-s text-grey">Version {APP_VERSION}</p>
      <p className="text-base text-grey">A self-hosted task manager in the style of TickTick. Your lists, tasks, habits and settings are stored on your own server, or in this browser in demo mode.</p>
    </>
  )
}
