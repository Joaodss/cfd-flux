import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { RunMeta } from '../src/api/jobs.ts'
import { coefficients } from '../src/viewer/plots.ts'
import { ALL_EXAMPLES } from '../src/scene/examples.ts'
import {
  benchmarkOfScene,
  BENCHMARKS,
  check,
  isBenchmark,
  toleranceAt,
} from '../src/viewer/references.ts'
import { SeriesBuffer } from '../src/viewer/series.ts'
import {
  defaultWindow,
  forceStatistics,
  heatElements,
  nusselt,
  oscillationFrequency,
  upwardCrossings,
  wallNusselt,
} from '../src/viewer/statistics.ts'

const meta = (rho0: number, velocity: number, length: number): RunMeta => ({
  units: { dx: 0.005, dt: 1e-4, rho0 },
  dimensionless: { reference: { length, velocity, reynolds: 100, temperatureDifference: 1 } },
  run: { outputEverySteps: 60, sampleEverySteps: 6, endTime: 12 },
})

describe('statistics', () => {
  // C_D = 3 + 0.1 sin(2·2πft), C_L = 0.2 + sin(2πft), f = 3 Hz, sampled at 1 kHz for 4 s.
  const f = 3
  const time = Array.from({ length: 4001 }, (_, i) => i / 1000)
  const cd = time.map((t) => 3 + 0.1 * Math.sin(4 * Math.PI * f * t))
  const cl = time.map((t) => 0.2 + Math.sin(2 * Math.PI * f * t))

  it('finds the frequency from the upward crossings through the mean', () => {
    const c = upwardCrossings(time, cl)
    // t = 1/3 … 11/3 s (the start, at the mean, has no sample before it)
    expect(c.length).toBe(11)
    expect(c[0]).toBeCloseTo(1 / 3, 6)
    expect(oscillationFrequency(time, cl)).toBeCloseTo(f, 6)
    expect(oscillationFrequency([0, 1], [1, 2])).toBeNull()
  })

  it('computes the window statistics and St = f L / U', () => {
    const s = forceStatistics(time, cd, cl, { from: 1, to: 4 }, meta(1, 1, 0.1))!
    expect(s.samples).toBe(3001)
    expect(s.cdMean).toBeCloseTo(3, 4)
    expect(s.cdMax).toBeCloseTo(3.1, 6)
    expect(s.clMean).toBeCloseTo(0.2, 4)
    expect(s.clAmplitude).toBeCloseTo(1, 5)
    expect(s.clMax).toBeCloseTo(1.2, 5)
    expect(s.frequency).toBeCloseTo(f, 6)
    expect(s.strouhal).toBeCloseTo(0.3, 6)
    expect(s.periods).toBe(8)
    // Gaps are left out; a window with fewer than two samples has no statistics.
    const gappy = [...cd]
    gappy[2000] = null as unknown as number
    expect(forceStatistics(time, gappy, cl, { from: 1, to: 4 }, meta(1, 1, 0.1))!.samples).toBe(
      3000,
    )
    expect(forceStatistics(time, cd, cl, { from: 5, to: 6 }, meta(1, 1, 0.1))).toBeNull()
  })

  it('defaults to the second half of the time so far', () => {
    expect(defaultWindow([2, 3, 6])).toEqual({ from: 4, to: 6 })
    expect(defaultWindow([])).toBeNull()
  })
})

describe('contract with cfd-verify', () => {
  // A real force history (8.5–10 s of the cylinder example) and the frequency
  // `cfd_verify::harness::oscillation_frequency` finds in it (crates/cfd-verify/tests/web_contract.rs).
  const dir = new URL('./fixtures/series/', import.meta.url)
  const csv = readFileSync(new URL('cylinder-forces.csv', dir), 'utf8')
  const expected = JSON.parse(readFileSync(new URL('cylinder-statistics.json', dir), 'utf8')) as {
    reference: { density: number; velocity: number; length: number }
    samples: number
    frequency: number
    strouhal: number
  }

  it('finds the same Strouhal number as Rust, to 1e-12', () => {
    const b = new SeriesBuffer()
    b.setFile('forces', csv)
    const forces = b.table('forces')!
    const { density, velocity, length } = expected.reference
    const m = meta(density, velocity, length)
    const c = coefficients(forces, 'Cylinder', m)!
    const time = forces.data[1]
    const s = forceStatistics(time, c.cd, c.cl, { from: -Infinity, to: Infinity }, m)!
    expect(s.samples).toBe(expected.samples)
    expect(Math.abs(s.strouhal! - expected.strouhal)).toBeLessThan(1e-12)
    expect(Math.abs(s.frequency! - expected.frequency)).toBeLessThan(1e-11)
  })
})

describe('references', () => {
  const st = BENCHMARKS.schaferTurek2D2

  it('come from cfd-verify, and every example names a known benchmark', () => {
    expect(st.quantities.strouhal).toEqual([0.295, 0.305])
    expect(st.checked).toEqual(['strouhal', 'cdMax'])
    for (const ex of ALL_EXAMPLES) {
      if (ex.benchmark !== undefined) expect(isBenchmark(ex.benchmark), ex.id).toBe(true)
    }
    expect(benchmarkOfScene('Cylinder Re=100')).toBe('schaferTurek2D2')
    expect(benchmarkOfScene('My scene')).toBeNull()
  })

  it("use the suite's tolerance at the run's resolution", () => {
    expect(toleranceAt(st, 20)).toEqual({ relative: 0.05, coarser: false })
    expect(toleranceAt(st, 40)).toEqual({ relative: 0.05, coarser: false })
    expect(toleranceAt(st, 80)).toEqual({ relative: 0.02, coarser: false })
    expect(toleranceAt(st, 10)).toEqual({ relative: 0.05, coarser: true })
    // Against the interval's centre: C_D,max 3.23.
    expect(check(st, 'cdMax', 3.37, 0.05)?.pass).toBe(true)
    expect(check(st, 'cdMax', 3.42, 0.05)?.pass).toBe(false)
    expect(check(st, 'clMax', 2, 0.05)?.pass).toBeNull() // information only
    expect(check(st, 'cdMean', 3, 0.05)).toBeNull()
  })
})

describe('Nusselt number', () => {
  const csv = readFileSync(new URL('./fixtures/series/cavity-heat.csv', import.meta.url), 'utf8')
  const expected = JSON.parse(
    readFileSync(new URL('./fixtures/series/cavity-nusselt.json', import.meta.url), 'utf8'),
  ) as {
    reference: { length: number; conductivity: number; deltaT: number }
    elements: {
      element: string
      samples: number
      mean: number
      min: number
      max: number
      spread: number
    }[]
  }

  it('is q L / (k ΔT · length)', () => {
    // Pure conduction through a wall of length 0.1 m: q = k ΔT / L per metre of wall length.
    expect(nusselt(0.0257 * 10, 0.1, 0.1, 0.0257, 10)).toBeCloseTo(1, 12)
    expect(nusselt(-2, 0.5, 0.1, 0.5, 4)).toBeCloseTo(-0.2, 12)
  })

  it('averages the frames of the window, per element', () => {
    const columns = ['step', 'time_s', 'A.q_W_m', 'A.length_m']
    const data = [
      [0, 1, 2, 3],
      [0, 1, 2, 3],
      [1, 2, null, 4],
      [1, 1, 1, 1],
    ]
    const scales = { length: 1, conductivity: 1, deltaT: 1 }
    expect(heatElements(columns)).toEqual(['A'])
    const [a] = wallNusselt(columns, data, { from: 1, to: 3 }, scales)
    // Rows at t = 1 and 3 (the gap at t = 2 is left out): Nu = 2 and 4.
    expect(a).toEqual({ element: 'A', samples: 2, mean: 3, min: 2, max: 4, spread: 2 / 3 })
    expect(wallNusselt(columns, data, { from: 10, to: 11 }, scales)).toEqual([])
  })

  it('finds the same Nusselt numbers as Rust, to 1e-12', () => {
    const b = new SeriesBuffer()
    b.setFile('heat', csv)
    const heat = b.table('heat')!
    const { length, conductivity, deltaT } = expected.reference
    const got = wallNusselt(
      heat.columns,
      heat.data,
      { from: -Infinity, to: Infinity },
      { length, conductivity, deltaT },
    )
    expect(got.map((g) => g.element)).toEqual(expected.elements.map((e) => e.element))
    for (const [i, e] of expected.elements.entries()) {
      expect(got[i].samples).toBe(e.samples)
      for (const key of ['mean', 'min', 'max', 'spread'] as const) {
        expect(Math.abs(got[i][key] - e[key]), `${e.element}.${key}`).toBeLessThan(1e-12)
      }
    }
  })

  it('has a benchmark for the heated cavity examples, checked at the suite’s tolerance', () => {
    const b = BENCHMARKS.deVahlDavis1e5
    expect(b.checked).toEqual(['nuHot'])
    expect(b.quantities.nuHot).toEqual([4.509, 4.509])
    expect(toleranceAt(b, 64)).toEqual({ relative: 0.02, coarser: false })
    expect(toleranceAt(b, 32).coarser).toBe(true)
    expect(benchmarkOfScene('Heated cavity Ra=1e5')).toBe('deVahlDavis1e5')
    expect(benchmarkOfScene('Heated cavity Ra=1e5 (quick)')).toBe('deVahlDavis1e5')
    expect(check(b, 'nuHot', 4.5417, 0.02)?.pass).toBe(true)
    expect(check(b, 'nuHot', 4.7, 0.02)?.pass).toBe(false)
  })
})
