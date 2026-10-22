import { describe, expect, it } from 'vitest'
import type { DecodedFrame } from '../src/frames/lffr.ts'
import { colormap, colourAt, COLORMAPS, cssGradient, lut } from '../src/viewer/colormaps.ts'
import { boxFactor, fluidFraction } from '../src/viewer/mask.ts'
import { quantitiesOf } from '../src/viewer/quantities.ts'
import { accumulate, rangeFor, seedFromHello, ticks } from '../src/viewer/ranges.ts'
import { frameStats } from '../src/viewer/stats.ts'

/** f32 → f16 bits for test data (round to nearest, normal numbers only). */
function f16(v: number): number {
  const view = new DataView(new ArrayBuffer(2)) as DataView & {
    setFloat16: (o: number, v: number, le?: boolean) => void
  }
  view.setFloat16(0, v, true)
  return view.getUint16(0, true)
}

describe('colour maps', () => {
  it('come from Rust, with the CLI images’ colours', () => {
    expect(COLORMAPS.map((m) => m.name)).toEqual([
      'viridis',
      'inferno',
      'magma',
      'greys',
      'coolwarm',
      'blueRed',
    ])
    // The values of cfd-io's test `colormaps_hit_their_end_samples`.
    const viridis = colormap('viridis')
    expect(colourAt(viridis, 0)).toEqual([68, 1, 84])
    expect(colourAt(viridis, 1)).toEqual([253, 231, 37])
    expect(colourAt(viridis, 1 / 16)).toEqual([70, 21, 102])
    expect(colourAt(colormap('coolwarm'), 0.5)).toEqual([221, 221, 221])
    expect(colourAt(colormap('inferno'), 2)).toEqual([252, 255, 164])
    expect(colourAt(colormap('inferno'), NaN)).toEqual([0, 0, 4])
    expect(colormap('coolwarm').diverging).toBe(true)
    expect(colormap('nope').name).toBe('viridis')
  })

  it('make a 256-entry lookup table and a CSS gradient', () => {
    const t = lut(colormap('greys'))
    expect(t.length).toBe(1024)
    expect([...t.subarray(0, 4)]).toEqual([0, 0, 0, 255])
    expect([...t.subarray(1020)]).toEqual([255, 255, 255, 255])
    expect(cssGradient(colormap('greys'))).toMatch(/^linear-gradient\(to right, rgb\(0 0 0\) 0%,/)
  })
})

describe('fluid fraction', () => {
  it('uses the server’s box factor and counts partial blocks', () => {
    expect(boxFactor(442, 84, 1024)).toBe(1)
    expect(boxFactor(4096, 4096, 1024)).toBe(4)
    expect(boxFactor(1025, 10, 1024)).toBe(2)
    // 5 × 3 cells, k = 2 → 3 × 2 blocks; one solid cell (1) at (1, 0).
    const cells = Uint8Array.from([0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
    const f = fluidFraction(cells, 5, 3, 2)
    expect([...f]).toEqual([0.75, 1, 1, 1, 1, 1])
    expect([...fluidFraction(cells, 5, 3, 1)].filter((v) => v === 0).length).toBe(1)
  })
})

describe('quantities', () => {
  it('derive |u|, u_x, u_y from a vector and keep scalars', () => {
    const q = quantitiesOf([
      { name: 'velocity', components: 2, dtype: 'f16' },
      { name: 'vorticity', components: 1, dtype: 'f16' },
    ])
    expect(q.map((x) => [x.key, x.component, x.symmetric])).toEqual([
      ['velocity', 'magnitude', false],
      ['velocity.x', 'x', true],
      ['velocity.y', 'y', true],
      ['vorticity', 'scalar', true],
    ])
    expect(q[3].unit).toBe('1/s')
  })
})

describe('frame statistics', () => {
  const frame = (velocity: number[], pressure: number[]): DecodedFrame => ({
    header: {
      jobId: 'j',
      frameIndex: 0,
      step: 0,
      simTime: 0,
      width: 2,
      height: 2,
      codec: 'none',
      fields: [
        { name: 'velocity', components: 2, dtype: 'f16' },
        { name: 'pressure', components: 1, dtype: 'f32' },
      ],
    },
    fields: [
      { name: 'velocity', components: 2, dtype: 'f16', data: Uint16Array.from(velocity, f16) },
      { name: 'pressure', components: 1, dtype: 'f32', data: Float32Array.from(pressure) },
    ],
  })

  it('cover fluid blocks only, corrected by their fluid fraction', () => {
    // Block 3 is solid (fraction 0); block 1 is half fluid, so its averaged values double.
    const s = frameStats(
      frame([3, 4, 0.5, 0, -1, 0, 99, 99], [10, -2, 4, 1000]),
      Float32Array.from([1, 0.5, 1, 0]),
    )
    expect(s.velocity).toMatchObject({ min: 1, max: 5, nonFinite: 0 })
    expect(s['velocity.x']).toMatchObject({ min: -1, max: 3 })
    expect(s['velocity.y']).toMatchObject({ min: 0, max: 4 })
    expect(s.pressure).toMatchObject({ min: -4, max: 10 })
    expect(s.pressure.absP99).toBeGreaterThanOrEqual(10)
    expect(s.pressure.absP99).toBeLessThanOrEqual(10.01)
  })

  it('count values beyond the f16 range', () => {
    const s = frameStats(frame([Infinity, 0, 1, 0, 2, 0, 3, 0], [0, 0, 0, 0]), null)
    expect(s.velocity).toMatchObject({ min: 1, max: 3, nonFinite: 1 })
  })

  it('give the 99th percentile of |value|', () => {
    const n = 1000
    const header = { ...frame([], []).header, width: n, height: 1 }
    header.fields = [{ name: 'vorticity', components: 1, dtype: 'f32' }]
    // 990 values in [−1, 1], ten spikes of 50: the percentile ignores the spikes.
    const values = Float32Array.from({ length: n }, (_, i) => (i < 10 ? 50 : Math.sin(i)))
    const s = frameStats(
      { header, fields: [{ name: 'vorticity', components: 1, dtype: 'f32', data: values }] },
      null,
    )
    expect(s.vorticity.max).toBe(50)
    expect(s.vorticity.absP99).toBeLessThan(1.05)
    expect(s.vorticity.absP99).toBeGreaterThan(0.95)
  })
})

describe('ranges', () => {
  const frame = { min: -2, max: 3, absP99: 2.5, nonFinite: 0 }
  it('follow the mode', () => {
    const seen = { min: -4, max: 5, absP99: 4 }
    expect(rangeFor('all', frame, seen, undefined)).toEqual([-4, 5])
    expect(rangeFor('frame', frame, seen, undefined)).toEqual([-2, 3])
    expect(rangeFor('symmetric', frame, seen, undefined)).toEqual([-4, 4])
    expect(rangeFor('manual', frame, seen, [0, 1])).toEqual([0, 1])
    expect(rangeFor('all', frame, undefined, undefined)).toEqual([-2, 3])
  })

  it('never collapse to an empty range', () => {
    const flat = { min: 2, max: 2, absP99: 2, nonFinite: 0 }
    const [lo, hi] = rangeFor('frame', flat, undefined, undefined)
    expect(lo).toBeLessThan(2)
    expect(hi).toBeGreaterThan(2)
    expect(rangeFor('frame', { ...flat, min: 0, max: 0 }, undefined, undefined)).toEqual([-1, 1])
  })

  it('accumulate frames and start from the stream’s hello ranges', () => {
    const seen = seedFromHello({ velocity: [0, 1.5], pressure: [-3, 2] })
    expect(seen['velocity.x']).toEqual({ min: -1.5, max: 1.5, absP99: 0 })
    // No percentile from hello: a symmetric range uses the frame's until frames are seen.
    expect(rangeFor('symmetric', frame, seen.pressure, undefined)).toEqual([-2.5, 2.5])
    const next = accumulate(seen, { pressure: frame })
    expect(next.pressure).toEqual({ min: -3, max: 3, absP99: 2.5 })
    expect(seen.pressure.max).toBe(2) // unchanged
    expect(ticks([0, 1])).toEqual([0, 0.25, 0.5, 0.75, 1])
  })
})
