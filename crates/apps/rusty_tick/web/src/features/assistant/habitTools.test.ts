import { describe, expect, it } from 'vitest'
import { checkinId, type Checkin, type Habit } from '@/features/habits/logic'
import { habitTools, pickHabit, type HabitContext } from './habitTools'

const NOW = new Date(2026, 9, 22, 12).getTime() // a Thursday
const TODAY = '2026-10-22'
const habit = (id: string, name: string, over: Partial<Habit> = {}): Habit => ({
  id,
  name,
  color: '#000',
  goal: '1 time per day',
  frequency: { kind: 'daily' },
  reminder: null,
  createdMs: 0,
  archived: false,
  ...over,
})

/** A context over `habits` and `checkins` that records what the tools did. */
function context(habits: Habit[], checkins: Checkin[] = []): { c: HabitContext; done: string[] } {
  const done: string[] = []
  const c: HabitContext = {
    habits,
    checkins: Object.fromEntries(checkins.map((k) => [k.id, k])),
    now: NOW,
    add: (input) => {
      done.push(`add ${JSON.stringify(input)}`)
      return 'new'
    },
    setCount: (id, day, count) => void done.push(`set ${id} ${day} ${count}`),
  }
  return { c, done }
}
const checkin = (habitId: string, count: number): Checkin => ({ id: checkinId(habitId, TODAY), habitId, day: TODAY, count })
const run = async (c: HabitContext, name: string, args: Record<string, string>): Promise<unknown> =>
  habitTools(c).find((t) => t.name === name)!.handler!(args)

describe('pickHabit', () => {
  const habits = [habit('a', 'Read'), habit('b', 'Read news'), habit('c', 'Meditate'), habit('d', 'Old', { archived: true })]

  it('prefers an exact name, then the only name containing the query, and skips archived habits', () => {
    expect(pickHabit(habits, ' READ ')).toEqual({ habit: habits[0] })
    expect(pickHabit(habits, 'medit')).toEqual({ habit: habits[2] })
    expect(pickHabit(habits, 'e')).toEqual({ matches: ['Read', 'Read news', 'Meditate'] })
    expect(pickHabit(habits, 'old')).toEqual({ error: 'No habit matches “old”.' })
    expect(pickHabit(habits, '  ')).toEqual({ error: 'Say which habit.' })
  })
})

describe('habit tools', () => {
  const water = habit('w', 'Water', { goal: '8 glasses per day' })
  const gym = habit('g', 'Gym', { frequency: { kind: 'weekdays', days: [1, 3] } }) // not due on Thursday

  it('lists the habits due today with their progress', async () => {
    const { c } = context([water, gym, habit('m', 'Meditate')], [checkin('w', 3)])
    expect(await run(c, 'list_habits', {})).toEqual({ habits: ['Water (3/8)', 'Meditate (0/1)'] })
  })

  it('checks in once more up to the goal, and no further', async () => {
    const { c, done } = context([water, habit('m', 'Meditate')], [checkin('w', 3), checkin('m', 1)])
    expect(await run(c, 'check_in_habit', { query: 'water' })).toEqual({ title: 'Water', progress: '4/8' })
    expect(await run(c, 'check_in_habit', { query: 'meditate' })).toEqual({ title: 'Meditate', progress: '1/1', already: 'yes' })
    expect(done).toEqual([`set w ${TODAY} 4`])
  })

  it('clears today on undo, and changes nothing for a habit it cannot name', async () => {
    const { c, done } = context([water, habit('w2', 'Water plants')], [checkin('w', 3)])
    expect(await run(c, 'check_in_habit', { query: 'water', undo: 'true' })).toEqual({ title: 'Water', undone: 'yes' })
    expect(await run(c, 'check_in_habit', { query: 'wat' })).toEqual({ matches: ['Water', 'Water plants'] })
    expect(await run(c, 'check_in_habit', { query: '' })).toEqual({ error: 'Say which habit.' })
    expect(done).toEqual([`set w ${TODAY} 0`])
  })

  it('adds a daily habit once, and refuses a blank or taken name', async () => {
    const { c, done } = context([water])
    expect(await run(c, 'add_habit', { name: ' Stretch ' })).toEqual({ title: 'Stretch' })
    expect(await run(c, 'add_habit', { name: 'water' })).toEqual({ error: 'You already have a habit called “water”.' })
    expect(await run(c, 'add_habit', { name: ' ' })).toEqual({ error: 'Say the habit’s name.' })
    expect(done).toHaveLength(1)
    expect(JSON.parse(done[0]!.slice(4))).toMatchObject({ name: 'Stretch', goal: '1 time per day', frequency: { kind: 'daily' }, reminder: null })
  })
})
