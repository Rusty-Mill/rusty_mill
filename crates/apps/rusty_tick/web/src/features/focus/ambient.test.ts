import { afterEach, describe, expect, it, vi } from 'vitest'
import { AMBIENT, RECIPES, fillNoise, isAmbient, startAmbient } from './ambient'

/** A repeatable stand-in for Math.random. */
function seeded(): () => number {
  let s = 12345
  return () => ((s = (s * 1103515245 + 12345) % 2147483648) / 2147483648)
}

const roughness = (xs: Float32Array): number => xs.reduce((sum, x, i) => (i ? sum + Math.abs(x - xs[i - 1]!) : 0), 0) / xs.length

describe('fillNoise', () => {
  it('stays within [-1, 1] and is not silent', () => {
    for (const kind of ['white', 'brown'] as const) {
      const out = new Float32Array(5000)
      fillNoise(out, kind, seeded())
      expect(Math.max(...out)).toBeLessThanOrEqual(1)
      expect(Math.min(...out)).toBeGreaterThanOrEqual(-1)
      expect(Math.max(...out.map(Math.abs))).toBeGreaterThan(0.1)
    }
  })
  it('brown noise is smoother (more low end) than white', () => {
    const white = new Float32Array(5000)
    const brown = new Float32Array(5000)
    fillNoise(white, 'white', seeded())
    fillNoise(brown, 'brown', seeded())
    expect(roughness(brown)).toBeLessThan(roughness(white) / 4)
  })
})

describe('sounds', () => {
  it('every sound but off has a recipe, quiet enough not to clip', () => {
    for (const a of AMBIENT.filter((s) => s.id !== 'off')) expect(RECIPES[a.id as keyof typeof RECIPES].gain).toBeLessThan(0.6)
    expect(isAmbient('rain')).toBe(true)
    expect(isAmbient('thunder')).toBe(false)
  })
})

describe('startAmbient', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('is a silent no-op without Web Audio, and for off', () => {
    expect(() => startAmbient('rain')()).not.toThrow()
    vi.stubGlobal('AudioContext', class { constructor() { throw new Error('should not be built') } })
    expect(() => startAmbient('off')()).not.toThrow()
  })

  it('builds a looping source and tears the context down on stop', () => {
    const calls: string[] = []
    const node = () => ({ connect: vi.fn(), start: () => calls.push('start'), stop: () => calls.push('stop'), frequency: { value: 0 }, gain: { value: 0 }, loop: false, buffer: null as unknown, type: '' })
    class FakeContext {
      sampleRate = 100
      destination = {}
      resume = async () => undefined
      close = async () => void calls.push('close')
      createBuffer = (_c: number, n: number) => ({ getChannelData: () => new Float32Array(n) })
      createBufferSource = node
      createBiquadFilter = node
      createGain = node
      createOscillator = node
    }
    vi.stubGlobal('AudioContext', FakeContext)
    const stop = startAmbient('waves') // brown noise, a lowpass and a swell
    expect(calls.filter((c) => c === 'start')).toHaveLength(2) // the source and the swell oscillator
    stop()
    expect(calls).toContain('close')
    expect(calls.filter((c) => c === 'stop')).toHaveLength(2)
  })
})
