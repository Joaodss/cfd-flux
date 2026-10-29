import { describe, expect, it } from 'vitest'
import { CellType } from '../src/editor/layers.ts'
import {
  inletSpawns,
  MAX_PARTICLES,
  particleCount,
  particlePixels,
  simulatedStep,
  substeps,
} from '../src/viewer/simulation.ts'

describe('particle release', () => {
  it('uses the fluid cells next to inlets, each once, as cell centres', () => {
    // 6 × 3: an inlet column at x = 0 with a wall cell above it.
    const w = 6
    const cells = new Uint8Array(w * 3)
    for (let y = 0; y < 3; y++) cells[y * w] = CellType.Inlet
    cells[2 * w + 1] = CellType.Solid
    const p = inletSpawns(cells, w, 3)
    expect(Array.from(p)).toEqual([1.5, 0.5, 1.5, 1.5])
    expect(inletSpawns(new Uint8Array(w * 3), w, 3).length).toBe(0)
  })

  it('subsamples a long inlet evenly and deterministically', () => {
    const cells = new Uint8Array(4 * 100)
    for (let y = 0; y < 100; y++) cells[y * 4] = CellType.Inlet
    const a = inletSpawns(cells, 4, 100, 10)
    expect(a.length).toBe(20)
    expect(a[0]).toBe(1.5)
    expect(Array.from(a)).toEqual(Array.from(inletSpawns(cells, 4, 100, 10)))
    const ys = Array.from(a).filter((_, i) => i % 2 === 1)
    expect(ys).toEqual([0.5, 10.5, 20.5, 30.5, 40.5, 50.5, 60.5, 70.5, 80.5, 90.5])
  })
})

describe('particle density and size', () => {
  it('scales with the fluid cells, capped per density and at least 64', () => {
    // The cylinder at 4 px per cell: 442 × 96 cells, about 42,000 of them fluid.
    expect([particleCount(42000, 'few'), particleCount(42000, 'normal')]).toEqual([2100, 6300])
    expect(particleCount(42000, 'many')).toBe(21000)
    // The 1023² cavity: every density at its cap, and they still differ.
    const cavity = 1023 * 1023
    expect(['few', 'normal', 'many'].map((d) => particleCount(cavity, d as 'few'))).toEqual([
      4096,
      12288,
      MAX_PARTICLES,
    ])
    expect(particleCount(100, 'normal')).toBe(64)
  })

  it('keeps the chosen size in exports instead of growing with the pixels per cell', () => {
    expect(particlePixels(2, 2, false)).toBe(4) // CSS pixels on a 2× screen
    expect(particlePixels(2, 4, true)).toBe(2) // an export at 4 px per cell
    expect(particlePixels(3, 1, true)).toBe(3)
  })
})

describe('simulated time per animation frame', () => {
  it('is the playback rate times the frame interval, capped at 50 ms of wall time', () => {
    expect(simulatedStep(1 / 60, 30, 0.02)).toBeCloseTo(0.01)
    expect(simulatedStep(2, 30, 0.02)).toBeCloseTo(0.05 * 30 * 0.02)
    expect(simulatedStep(-1, 30, 0.02)).toBe(0)
  })

  it('splits steps so no particle moves more than 0.75 cells', () => {
    expect(substeps(1, 0.01, 0.01)).toEqual({ count: 2, dt: 0.005 }) // 1 cell → 2 passes
    expect(substeps(0.1, 0.01, 0.01)).toEqual({ count: 1, dt: 0.01 })
    expect(substeps(0, 0.01, 0.01).count).toBe(1)
    expect(substeps(1000, 0.01, 0.01).count).toBe(8) // capped
  })
})
