// Colour ranges (design proposal §4.3): automatic over all frames so far (default), over this
// frame, symmetric around zero (diverging quantities), or typed in.
import type { FrameStats, QuantityStats } from './stats.ts'

export type RangeMode = 'all' | 'frame' | 'symmetric' | 'manual'

export const RANGE_MODES: readonly (readonly [RangeMode, string])[] = [
  ['all', 'All frames'],
  ['frame', 'This frame'],
  ['symmetric', 'Symmetric (±p99)'],
  ['manual', 'Manual'],
]

/**
 * What the frames seen so far span, per quantity: min, max and the largest 99th percentile of
 * |value| (0 while no frame has been decoded: the stream's `hello` ranges carry no percentile).
 */
export type Seen = Record<string, { min: number; max: number; absP99: number }>

/** Adds a frame's statistics to what was seen (a new object; the inputs are not changed). */
export function accumulate(seen: Seen, stats: FrameStats): Seen {
  const out = { ...seen }
  for (const [key, s] of Object.entries(stats)) {
    const prev = out[key]
    out[key] = prev
      ? {
          min: Math.min(prev.min, s.min),
          max: Math.max(prev.max, s.max),
          absP99: Math.max(prev.absP99, s.absP99),
        }
      : { min: s.min, max: s.max, absP99: s.absP99 }
  }
  return out
}

/**
 * Seeds what was seen from the stream's `hello` ranges (min/max of each field over the frames
 * before this viewer joined; velocity: of its magnitude). Components get ± the magnitude. They
 * carry no percentile, so symmetric ranges wait for decoded frames.
 */
export function seedFromHello(ranges: Record<string, [number, number]> | undefined): Seen {
  const out: Seen = {}
  for (const [name, [lo, hi]] of Object.entries(ranges ?? {})) {
    if (!Number.isFinite(lo) || !Number.isFinite(hi)) continue
    out[name] = { min: lo, max: hi, absP99: 0 }
    if (name === 'velocity') {
      out[`${name}.x`] = { min: -hi, max: hi, absP99: 0 }
      out[`${name}.y`] = { min: -hi, max: hi, absP99: 0 }
    }
  }
  return out
}

/** The colour range of a quantity; never empty (a constant field gets ±1 around its value). */
export function rangeFor(
  mode: RangeMode,
  frame: QuantityStats | undefined,
  seen: Seen[string] | undefined,
  manual: [number, number] | undefined,
): [number, number] {
  let r: [number, number]
  switch (mode) {
    case 'manual':
      r = manual ?? [0, 1]
      break
    case 'frame':
      r = frame ? [frame.min, frame.max] : [0, 1]
      break
    case 'symmetric': {
      const h = seen?.absP99 || frame?.absP99 || 1
      r = [-h, h]
      break
    }
    case 'all':
      r = seen ? [seen.min, seen.max] : frame ? [frame.min, frame.max] : [0, 1]
      break
  }
  if (!(r[1] > r[0])) {
    const c = Number.isFinite(r[0]) ? r[0] : 0
    const d = Math.abs(c) > 0 ? Math.abs(c) * 1e-3 : 1
    r = [c - d, c + d]
  }
  return r
}

/** Five evenly spaced tick values of a range (the legend). */
export function ticks([lo, hi]: [number, number], n = 5): number[] {
  return Array.from({ length: n }, (_, i) => lo + ((hi - lo) * i) / (n - 1))
}
