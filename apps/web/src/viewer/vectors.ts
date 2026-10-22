// The scale of the velocity arrows: a length proportional to |u| that keeps the fastest arrow
// inside its grid cell, and the "nice" reference value drawn in the legend.

/** Distance between arrows, in CSS pixels. */
export const ARROW_SPACING = 24

/** The largest 1, 2 or 5 × 10ⁿ not above `v` (0 for v ≤ 0). */
export function niceBelow(v: number): number {
  if (!(v > 0)) return 0
  const e = Math.floor(Math.log10(v))
  const m = v / 10 ** e
  return (m >= 5 ? 5 : m >= 2 ? 2 : 1) * 10 ** e
}

export interface ArrowScale {
  /** CSS pixels per m/s. */
  scale: number
  /** The legend's reference arrow: its speed (m/s) and length (CSS pixels). */
  reference: { value: number; length: number }
}

/** The fastest speed (`vmax`, m/s) maps to 90 % of the spacing. Null without flow. */
export function arrowScale(vmax: number, spacing = ARROW_SPACING): ArrowScale | null {
  if (!(vmax > 0)) return null
  const scale = (0.9 * spacing) / vmax
  const value = niceBelow(vmax)
  return { scale, reference: { value, length: value * scale } }
}
