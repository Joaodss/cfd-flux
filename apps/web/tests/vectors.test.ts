import { describe, expect, it } from 'vitest'
import { arrowScale, niceBelow } from '../src/viewer/vectors.ts'

describe('arrow scale', () => {
  it('rounds the reference down to 1, 2 or 5 × 10ⁿ', () => {
    expect(niceBelow(0.0734)).toBeCloseTo(0.05)
    expect(niceBelow(1)).toBe(1)
    expect(niceBelow(1.99)).toBe(1)
    expect(niceBelow(2)).toBe(2)
    expect(niceBelow(4.9)).toBe(2)
    expect(niceBelow(730)).toBe(500)
    expect(niceBelow(0)).toBe(0)
    expect(niceBelow(NaN)).toBe(0)
  })

  it('maps the fastest speed to 90 % of the spacing and the reference to its share', () => {
    const a = arrowScale(1.5, 24)!
    expect(a.scale * 1.5).toBeCloseTo(21.6)
    expect(a.reference.value).toBe(1)
    expect(a.reference.length).toBeCloseTo(21.6 / 1.5)
    expect(arrowScale(0)).toBeNull()
  })
})
