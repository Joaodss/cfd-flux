// Statistics of a decoded frame per quantity (min, max, the 99th percentile of |value|), over
// fluid cells only. Computed in the decode worker, next to decoding, so the page's thread never
// loops over a whole frame.
import { halfTable } from '../frames/half.ts'
import type { DecodedField, DecodedFrame } from '../frames/lffr.ts'
import { quantitiesOf, type Component } from './quantities.ts'

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

/** One quantity of a field as numbers, divided by the block's fluid fraction (NaN outside). */
function reader(
  field: DecodedField,
  component: Component,
  fraction: Float32Array | null,
): { n: number; at: (i: number) => number } {
  const half = halfTable()
  const raw = field.data
  const get = field.dtype === 'f16' ? (j: number) => half[raw[j]] : (j: number) => raw[j]
  const c = field.components
  const n = raw.length / c
  const value = (i: number): number => {
    const a = get(i * c)
    if (c === 1) return a
    const b = get(i * c + 1)
    return component === 'magnitude' ? Math.hypot(a, b) : component === 'y' ? b : a
  }
  const f = fraction && fraction.length === n ? fraction : null
  return {
    n,
    at: f ? (i) => (f[i] > 0 ? value(i) / f[i] : NaN) : value,
  }
}

/** Statistics of every quantity of a frame; `fraction` (per block) excludes non-fluid blocks. */
export function frameStats(frame: DecodedFrame, fraction: Float32Array | null): FrameStats {
  const out: FrameStats = {}
  for (const q of quantitiesOf(frame.header.fields)) {
    const field = frame.fields.find((f) => f.name === q.field)
    if (!field) continue
    const { n, at } = reader(field, q.component, fraction)
    let min = Infinity
    let max = -Infinity
    let nonFinite = 0
    let fluid = 0
    for (let i = 0; i < n; i++) {
      const v = at(i)
      if (Number.isNaN(v) && fraction && fraction[i] === 0) continue // outside the fluid
      fluid++
      if (!Number.isFinite(v)) {
        nonFinite++
        continue
      }
      if (v < min) min = v
      if (v > max) max = v
    }
    const absMax = Math.max(Math.abs(min), Math.abs(max))
    let absP99 = 0
    if (Number.isFinite(absMax) && absMax > 0) {
      // A histogram of |v|; the percentile is the upper edge of the bin that reaches 99%.
      const hist = new Uint32Array(BINS)
      let counted = 0
      for (let i = 0; i < n; i++) {
        const v = Math.abs(at(i))
        if (!Number.isFinite(v)) continue
        hist[Math.min(BINS - 1, Math.floor((v / absMax) * BINS))]++
        counted++
      }
      let sum = 0
      let bin = 0
      while (bin < BINS && sum + hist[bin] < 0.99 * counted) sum += hist[bin++]
      absP99 = (absMax * Math.min(BINS, bin + 1)) / BINS
    }
    out[q.key] =
      fluid - nonFinite > 0
        ? { min, max, absP99, nonFinite }
        : { min: 0, max: 0, absP99: 0, nonFinite }
  }
  return out
}
