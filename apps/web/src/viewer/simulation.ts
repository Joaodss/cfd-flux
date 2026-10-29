// The CPU side of the tracer particles and the dye (design proposal §5): where particles are
// released, and how far the fields advance per animation frame. The advection itself runs on the
// GPU (`render/simulation.ts`).
import { CellType } from '../editor/layers.ts'

/** Cells where particles may be released, as x, y pairs (cell centres). */
export const MAX_SPAWNS = 16384

/**
 * The fluid cells next to an inlet cell (the inlet cells themselves carry no velocity in the
 * frames), as cell centres x, y. At most `max`: a regular subsample, so the choice is stable.
 */
export function inletSpawns(
  cellType: Uint8Array,
  width: number,
  height: number,
  max = MAX_SPAWNS,
): Float32Array {
  const seen = new Set<number>()
  const points: number[] = []
  const add = (x: number, y: number) => {
    if (x < 0 || y < 0 || x >= width || y >= height) return
    const id = y * width + x
    if (cellType[id] !== CellType.Fluid || seen.has(id)) return
    seen.add(id)
    points.push(x + 0.5, y + 0.5)
  }
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (cellType[y * width + x] !== CellType.Inlet) continue
      add(x - 1, y)
      add(x + 1, y)
      add(x, y - 1)
      add(x, y + 1)
    }
  }
  const n = points.length / 2
  if (n <= max) return Float32Array.from(points)
  const out = new Float32Array(max * 2)
  for (let i = 0; i < max; i++) {
    const j = Math.floor((i * n) / max)
    out[2 * i] = points[2 * j]
    out[2 * i + 1] = points[2 * j + 1]
  }
  return out
}

/**
 * Simulated seconds that pass during `wall` seconds of animation: frames per second × the
 * simulated time between frames (so particle speed is physical: at 30 frames/s and 0.02 s per
 * frame the flow runs at 0.6 × real time).
 */
export function simulatedStep(
  wall: number,
  framesPerSecond: number,
  frameInterval: number,
): number {
  return Math.min(Math.max(wall, 0), 0.05) * framesPerSecond * frameInterval
}

/** How a step of `dt` simulated seconds is split so that nothing moves more than `limit` cells. */
export function substeps(
  maxSpeed: number,
  dt: number,
  cellSize: number,
  limit = 0.75,
  cap = 8,
): { count: number; dt: number } {
  const cells = (maxSpeed * dt) / cellSize
  const count = cells > 0 ? Math.min(cap, Math.max(1, Math.ceil(cells / limit))) : 1
  return { count, dt: dt / count }
}

/** Where particles are released. */
export type ReleaseMode = 'inlets' | 'line' | 'domain'

/** Particles the GPU state holds (a 256 × 128 texture): the most that can be shown. */
export const MAX_PARTICLES = 32768

/**
 * How many particles are shown: tracers per fluid cell, so a small grid is not buried under them,
 * capped per density so the choices still differ on a large grid (where every one would reach
 * `MAX_PARTICLES`).
 */
export const DENSITIES = {
  few: { label: 'Few', perCell: 0.05, max: 4096 },
  normal: { label: 'Normal', perCell: 0.15, max: 12288 },
  many: { label: 'Many', perCell: 0.5, max: MAX_PARTICLES },
} as const
export type Density = keyof typeof DENSITIES

/** The particles shown on a grid of `fluidCells` fluid cells at a density (at least 64). */
export function particleCount(fluidCells: number, density: Density): number {
  const d = DENSITIES[density]
  return Math.min(d.max, Math.max(64, Math.round(fluidCells * d.perCell)))
}

/** Particle diameters offered, in CSS pixels on screen and in pixels of an exported picture. */
export const PARTICLE_SIZES = [1, 2, 3, 4] as const
export type ParticleSize = (typeof PARTICLE_SIZES)[number]

/**
 * A particle's diameter in device pixels. On screen it is CSS pixels, so × the device-pixel
 * ratio; an export at n pixels per cell draws with a ratio of n, but its pixels are the
 * picture's own: the size stays as chosen instead of growing to n × 2 px (2 × 2 cells at 4 px
 * per cell).
 */
export function particlePixels(size: ParticleSize, dpr: number, exported: boolean): number {
  return exported ? size : size * dpr
}

/** Dye colours (the three channels): smoke white, orange and cyan. */
export const DYE_COLOURS: readonly [number, number, number][] = [
  [1, 1, 1],
  [1, 0.55, 0.12],
  [0.2, 0.85, 1],
]
