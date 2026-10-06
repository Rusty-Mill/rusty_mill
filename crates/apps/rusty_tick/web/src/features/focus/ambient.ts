/**
 * Ambient sound for focus sessions, synthesised with Web Audio so no audio files
 * ship: a noise source through a filter, optionally swelling slowly (waves).
 * `fillNoise` and `RECIPES` are pure; `startAmbient` is the only part that touches the browser.
 */
export const AMBIENT = [
  { id: 'off', label: 'Off' },
  { id: 'white', label: 'White noise' },
  { id: 'rain', label: 'Rain' },
  { id: 'waves', label: 'Waves' },
] as const

export type AmbientId = (typeof AMBIENT)[number]['id']

export const isAmbient = (v: unknown): v is AmbientId => AMBIENT.some((a) => a.id === v)

export interface Recipe {
  noise: 'white' | 'brown'
  highpassHz: number | null
  lowpassHz: number | null
  gain: number
  /** Hz of a slow gain swell; 0 for steady. */
  swellHz: number
}

export const RECIPES: Record<Exclude<AmbientId, 'off'>, Recipe> = {
  white: { noise: 'white', highpassHz: null, lowpassHz: null, gain: 0.12, swellHz: 0 },
  rain: { noise: 'white', highpassHz: 900, lowpassHz: 9000, gain: 0.2, swellHz: 0 },
  waves: { noise: 'brown', highpassHz: null, lowpassHz: 700, gain: 0.5, swellHz: 0.1 },
}

/** Fill `out` with noise in [-1, 1]: white is uniform; brown is white integrated, so it is all low end. */
export function fillNoise(out: Float32Array, kind: Recipe['noise'], rand: () => number = Math.random): void {
  let last = 0
  for (let i = 0; i < out.length; i++) {
    const white = rand() * 2 - 1
    if (kind === 'white') out[i] = white
    else {
      last = (last + 0.02 * white) / 1.02
      out[i] = Math.max(-1, Math.min(1, last * 3.5))
    }
  }
}

const SECONDS = 4 // the loop; long enough that the repeat is not noticeable in noise

/** Start the sound; the returned function stops it. A no-op where Web Audio is missing. */
export function startAmbient(id: AmbientId): () => void {
  const Ctx = (globalThis as { AudioContext?: typeof AudioContext }).AudioContext
  if (id === 'off' || !Ctx) return () => undefined
  const r = RECIPES[id]
  const ctx = new Ctx()
  void ctx.resume?.()
  const buffer = ctx.createBuffer(1, ctx.sampleRate * SECONDS, ctx.sampleRate)
  fillNoise(buffer.getChannelData(0), r.noise)
  const source = ctx.createBufferSource()
  source.buffer = buffer
  source.loop = true

  let node: AudioNode = source
  const chain = (filter: BiquadFilterNode, type: BiquadFilterType, hz: number): void => {
    filter.type = type
    filter.frequency.value = hz
    node.connect(filter)
    node = filter
  }
  if (r.highpassHz) chain(ctx.createBiquadFilter(), 'highpass', r.highpassHz)
  if (r.lowpassHz) chain(ctx.createBiquadFilter(), 'lowpass', r.lowpassHz)
  const gain = ctx.createGain()
  gain.gain.value = r.gain
  node.connect(gain)
  gain.connect(ctx.destination)

  let lfo: OscillatorNode | null = null
  if (r.swellHz > 0) {
    lfo = ctx.createOscillator()
    lfo.frequency.value = r.swellHz
    const depth = ctx.createGain()
    depth.gain.value = r.gain * 0.6
    lfo.connect(depth)
    depth.connect(gain.gain)
    lfo.start()
  }
  source.start()
  return () => {
    source.stop()
    lfo?.stop()
    void ctx.close()
  }
}
