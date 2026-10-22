// Streamlines of one frame (design proposal §5): RK4 on the frame's velocity with bilinear
// interpolation, seeded on a jittered grid with a minimum separation between lines (a simplified
// Jobard–Lefer) and stopped at non-fluid cells, domain edges, stagnation, a maximum length or
// when a line gets too close to another. Pure functions on typed arrays: they run in the decode
// worker (`protocol.ts`) and in tests, on analytic fields.
//
// Coordinates are *cells* with the origin at the domain's bottom-left corner, so the centre of
// cell (i, j) is (i + 0.5, j + 0.5), as in the shaders.

/** A velocity field: one value per block of `k × k` cells (row-major from the bottom-left). */
export interface VelocityField {
  /** The domain in cells. */
  width: number
  height: number
  /** Cells per block side (1: a value per cell). */
  k: number
  /** The blocks' extent: `ceil(width / k)` × `ceil(height / k)`. */
  bw: number
  bh: number
  /** Velocity of each block (m/s; zero in blocks without fluid). */
  u: Float32Array
  v: Float32Array
  /** Non-zero where a cell is not fluid (full resolution), or null: all fluid. */
  solid: Uint8Array | null
}

export interface StreamlineOptions {
  /** Distance between seeds and lines, in cells. 0 disables the separation test. */
  separation: number
  /** Arc length of one RK4 step, in cells. */
  step: number
  /** Longest line (cells, both directions together). */
  maxLength: number
  /** Stop where |u| falls below this fraction of the field's largest speed. */
  stagnation: number
  /** A line stops when it comes closer than this fraction of `separation` to another one. */
  test: number
  /** Seed for the jitter (the same seed gives the same seeds, so lines change smoothly in time). */
  seed: number
}

export const DEFAULT_OPTIONS: StreamlineOptions = {
  separation: 8,
  step: 0.5,
  maxLength: 1000,
  stagnation: 0.01,
  test: 0.5,
  seed: 1,
}

/** Builds a field from per-block arrays; the sizes follow from the domain and the block factor. */
export function velocityField(
  width: number,
  height: number,
  k: number,
  u: Float32Array,
  v: Float32Array,
  solid: Uint8Array | null = null,
): VelocityField {
  const bw = Math.ceil(width / k)
  const bh = Math.ceil(height / k)
  if (u.length !== bw * bh || v.length !== bw * bh) throw new Error('velocity size mismatch')
  return { width, height, k, bw, bh, u, v, solid }
}

/** Whether a point is outside the domain or in a non-fluid cell. */
export function isBlocked(f: VelocityField, x: number, y: number): boolean {
  if (!(x >= 0 && y >= 0 && x < f.width && y < f.height)) return true
  return f.solid !== null && f.solid[Math.floor(y) * f.width + Math.floor(x)] !== 0
}

/**
 * Bilinear interpolation between the blocks' centres (clamped at the domain's edges), written to
 * `out`. A block's value is the mean over its fluid cells; blocks without fluid are zero, which
 * pulls the interpolated speed towards zero next to a wall.
 */
export function sampleVelocity(f: VelocityField, x: number, y: number, out: Float64Array): void {
  const gx = Math.min(Math.max(x / f.k - 0.5, 0), f.bw - 1)
  const gy = Math.min(Math.max(y / f.k - 0.5, 0), f.bh - 1)
  const i0 = Math.min(Math.floor(gx), Math.max(f.bw - 2, 0))
  const j0 = Math.min(Math.floor(gy), Math.max(f.bh - 2, 0))
  const i1 = Math.min(i0 + 1, f.bw - 1)
  const j1 = Math.min(j0 + 1, f.bh - 1)
  const tx = gx - i0
  const ty = gy - j0
  const a = j0 * f.bw + i0
  const b = j0 * f.bw + i1
  const c = j1 * f.bw + i0
  const d = j1 * f.bw + i1
  const w00 = (1 - tx) * (1 - ty)
  const w10 = tx * (1 - ty)
  const w01 = (1 - tx) * ty
  const w11 = tx * ty
  out[0] = w00 * f.u[a] + w10 * f.u[b] + w01 * f.u[c] + w11 * f.u[d]
  out[1] = w00 * f.v[a] + w10 * f.v[b] + w01 * f.v[c] + w11 * f.v[d]
}

/** The largest speed over the blocks. */
export function maxSpeed(f: VelocityField): number {
  let m = 0
  for (let i = 0; i < f.u.length; i++) m = Math.max(m, Math.hypot(f.u[i], f.v[i]))
  return m
}

const scratch = new Float64Array(2)

/** Unit direction of the flow at a point, signed; false where it vanishes (|u| ≤ `floor`). */
function direction(
  f: VelocityField,
  x: number,
  y: number,
  sign: number,
  floor: number,
  out: Float64Array,
): boolean {
  sampleVelocity(f, x, y, scratch)
  const s = Math.hypot(scratch[0], scratch[1])
  if (!(s > floor)) return false
  out[0] = (sign * scratch[0]) / s
  out[1] = (sign * scratch[1]) / s
  return true
}

/**
 * One RK4 step of arc length `h` along the unit direction field (so the same step works in fast
 * and slow flow); false if the flow vanishes at one of the four stages. The new point is in `p`.
 */
export function rk4Step(
  f: VelocityField,
  p: Float64Array,
  h: number,
  sign: number,
  floor: number,
): boolean {
  const k1 = new Float64Array(2)
  const k2 = new Float64Array(2)
  const k3 = new Float64Array(2)
  const k4 = new Float64Array(2)
  const [x, y] = p
  if (!direction(f, x, y, sign, floor, k1)) return false
  if (!direction(f, x + 0.5 * h * k1[0], y + 0.5 * h * k1[1], sign, floor, k2)) return false
  if (!direction(f, x + 0.5 * h * k2[0], y + 0.5 * h * k2[1], sign, floor, k3)) return false
  if (!direction(f, x + h * k3[0], y + h * k3[1], sign, floor, k4)) return false
  p[0] = x + (h / 6) * (k1[0] + 2 * k2[0] + 2 * k3[0] + k4[0])
  p[1] = y + (h / 6) * (k1[1] + 2 * k2[1] + 2 * k3[1] + k4[1])
  return true
}

/**
 * Points already drawn, in a uniform grid of buckets of `size` cells, for the distance tests.
 * Each point remembers its line and its position along it, so a line does not stop at its own
 * recent points.
 */
export class Occupancy {
  private readonly buckets = new Map<number, number[]>()
  readonly x: number[] = []
  readonly y: number[] = []
  readonly line: number[] = []
  readonly seq: number[] = []
  private readonly nx: number

  private readonly size: number

  constructor(width: number, size: number) {
    this.size = size
    this.nx = Math.ceil(width / size) + 1
  }

  private key(x: number, y: number): number {
    return Math.floor(y / this.size) * this.nx + Math.floor(x / this.size)
  }

  add(x: number, y: number, line: number, seq: number): void {
    const id = this.x.length
    this.x.push(x)
    this.y.push(y)
    this.line.push(line)
    this.seq.push(seq)
    const key = this.key(x, y)
    const bucket = this.buckets.get(key)
    if (bucket) bucket.push(id)
    else this.buckets.set(key, [id])
  }

  /** Whether a point of another line (or an old point of `line`) is closer than `d`. */
  near(x: number, y: number, d: number, line: number, oldest: number): boolean {
    const r = Math.ceil(d / this.size)
    const cx = Math.floor(x / this.size)
    const cy = Math.floor(y / this.size)
    const d2 = d * d
    for (let j = cy - r; j <= cy + r; j++) {
      for (let i = cx - r; i <= cx + r; i++) {
        const bucket = this.buckets.get(j * this.nx + i)
        if (!bucket) continue
        for (const id of bucket) {
          if (this.line[id] === line && this.seq[id] > oldest) continue
          const dx = this.x[id] - x
          const dy = this.y[id] - y
          if (dx * dx + dy * dy < d2) return true
        }
      }
    }
    return false
  }
}

/** A line: points as x, y pairs from its upstream end to its downstream end. */
export type Polyline = Float32Array

/** Mulberry32: a small seeded generator (deterministic, so jitter is reproducible). */
function random(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/**
 * Traces the line through a seed in both directions. `occupancy` (optional) makes it stop near
 * other lines; its own points are added when it is done. Returns null if the seed is blocked or
 * the line is shorter than a few steps.
 */
export function traceStreamline(
  f: VelocityField,
  x0: number,
  y0: number,
  options: StreamlineOptions,
  floor: number,
  occupancy?: Occupancy,
  lineId = 0,
): Polyline | null {
  if (isBlocked(f, x0, y0)) return null
  const { step: h } = options
  const test = options.separation * options.test
  // Own points younger than this (in steps) are the line's neighbours, not other lines.
  const skip = test > 0 ? Math.ceil((2 * test) / h) + 2 : 0
  const half = Math.floor(options.maxLength / h / 2)
  const trace = (sign: number, other: number[]): number[] => {
    const pts: number[] = []
    const p = new Float64Array([x0, y0])
    for (let n = 1; n <= half; n++) {
      if (!rk4Step(f, p, h, sign, floor) || isBlocked(f, p[0], p[1])) break
      if (occupancy && test > 0 && occupancy.near(p[0], p[1], test, lineId, -1)) break
      // Stop at a loop that comes back onto the line's own earlier points (closed streamlines).
      if (occupancy && test > 0 && n > skip && nearOwn(pts, p[0], p[1], test, skip)) break
      // The upstream half must not retrace the downstream one round a closed loop.
      if (occupancy && test > 0 && nearOwn(other, p[0], p[1], test, skip, true)) break
      pts.push(p[0], p[1])
    }
    return pts
  }
  const down = trace(1, [])
  const up = trace(-1, down)
  const total = down.length / 2 + up.length / 2
  if (total < 3) return null
  const out = new Float32Array((total + 1) * 2)
  let o = 0
  for (let i = up.length - 2; i >= 0; i -= 2) {
    out[o++] = up[i]
    out[o++] = up[i + 1]
  }
  out[o++] = x0
  out[o++] = y0
  for (let i = 0; i < down.length; i += 2) {
    out[o++] = down[i]
    out[o++] = down[i + 1]
  }
  if (occupancy) {
    for (let i = 0; i < out.length; i += 2) occupancy.add(out[i], out[i + 1], lineId, i / 2)
  }
  return out
}

/**
 * Whether a point is within `d` of a half-line's points, leaving out the `skip` nearest to where
 * the point comes from (the line's own recent points, or with `fromStart` the first ones, beside
 * the seed).
 */
function nearOwn(
  pts: number[],
  x: number,
  y: number,
  d: number,
  skip: number,
  fromStart = false,
): boolean {
  const d2 = d * d
  const from = fromStart ? 2 * skip : 0
  const to = fromStart ? pts.length : pts.length - 2 * skip
  for (let i = from; i < to; i += 2) {
    const dx = pts[i] - x
    const dy = pts[i + 1] - y
    if (dx * dx + dy * dy < d2) return true
  }
  return false
}

/**
 * The streamlines of a field: candidate seeds on a jittered grid of `separation` cells (in
 * random order), each traced unless it is closer than `separation` to a line already drawn.
 */
export function streamlines(
  f: VelocityField,
  partial: Partial<StreamlineOptions> = {},
): Polyline[] {
  const options = { ...DEFAULT_OPTIONS, ...partial }
  const speed = maxSpeed(f)
  if (!(speed > 0)) return []
  const floor = options.stagnation * speed
  const d = Math.max(options.separation, 1)
  const occupancy = new Occupancy(f.width, d)
  const rand = random(options.seed)
  const seeds: [number, number][] = []
  for (let y = 0; y < f.height; y += d) {
    for (let x = 0; x < f.width; x += d) {
      seeds.push([x + rand() * Math.min(d, f.width - x), y + rand() * Math.min(d, f.height - y)])
    }
  }
  // Shuffle (Fisher–Yates): lines are not all seeded from the bottom-left corner first.
  for (let i = seeds.length - 1; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1))
    ;[seeds[i], seeds[j]] = [seeds[j], seeds[i]]
  }
  const lines: Polyline[] = []
  for (const [x, y] of seeds) {
    if (options.separation > 0 && occupancy.near(x, y, options.separation, -1, -1)) continue
    const line = traceStreamline(f, x, y, options, floor, occupancy, lines.length)
    if (line) lines.push(line)
  }
  return lines
}

/** Polylines as line segments for GL_LINES: x0, y0, x1, y1 for each consecutive pair. */
export function toSegments(lines: readonly Polyline[]): Float32Array {
  let n = 0
  for (const l of lines) n += Math.max(0, l.length / 2 - 1)
  const out = new Float32Array(n * 4)
  let o = 0
  for (const l of lines) {
    for (let i = 0; i + 3 < l.length; i += 2) {
      out[o++] = l[i]
      out[o++] = l[i + 1]
      out[o++] = l[i + 2]
      out[o++] = l[i + 3]
    }
  }
  return out
}
