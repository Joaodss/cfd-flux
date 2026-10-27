// Statistics of a decoded frame per quantity (min, max, the 99th percentile of |value|), over
// fluid cells only. Computed in the decode worker, next to decoding, so the page's thread never
// loops over a whole frame.
//
// Playback at 1024² leaves the worker a few milliseconds per frame, so these are plain indexed
// loops over typed arrays: each component of a field is read once — multiplied by the inverse
// of its block's fluid fraction into a reused `Float32Array`, with its min/max on the way — |u|
// comes from those, and the percentile's histogram reads a strided subsample.
import { halfTable } from '../frames/half.ts'
import type { DecodedField, DecodedFrame } from '../frames/lffr.ts'
import { quantitiesOf } from './quantities.ts'

export interface QuantityStats {
  min: number
  max: number
  /** 99th percentile of |value| (the default half-width of symmetric ranges). */
  absP99: number
  /** Values that are ∞ or NaN (e.g. beyond f16's ±65,504). */
  nonFinite: number
}

export type FrameStats = Record<string, QuantityStats>

const BINS = 2048
/** Values the percentile's histogram looks at, at most: a stride over larger frames. */
const SAMPLES = 1 << 18

/** Scratch arrays (one per component slot), kept between frames: the worker does one at a time. */
const scratch: Float32Array[] = []
function buffer(slot: number, n: number): Float32Array {
  let b = scratch[slot]
  if (!b || b.length < n) b = scratch[slot] = new Float32Array(n)
  return b
}

/**
 * Per block, what a value is multiplied by: 1 / fluid fraction, NaN outside the fluid (1
 * everywhere without a mask); and how many blocks are outside. Kept while the mask stays.
 */
let weights: { of: Float32Array | null; n: number; inverse: Float32Array; outside: number } | null =
  null
function weightsFor(fraction: Float32Array | null, n: number) {
  if (weights && weights.of === fraction && weights.n === n) return weights
  const inverse = new Float32Array(n)
  let outside = 0
  if (fraction) {
    for (let i = 0; i < n; i++) {
      const f = fraction[i]
      inverse[i] = f > 0 ? 1 / f : NaN
      if (!(f > 0)) outside++
    }
  } else {
    inverse.fill(1)
  }
  weights = { of: fraction, n, inverse, outside }
  return weights
}

/** Min, max and the count of non-finite values (∞, NaN — outside the fluid included). */
interface Range {
  min: number
  max: number
  bad: number
}

/** Component `c` of a field, weighted, into `out`. Two loops, each over one kind of array. */
function component(field: DecodedField, c: number, w: Float32Array, out: Float32Array): Range {
  const stride = field.components
  const n = field.data.length / stride
  let min = Infinity
  let max = -Infinity
  let bad = 0
  if (field.dtype === 'f16') {
    const half = halfTable()
    const raw = field.data
    for (let i = 0; i < n; i++) {
      const v = half[raw[i * stride + c]] * w[i]
      out[i] = v
      if (v - v === 0) {
        if (v < min) min = v
        if (v > max) max = v
      } else bad++
    }
  } else {
    const raw = field.data
    for (let i = 0; i < n; i++) {
      const v = raw[i * stride + c] * w[i]
      out[i] = v
      if (v - v === 0) {
        if (v < min) min = v
        if (v > max) max = v
      } else bad++
    }
  }
  return { min, max, bad }
}

/** |(x, y)| into `out`. */
function magnitude(x: Float32Array, y: Float32Array, n: number, out: Float32Array): Range {
  let min = Infinity
  let max = -Infinity
  let bad = 0
  for (let i = 0; i < n; i++) {
    const v = Math.sqrt(x[i] * x[i] + y[i] * y[i])
    out[i] = v
    if (v - v === 0) {
      if (v < min) min = v
      if (v > max) max = v
    } else bad++
  }
  return { min, max, bad }
}

/** The statistics of `n` values from their range; `outside` of the non-finite ones are no fluid. */
function stats(values: Float32Array, n: number, range: Range, outside: number): QuantityStats {
  const { min, max } = range
  const nonFinite = range.bad - outside
  if (n - range.bad === 0) return { min: 0, max: 0, absP99: 0, nonFinite }
  const absMax = Math.max(Math.abs(min), Math.abs(max))
  let absP99 = 0
  if (absMax > 0) {
    // A histogram of |v|; the percentile is the upper edge of the bin that reaches 99%.
    const hist = new Uint32Array(BINS)
    const scale = BINS / absMax
    const step = Math.ceil(n / SAMPLES)
    let counted = 0
    for (let i = 0; i < n; i += step) {
      const v = values[i]
      if (v - v !== 0) continue
      const bin = (Math.abs(v) * scale) | 0
      hist[bin < BINS ? bin : BINS - 1]++
      counted++
    }
    let sum = 0
    let bin = 0
    while (bin < BINS && sum + hist[bin] < 0.99 * counted) sum += hist[bin++]
    absP99 = (absMax * Math.min(BINS, bin + 1)) / BINS
  }
  return { min, max, absP99, nonFinite }
}

/** Statistics of every quantity of a frame; `fraction` (per block) excludes non-fluid blocks. */
export function frameStats(frame: DecodedFrame, fraction: Float32Array | null): FrameStats {
  const out: FrameStats = {}
  const quantities = quantitiesOf(frame.header.fields)
  for (const field of frame.fields) {
    const own = quantities.filter((q) => q.field === field.name)
    if (own.length === 0) continue
    const n = field.data.length / field.components
    const { inverse, outside } = weightsFor(fraction && fraction.length === n ? fraction : null, n)
    const x = buffer(0, n)
    const rx = component(field, 0, inverse, x)
    if (field.components === 1) {
      for (const q of own) out[q.key] = stats(x, n, rx, outside)
      continue
    }
    const y = buffer(1, n)
    const ry = component(field, 1, inverse, y)
    const m = buffer(2, n)
    const rm = own.some((q) => q.component === 'magnitude') ? magnitude(x, y, n, m) : null
    for (const q of own) {
      out[q.key] =
        q.component === 'magnitude' && rm
          ? stats(m, n, rm, outside)
          : q.component === 'y'
            ? stats(y, n, ry, outside)
            : stats(x, n, rx, outside)
    }
  }
  return out
}
