import { describe, expect, it } from 'vitest'
import {
  isBlocked,
  maxSpeed,
  sampleVelocity,
  streamlines,
  toSegments,
  traceStreamline,
  velocityField,
  DEFAULT_OPTIONS,
  type VelocityField,
} from '../src/viewer/streamlines.ts'

/** A field from a function of the position (m/s), sampled at the centres of the blocks. */
function field(
  width: number,
  height: number,
  fn: (x: number, y: number) => [number, number],
  k = 1,
  solid: Uint8Array | null = null,
): VelocityField {
  const bw = Math.ceil(width / k)
  const bh = Math.ceil(height / k)
  const u = new Float32Array(bw * bh)
  const v = new Float32Array(bw * bh)
  for (let j = 0; j < bh; j++) {
    for (let i = 0; i < bw; i++) {
      ;[u[j * bw + i], v[j * bw + i]] = fn((i + 0.5) * k, (j + 0.5) * k)
    }
  }
  return velocityField(width, height, k, u, v, solid)
}

const POINTS = (l: Float32Array) =>
  Array.from({ length: l.length / 2 }, (_, i) => [l[2 * i], l[2 * i + 1]])

describe('velocity sampling', () => {
  it('interpolates a linear field exactly, also between larger blocks', () => {
    for (const k of [1, 2, 4]) {
      const f = field(64, 32, (x, y) => [2 * x + y, x - 3 * y], k)
      const out = new Float64Array(2)
      // Inside the outermost block centres, bilinear interpolation reproduces a linear field.
      for (const [x, y] of [
        [10.3, 7.7],
        [31.9, 20.1],
        [5.5, 12.25],
      ]) {
        sampleVelocity(f, x, y, out)
        expect(out[0]).toBeCloseTo(2 * x + y, 4)
        expect(out[1]).toBeCloseTo(x - 3 * y, 4)
      }
    }
  })

  it('reports the domain’s edges and solid cells as blocked', () => {
    const solid = new Uint8Array(10 * 10)
    solid[5 * 10 + 5] = 2
    const f = field(10, 10, () => [1, 0], 1, solid)
    expect(isBlocked(f, 5.5, 5.5)).toBe(true)
    expect(isBlocked(f, 4.9, 5.5)).toBe(false)
    expect(isBlocked(f, -0.1, 2)).toBe(true)
    expect(isBlocked(f, 10, 2)).toBe(true)
    expect(maxSpeed(f)).toBeCloseTo(1)
  })
})

describe('streamlines', () => {
  it('are straight in a uniform flow and run to the domain’s edges', () => {
    const angle = Math.atan2(0.5, 1)
    const f = field(120, 60, () => [1, 0.5])
    const lines = streamlines(f, { separation: 10, maxLength: 400 })
    expect(lines.length).toBeGreaterThan(4)
    const [dx, dy] = [Math.cos(angle), Math.sin(angle)]
    for (const l of lines) {
      const pts = POINTS(l)
      for (const [x, y] of pts) {
        // Distance from the line through the first point with the flow's direction.
        const d = Math.abs((x - pts[0][0]) * dy - (y - pts[0][1]) * dx)
        expect(d).toBeLessThan(1e-4)
      }
      // Downstream: x increases along the line, from the upstream end to the downstream one.
      for (let i = 1; i < pts.length; i++) expect(pts[i][0]).toBeGreaterThan(pts[i - 1][0])
    }
    // A line that was not cut short by its neighbours reaches an edge (within a step).
    const reach = lines.filter((l) => {
      const [x, y] = [l[l.length - 2], l[l.length - 1]]
      return x > 119 || y > 59
    })
    expect(reach.length).toBeGreaterThan(0)
  })

  it('keep their distance: no two points of different lines are closer than the test distance', () => {
    const f = field(120, 60, () => [1, 0.5])
    const o = { separation: 10, maxLength: 400 }
    const lines = streamlines(f, o)
    const d = o.separation * DEFAULT_OPTIONS.test
    let min = Infinity
    for (let a = 0; a < lines.length; a++) {
      for (let b = a + 1; b < lines.length; b++) {
        for (const [xa, ya] of POINTS(lines[a])) {
          for (const [xb, yb] of POINTS(lines[b])) min = Math.min(min, Math.hypot(xa - xb, ya - yb))
        }
      }
    }
    // Seeds are at least `separation` apart, and a line stops at `test × separation` of another.
    expect(min).toBeGreaterThanOrEqual(d * 0.99)
  })

  it('close into circles in a solid-body rotation, with the RK4 error of a few 1e-5 cells', () => {
    const [cx, cy, omega, r] = [50, 50, 0.3, 20]
    const f = field(100, 100, (x, y) => [-omega * (y - cy), omega * (x - cx)])
    // No separation test; each direction goes round once (2πr = 125.66 cells).
    const o = {
      ...DEFAULT_OPTIONS,
      separation: 0,
      test: 0,
      maxLength: 2 * 2 * Math.PI * r,
      step: 0.5,
    }
    const line = traceStreamline(f, cx + r, cy, o, 0)!
    const pts = POINTS(line)
    expect(pts.length).toBeGreaterThan(400)
    let worst = 0
    for (const [x, y] of pts) worst = Math.max(worst, Math.abs(Math.hypot(x - cx, y - cy) - r))
    expect(worst).toBeLessThan(1e-4)
    // Both ends have gone once round: they are a step or two from the seed.
    for (const [x, y] of [pts[0], pts[pts.length - 1]]) {
      expect(Math.hypot(x - (cx + r), y - cy)).toBeLessThan(o.step)
    }
    // Anticlockwise (omega > 0): downstream of the seed y increases.
    const seedIndex = Math.round((pts.length - 1) / 2)
    expect(pts[seedIndex + 1][1]).toBeGreaterThan(cy)
  })

  it('stop after one turn of a closed loop when the separation test is on', () => {
    const [cx, cy, omega, r] = [50, 50, 0.3, 20]
    const f = field(100, 100, (x, y) => [-omega * (y - cy), omega * (x - cx)])
    const line = traceStreamline(
      f,
      cx + r,
      cy,
      { ...DEFAULT_OPTIONS, separation: 8, maxLength: 1000 },
      0,
      undefined,
    )!
    // Without an occupancy grid nothing stops it but the length: 1000 cells ≈ 8 turns.
    expect(line.length / 2).toBeGreaterThan(1500)
    const lines = streamlines(f, { separation: 8, maxLength: 1000 })
    // With it, a line stops where it meets itself: about one turn (125.7 cells = 252 steps).
    for (const l of lines) {
      const radius = Math.hypot(l[0] - cx, l[1] - cy)
      const length = (l.length / 2 - 1) * DEFAULT_OPTIONS.step
      expect(length).toBeLessThan(2 * Math.PI * radius + 12)
    }
    expect(lines.length).toBeGreaterThan(3)
  })

  it('stop at solid cells', () => {
    const solid = new Uint8Array(100 * 40)
    for (let y = 15; y < 25; y++) for (let x = 50; x < 60; x++) solid[y * 100 + x] = 1
    const f = field(100, 40, () => [1, 0], 1, solid)
    const lines = streamlines(f, { separation: 5, maxLength: 300 })
    expect(lines.length).toBeGreaterThan(5)
    for (const l of lines) {
      for (const [x, y] of POINTS(l)) expect(isBlocked(f, x, y)).toBe(false)
    }
    // One that runs straight at the block ends within a step of its face.
    const hit = lines.find((l) => {
      const [x, y] = [l[l.length - 2], l[l.length - 1]]
      return y > 15 && y < 25 && x < 50
    })!
    expect(hit).toBeDefined()
    expect(50 - hit[hit.length - 2]).toBeLessThan(DEFAULT_OPTIONS.step + 1e-3)
  })

  it('stop at stagnation points', () => {
    // A saddle at (50, 50): |u| = distance from the centre.
    const f = field(100, 100, (x, y) => [x - 50, -(y - 50)])
    const floor = DEFAULT_OPTIONS.stagnation * maxSpeed(f)
    const line = traceStreamline(f, 53, 50, { ...DEFAULT_OPTIONS, separation: 0, test: 0 }, floor)!
    for (const [x, y] of POINTS(line))
      expect(Math.hypot(x - 50, y - 50)).toBeGreaterThan(floor * 0.9)
    // Upstream the line ends near the stagnation point but not at it.
    expect(Math.hypot(line[0] - 50, line[1] - 50)).toBeLessThan(floor + 1)
  })

  it('are reproducible, empty without flow, and exported as segments', () => {
    const f = field(60, 30, (x, y) => [1 + 0.01 * y, 0.2 * Math.sin(x / 10)])
    const a = streamlines(f, { separation: 6 })
    const b = streamlines(f, { separation: 6 })
    expect(a.map((l) => Array.from(l))).toEqual(b.map((l) => Array.from(l)))
    expect(streamlines(field(10, 10, () => [0, 0]))).toEqual([])
    const segments = toSegments(a)
    const expected = a.reduce((n, l) => n + l.length / 2 - 1, 0)
    expect(segments.length).toBe(expected * 4)
    // Consecutive segments of a line share their joint.
    const l = a[0]
    expect([...segments.subarray(0, 4)]).toEqual([l[0], l[1], l[2], l[3]])
    expect([...segments.subarray(4, 8)]).toEqual([l[2], l[3], l[4], l[5]])
  })
})
