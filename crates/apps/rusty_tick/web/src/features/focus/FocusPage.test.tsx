import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createServices, ServicesProvider } from '@/app/services'
import { FocusPage } from './FocusPage'
import { resetEstimatesStore } from '../estimates/store'
import { resetFocusStore, useFocus } from './store'

const START = new Date(2026, 8, 29, 10, 0, 0).getTime()

async function setup() {
  const services = createServices('demo')
  let utils!: ReturnType<typeof render>
  await act(async () => {
    utils = render(
      <ServicesProvider services={services}>
        <FocusPage />
      </ServicesProvider>,
    )
  })
  return { services, ...utils }
}

// userEvent waits on real timers internally, which deadlock under fake ones; fireEvent is synchronous.
const click = (el: HTMLElement): void => void act(() => { fireEvent.click(el) })
const advance = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms) })

describe('FocusPage', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] })
    vi.setSystemTime(START)
    resetFocusStore()
    resetEstimatesStore()
  })
  afterEach(() => {
    cleanup() // unmount first so resetting the store does not update a live page
    resetFocusStore()
    resetEstimatesStore()
    vi.useRealTimers()
  })

  it('shows the idle timer and the empty record state', async () => {
    await setup()
    expect(screen.getByText('25:00')).toBeInTheDocument()
    expect(screen.getByText('No focus record yet')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Start' })).toBeInTheDocument()
  })

  it('counts down from timestamps, pauses and continues', async () => {
    await setup()
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(61_000)
    expect(screen.getByText('23:59')).toBeInTheDocument()
    click(screen.getByRole('button', { name: 'Pause' }))
    await advance(120_000)
    expect(screen.getByText('23:59')).toBeInTheDocument() // frozen
    click(screen.getByRole('button', { name: 'Continue' }))
    await advance(60_000)
    expect(screen.getByText('22:59')).toBeInTheDocument()
  })

  it('stays correct when the clock jumps (a throttled tab)', async () => {
    await setup()
    click(screen.getByRole('button', { name: 'Start' }))
    act(() => { vi.setSystemTime(START + 10 * 60_000) })
    await advance(300)
    expect(screen.getByText('15:00')).toBeInTheDocument()
  })

  it('records a finished pomo, lists it and offers a break', async () => {
    const { services } = await setup()
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(25 * 60_000 + 500)
    expect(screen.getByRole('button', { name: 'Start break' })).toBeInTheDocument()
    const list = screen.getByRole('list', { name: 'Focus records' })
    expect(within(list).getByText('25m')).toBeInTheDocument()
    expect(within(list).getByText('Today')).toBeInTheDocument()
    expect(screen.queryByText('No focus record yet')).toBeNull()
    const docs = await services.api.listDocs('focus')
    expect(docs.filter((d) => (d.body as { kind?: string }).kind === 'pomo')).toHaveLength(1)
    // Take the break: a 5:00 countdown.
    click(screen.getByRole('button', { name: 'Start break' }))
    expect(screen.getByText('05:00')).toBeInTheDocument()
    await advance(5 * 60_000 + 500)
    expect(screen.getByRole('button', { name: 'Start' })).toBeInTheDocument()
  })

  it('skips the offered break', async () => {
    await setup()
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(25 * 60_000 + 500)
    click(screen.getByRole('button', { name: 'Skip' }))
    expect(screen.getByRole('button', { name: 'Start' })).toBeInTheDocument()
  })

  it('giving up a pomo records nothing', async () => {
    await setup()
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(60_000)
    click(screen.getByRole('button', { name: 'Give up' }))
    click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Give up' }))
    expect(screen.getByText('No focus record yet')).toBeInTheDocument()
    expect(screen.getByText('25:00')).toBeInTheDocument()
  })

  it('stopwatch counts up and records on stop; the record can be deleted', async () => {
    const { services } = await setup()
    click(screen.getByRole('tab', { name: 'Stopwatch' }))
    expect(screen.getByText('00:00')).toBeInTheDocument()
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(90_000)
    expect(screen.getByText('01:30')).toBeInTheDocument()
    click(screen.getByRole('button', { name: 'Stop' }))
    const list = screen.getByRole('list', { name: 'Focus records' })
    expect(within(list).getByText('Stopwatch')).toBeInTheDocument()
    expect(within(list).getByText('1m')).toBeInTheDocument()
    expect((await services.api.listDocs('focus')).some((d) => (d.body as { kind?: string }).kind === 'stopwatch')).toBe(true)
    click(screen.getByRole('button', { name: /Delete record/ }))
    expect(screen.getByText('No focus record yet')).toBeInTheDocument()
    expect((await services.api.listDocs('focus')).filter((d) => (d.body as { kind?: string }).kind === 'stopwatch')).toHaveLength(0)
  })

  it('keeps a running session when the page is left and re-entered', async () => {
    const { unmount, services } = await setup()
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(30_000)
    unmount()
    await advance(60_000)
    await act(async () => { render(<ServicesProvider services={services}><FocusPage /></ServicesProvider>) })
    expect(screen.getByText('23:30')).toBeInTheDocument()
    expect(useFocus.getState().session).not.toBeNull()
  })

  it('links a task and shows its title on the record', async () => {
    const { services } = await setup()
    await act(async () => { await services.store.getState().boot() })
    const title = Object.values(services.store.getState().tasks).find((t) => t.status === 'open' && t.deletedMs === null)?.title
    expect(title).toBeTruthy()
    act(() => { fireEvent.change(screen.getByRole('combobox', { name: 'Task' }), { target: { value: Object.values(services.store.getState().tasks).find((t) => t.title === title)!.id } }) })
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(25 * 60_000 + 500)
    expect(within(screen.getByRole('list', { name: 'Focus records' })).getByText(title!)).toBeInTheDocument()
  })
  it('logs interruptions during a session and puts them on the record', async () => {
    await setup()
    click(screen.getByRole('tab', { name: 'Stopwatch' }))
    click(screen.getByRole('button', { name: 'Start' }))
    click(screen.getByRole('button', { name: 'Interrupted' }))
    click(screen.getByRole('button', { name: 'Interrupted' }))
    expect(screen.getByText('Interruptions: 2')).toBeInTheDocument()
    await advance(5_000)
    click(screen.getByRole('button', { name: 'Stop' }))
    expect(within(screen.getByRole('list', { name: 'Focus records' })).getByText(/2 interruptions/)).toBeInTheDocument()
  })

  it('estimates pomos for a task and compares them with the pomos done', async () => {
    const { services } = await setup()
    await act(async () => { await services.store.getState().boot() })
    const task = Object.values(services.store.getState().tasks).find((t) => t.status === 'open' && t.deletedMs === null)!
    act(() => { fireEvent.change(screen.getByRole('combobox', { name: 'Task' }), { target: { value: task.id } }) })
    click(screen.getByRole('button', { name: 'More estimated pomos' }))
    click(screen.getByRole('button', { name: 'More estimated pomos' }))
    expect(within(screen.getByRole('group', { name: 'Pomo estimate' })).getByText('2')).toBeInTheDocument()
    click(screen.getByRole('button', { name: 'Start' }))
    await advance(25 * 60_000 + 500)
    expect(within(screen.getByRole('region', { name: 'Estimates' })).getByText('1 / 2 pomos')).toBeInTheDocument()
  })
  it('plays the chosen ambient sound only while a focus session runs', async () => {
    const started: string[] = []
    const stopped: string[] = []
    vi.stubGlobal('AudioContext', class {
      sampleRate = 100
      destination = {}
      resume = async () => undefined
      close = async () => void stopped.push('ctx')
      createBuffer = (_c: number, n: number) => ({ getChannelData: () => new Float32Array(n) })
      createBufferSource = () => ({ connect: () => undefined, start: () => started.push('src'), stop: () => undefined, loop: false, buffer: null })
      createBiquadFilter = () => ({ connect: () => undefined, frequency: { value: 0 }, type: '' })
      createGain = () => ({ connect: () => undefined, gain: { value: 0 } })
    })
    try {
      await setup()
      act(() => { fireEvent.change(screen.getByRole('combobox', { name: 'Ambient sound' }), { target: { value: 'rain' } }) })
      expect(started).toHaveLength(0) // idle: silent
      click(screen.getByRole('button', { name: 'Start' }))
      expect(started).toHaveLength(1)
      click(screen.getByRole('button', { name: 'Pause' }))
      expect(stopped).toHaveLength(1) // paused: stopped
    } finally {
      vi.unstubAllGlobals()
    }
  })
})
