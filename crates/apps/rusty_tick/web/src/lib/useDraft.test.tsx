import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useDraft } from './useDraft'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe('useDraft', () => {
  it('starts from the server value', () => {
    const { result } = renderHook(() => useDraft('a', 'hello', vi.fn()))
    expect(result.current.value).toBe('hello')
    expect(result.current.dirty).toBe(false)
  })

  it('saves once, 500ms after the last edit', () => {
    const save = vi.fn()
    const { result } = renderHook(() => useDraft<string>('a', '', save))
    act(() => result.current.set('h'))
    act(() => vi.advanceTimersByTime(300))
    act(() => result.current.set('he'))
    act(() => vi.advanceTimersByTime(300))
    expect(save).not.toHaveBeenCalled() // the second edit restarted the clock
    act(() => vi.advanceTimersByTime(250))
    expect(save).toHaveBeenCalledOnce()
    expect(save).toHaveBeenCalledWith('he')
    expect(result.current.dirty).toBe(false)
  })

  it('flush saves immediately, and only if there is something to save', () => {
    const save = vi.fn()
    const { result } = renderHook(() => useDraft<string>('a', 'x', save))
    act(() => result.current.flush())
    expect(save).not.toHaveBeenCalled()
    act(() => result.current.set('y'))
    act(() => result.current.flush())
    expect(save).toHaveBeenCalledWith('y')
    act(() => vi.advanceTimersByTime(1000))
    expect(save).toHaveBeenCalledOnce() // the pending timer was cancelled
  })

  it('saves what is pending when the component goes away', () => {
    const save = vi.fn()
    const { result, unmount } = renderHook(() => useDraft<string>('a', '', save))
    act(() => result.current.set('unsaved'))
    unmount()
    expect(save).toHaveBeenCalledWith('unsaved')
  })

  it('adopts a new server value when nothing is unsaved', () => {
    const { result, rerender } = renderHook(({ v }) => useDraft('a', v, vi.fn()), { initialProps: { v: 'one' } })
    rerender({ v: 'two' })
    expect(result.current.value).toBe('two')
  })

  it('does not overwrite an edit in progress', () => {
    const { result, rerender } = renderHook(({ v }) => useDraft('a', v, vi.fn()), { initialProps: { v: 'one' } })
    act(() => result.current.set('typing…'))
    rerender({ v: 'from the server' })
    expect(result.current.value).toBe('typing…')
  })

  it('resets for another record, saving the old edit first', () => {
    const save = vi.fn()
    const { result, rerender } = renderHook(({ k, v }) => useDraft(k, v, save), { initialProps: { k: 'a', v: 'A' } })
    act(() => result.current.set('A edited'))
    rerender({ k: 'b', v: 'B' })
    expect(save).toHaveBeenCalledWith('A edited')
    expect(result.current.value).toBe('B')
    expect(result.current.dirty).toBe(false)
  })

  it("saves a pending edit to the record it was made on, not the one we moved to", () => {
    const saved: string[] = []
    // Like the detail pane: `save` closes over the id of the record being shown.
    const { result, rerender } = renderHook(({ id, v }) => useDraft(id, v, (value) => saved.push(`${id}=${value}`)), { initialProps: { id: 'A', v: 'a' } })
    act(() => result.current.set('a edited'))
    rerender({ id: 'B', v: 'b' })
    expect(saved).toEqual(['A=a edited'])
    expect(result.current.value).toBe('b')
    act(() => result.current.set('b edited'))
    act(() => vi.advanceTimersByTime(600))
    expect(saved).toEqual(['A=a edited', 'B=b edited'])
  })

  it('compares with the given equality, so equal arrays are not a change', () => {
    const eq = (a: number[], b: number[]) => JSON.stringify(a) === JSON.stringify(b)
    const { result, rerender } = renderHook(({ v }) => useDraft('a', v, vi.fn(), 500, eq), { initialProps: { v: [1] } })
    const before = result.current.value
    rerender({ v: [1] })
    expect(result.current.value).toBe(before)
  })
})
